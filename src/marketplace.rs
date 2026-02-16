// src/marketplace.rs
//
// =============================================================================
// UNIFIEDLAB: MARKETPLACE COORDINATOR (v 0.1 )
// =============================================================================
//
// The Global Scheduler.
// Manages the DAG, matches jobs to workers, and handles dynamic expansion.
// **TODO** write a detailed expansion plan

use crate::checkpoint::{CheckpointStore, WorkerInfo};
use crate::core::{CalculationResult, Job, JobConfig, JobStatus};
use crate::eventlog::EventEnvelope;
use crate::transport::Transport;
use crate::workflow::{NodeType, WorkflowEngine};

use anyhow::{anyhow, Result};
use petgraph::graph::NodeIndex;
use petgraph::Direction;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet, VecDeque};
use std::time::{Duration, Instant};
use uuid::Uuid;

// =============================================================================
// 1. WIRE PROTOCOL
// =============================================================================

pub const EV_JOB_SUBMIT: &str = "job.submit";
pub const EV_JOB_COMPLETE: &str = "job.complete";
pub const EV_WORK_GRANT: &str = "work.grant";
pub const MSG_WORK_REQUEST: &str = "work.request";
pub const MSG_JOB_COMPLETE: &str = "job.complete_report";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobSubmit {
    pub jobs: Vec<Job>,
    pub deps: Vec<(Uuid, Uuid)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkGrant {
    pub worker_id: String,
    pub grant_id: String,
    pub jobs: Vec<Job>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkRequest {
    pub worker_id: String,
    pub available_cores: usize,
    pub available_gpus: usize,
    pub max_jobs: usize,
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobCompleteReport {
    pub job_id: Uuid,
    pub status: JobStatus,
    pub result: Option<CalculationResult>,
    pub error: Option<String>,
}

// =============================================================================
// 2. INTERNAL STATE
// =============================================================================

struct NodeState {
    job: Job,
    parents_total: usize,
    parents_done: usize,
    blocked: bool,
    inflight: bool,
    enqueued: bool,
    assigned_to: Option<String>,
}

impl NodeState {
    fn is_state_runnable(&self) -> bool {
        !self.inflight
            && !self.blocked
            && self.parents_done >= self.parents_total
            && self.job.status == JobStatus::Pending
            && !self.enqueued
    }

    fn is_runnable_logic_only(&self) -> bool {
        !self.inflight
            && !self.blocked
            && self.parents_done >= self.parents_total
            && self.job.status == JobStatus::Pending
    }
}

struct WorkerLive {
    _last_seen: Instant,
    available_cores: usize,
    available_gpus: usize,
    inflight_jobs: usize,
    wants_work: bool,
    tags: HashSet<String>,
}

// =============================================================================
// 3. COORDINATOR IMPLEMENTATION
// =============================================================================

pub struct MarketplaceCoordinator {
    transport: Box<dyn Transport>,
    store: CheckpointStore,
    workflow: WorkflowEngine,
    landscape_registry: HashMap<String, Uuid>,
    nodes: HashMap<Uuid, NodeState>,
    ready_queue: VecDeque<Uuid>,
    workers: HashMap<String, WorkerLive>,
    dirty_jobs: HashSet<Uuid>,
    last_ckpt: Instant,
    global_cursor: u64,
}

impl MarketplaceCoordinator {
    pub async fn open(transport: Box<dyn Transport>, store: CheckpointStore) -> Result<Self> {
        let jobs_map = store.restore_jobs()?;
        let cursor = store.get_cursor()?;

        let mut nodes = HashMap::new();
        let mut workflow = WorkflowEngine::new();
        let mut landscape_registry = HashMap::new();

        for (id, job) in jobs_map {
            nodes.insert(
                id,
                NodeState {
                    job: job.clone(),
                    parents_total: 0,
                    parents_done: 0,
                    blocked: false,
                    inflight: false,
                    enqueued: false,
                    assigned_to: None,
                },
            );

            let n_type = job
                .flow_context
                .get("node_type")
                .and_then(|v| serde_json::from_value(v.clone()).ok())
                .unwrap_or(NodeType::Compute);

            if job.status == JobStatus::Completed {
                let fingerprint = Self::fingerprint_job(&job.config);
                landscape_registry.insert(fingerprint, id);
            }

            let _ = workflow.add_smart_node(job, n_type, vec![], 50, true);
        }

        let completed_or_failed: HashSet<Uuid> = nodes
            .values()
            .filter(|n| matches!(n.job.status, JobStatus::Completed | JobStatus::Failed))
            .map(|n| n.job.id)
            .collect();

        for (_cid, node) in &mut nodes {
            node.parents_total = node.job.parent_ids.len();
            node.parents_done = node
                .job
                .parent_ids
                .iter()
                .filter(|pid| completed_or_failed.contains(pid))
                .count();

            if node.job.status == JobStatus::Pending && node.parents_total > node.parents_done {
                node.blocked = true;
                node.job.status = JobStatus::Blocked;
            } else if node.job.status == JobStatus::Running {
                node.inflight = false;
                node.job.status = JobStatus::Pending;
            }
        }

        let mut coord = Self {
            transport,
            store,
            nodes,
            workflow,
            landscape_registry,
            ready_queue: VecDeque::new(),
            workers: HashMap::new(),
            dirty_jobs: HashSet::new(),
            last_ckpt: Instant::now(),
            global_cursor: cursor,
        };

        coord.rebuild_ready_queue();
        coord.transport.seek(cursor).await?;

        Ok(coord)
    }

    fn fingerprint_job(config: &JobConfig) -> String {
        let mut hasher = Sha256::new();
        hasher.update(
            serde_json::to_string(&config)
                .unwrap_or_default()
                .as_bytes(),
        );
        format!("{:x}", hasher.finalize())
    }

    pub async fn tick(&mut self) -> Result<()> {
        let msgs = self.transport.recv_worker_messages().await?;
        for env in msgs {
            self.handle_worker_message(env).await?;
        }
        self.schedule_work().await?;
        self.maybe_checkpoint()?;
        Ok(())
    }

    async fn handle_worker_message(&mut self, env: EventEnvelope) -> Result<()> {
        if env.next_offset > self.global_cursor {
            self.global_cursor = env.next_offset;
        }

        match env.record.kind.as_str() {
            MSG_WORK_REQUEST => {
                if let Ok(req) = serde_json::from_value::<WorkRequest>(env.record.payload) {
                    self.update_worker_live(req);
                }
            }
            MSG_JOB_COMPLETE => {
                if let Ok(rep) = serde_json::from_value::<JobCompleteReport>(env.record.payload) {
                    self.transport
                        .broadcast(EV_JOB_COMPLETE, serde_json::to_value(&rep)?)
                        .await?;
                    self.apply_job_complete(rep).await?;
                }
            }
            EV_JOB_SUBMIT => {
                if let Ok(sub) = serde_json::from_value::<JobSubmit>(env.record.payload) {
                    self.transport
                        .broadcast(EV_JOB_SUBMIT, serde_json::to_value(&sub)?)
                        .await?;
                    self.ingest_submission(sub);
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn update_worker_live(&mut self, req: WorkRequest) {
        let tags: HashSet<String> = req.tags.into_iter().collect();
        let entry = self
            .workers
            .entry(req.worker_id.clone())
            .or_insert_with(|| WorkerLive {
                _last_seen: Instant::now(),
                available_cores: 0,
                available_gpus: 0,
                inflight_jobs: 0,
                wants_work: false,
                tags: HashSet::new(),
            });

        entry._last_seen = Instant::now();
        entry.available_cores = req.available_cores;
        entry.available_gpus = req.available_gpus;
        entry.wants_work = true;
        entry.tags = tags;
    }

    async fn apply_job_complete(&mut self, rep: JobCompleteReport) -> Result<()> {
        let job_id = rep.job_id;

        if let Some(node) = self.nodes.get_mut(&job_id) {
            node.inflight = false;
            node.job.status = rep.status.clone();
            node.job.result = rep.result.clone();
            node.job.error_log = rep.error;
            node.job.updated_at = chrono::Utc::now();
            self.dirty_jobs.insert(job_id);

            if rep.status == JobStatus::Completed {
                let finger = Self::fingerprint_job(&node.job.config);
                self.landscape_registry.insert(finger, job_id);
            }

            if let Some(wid) = &node.job.node_id {
                if let Some(w) = self.workers.get_mut(wid) {
                    w.inflight_jobs = w.inflight_jobs.saturating_sub(1);
                }
            }
        } else {
            return Ok(());
        }

        if rep.status == JobStatus::Completed {
            if let Some(&wf_idx) = self.workflow.id_map.get(&job_id) {
                let node_type = self.workflow.graph[wf_idx].node_type.clone();
                match node_type {
                    NodeType::Switch { .. } => {
                        if let Some(res) = &rep.result {
                            let val = serde_json::to_value(res).unwrap_or(Value::Null);
                            self.workflow.resolve_logic_branch(wf_idx, &val);
                            self.sync_pruning_to_scheduler();
                        }
                    }
                    NodeType::Generator { .. } => {
                        if let Some(res) = &rep.result {
                            if let Some(next_gen) = &res.next_generation {
                                if let Err(e) = self
                                    .expand_generator_defensive(wf_idx, next_gen.clone())
                                    .await
                                {
                                    log::error!("Expansion Failed for {}: {}", job_id, e);
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        let mut unblocked = Vec::new();
        for (cid, cnode) in &mut self.nodes {
            if cnode.job.parent_ids.contains(&job_id) {
                cnode.parents_done += 1;
                if cnode.parents_done >= cnode.parents_total {
                    if cnode.job.status == JobStatus::Blocked {
                        if cnode.job.error_log.as_deref() != Some("Pruned by Logic Condition") {
                            cnode.job.status = JobStatus::Pending;
                            cnode.blocked = false;
                            unblocked.push(*cid);
                        }
                    }
                }
            }
        }

        for cid in unblocked {
            self.dirty_jobs.insert(cid);
            if let Some(n) = self.nodes.get_mut(&cid) {
                if n.is_state_runnable() {
                    n.enqueued = true;
                    self.ready_queue.push_back(cid);
                }
            }
        }
        Ok(())
    }

    async fn expand_generator_defensive(
        &mut self,
        gen_idx: NodeIndex,
        payload: Vec<Value>,
    ) -> Result<()> {
        log::info!("🧠 Evaluating Generator Output...");

        if payload.len() > 100 {
            return Err(anyhow!(
                "Expansion Governor: Request > 100 children. Rejected."
            ));
        }

        let gen_node = &self.workflow.graph[gen_idx];

        let physics_template_val = gen_node
            .job
            .config
            .params
            .get("physics_template")
            .ok_or(anyhow!("Missing physics_template in generator params"))?;

        let physics_template: JobConfig = serde_json::from_value(physics_template_val.clone())?;

        let params = &gen_node.job.config.params;

        // FIXED: Added explicit types |v: &Value| for inference
        let gen_counter = params
            .get("gen_counter")
            .and_then(|v: &Value| v.as_u64())
            .unwrap_or(0);

        let gen_limit = params
            .get("gen_limit")
            .and_then(|v: &Value| v.as_u64())
            .unwrap_or(0);

        let next_agent_config = if gen_counter < gen_limit {
            let mut new_config = gen_node.job.config.clone();
            if let Some(obj) = new_config.params.as_object_mut() {
                obj.insert("gen_counter".to_string(), json!(gen_counter + 1));
            }
            Some(new_config)
        } else {
            None
        };

        self.workflow
            .expand_generator(gen_idx, payload, physics_template, next_agent_config)?;

        self.sync_graph_to_scheduler_with_memoization().await
    }

    async fn sync_graph_to_scheduler_with_memoization(&mut self) -> Result<()> {
        let mut new_jobs = Vec::new();
        let mut new_deps = Vec::new();
        let mut cache_hits = 0;

        for idx in self.workflow.graph.node_indices() {
            let wf_node = &self.workflow.graph[idx];

            if !self.nodes.contains_key(&wf_node.job.id) {
                let mut job = wf_node.job.clone();
                job.flow_context.insert(
                    "node_type".into(),
                    serde_json::to_value(&wf_node.node_type).unwrap(),
                );

                if matches!(wf_node.node_type, NodeType::Compute) {
                    let fp = Self::fingerprint_job(&job.config);
                    if let Some(&existing_id) = self.landscape_registry.get(&fp) {
                        if let Some(existing_node) = self.nodes.get(&existing_id) {
                            if let Some(res) = &existing_node.job.result {
                                log::info!("♻️ Memoization Hit! {}", job.id);
                                job.status = JobStatus::Completed;
                                job.result = Some(res.clone());
                                job.flow_context
                                    .insert("memoized_from".into(), json!(existing_id));
                                cache_hits += 1;
                            }
                        }
                    }
                }

                let parents: Vec<Uuid> = self
                    .workflow
                    .graph
                    .neighbors_directed(idx, Direction::Incoming)
                    .map(|p| self.workflow.graph[p].job.id)
                    .collect();
                job.parent_ids = parents.clone();

                for pid in parents {
                    new_deps.push((pid, job.id));
                }
                new_jobs.push(job);
            }
        }

        if !new_jobs.is_empty() {
            log::info!(
                "🚀 Expansion: {} new jobs ({} memoized)",
                new_jobs.len(),
                cache_hits
            );
            let submit = JobSubmit {
                jobs: new_jobs,
                deps: new_deps,
            };
            self.transport
                .broadcast(EV_JOB_SUBMIT, serde_json::to_value(&submit)?)
                .await?;
            self.ingest_submission(submit);
        }
        Ok(())
    }

    fn sync_pruning_to_scheduler(&mut self) {
        for idx in self.workflow.graph.node_indices() {
            let wf_node = &self.workflow.graph[idx];
            if wf_node.is_pruned {
                if let Some(sched_node) = self.nodes.get_mut(&wf_node.job.id) {
                    if sched_node.job.status != JobStatus::Failed {
                        sched_node.job.status = JobStatus::Failed;
                        sched_node.job.error_log = Some("Pruned by Logic Condition".into());
                        sched_node.blocked = false;
                        self.dirty_jobs.insert(sched_node.job.id);
                    }
                }
            }
        }
    }

    async fn schedule_work(&mut self) -> Result<()> {
        let worker_ids: Vec<String> = self.workers.keys().cloned().collect();

        for wid in worker_ids {
            let (mut cap_cores, mut cap_gpus, worker_tags) = {
                let w = self.workers.get(&wid).unwrap();
                if !w.wants_work || w.inflight_jobs >= 64 {
                    continue;
                }
                (w.available_cores, w.available_gpus, w.tags.clone())
            };

            let mut grant_batch = Vec::new();
            let mut rotated = 0;
            let q_len = self.ready_queue.len();

            while rotated < q_len && cap_cores > 0 {
                if let Some(jid) = self.ready_queue.pop_front() {
                    if let Some(node) = self.nodes.get_mut(&jid) {
                        node.enqueued = false;
                    }

                    let (runnable, tag_match, req_cores, req_gpus) =
                        if let Some(node) = self.nodes.get(&jid) {
                            let is_valid = node.is_runnable_logic_only();
                            if !is_valid {
                                (false, false, 0, 0)
                            } else {
                                let req_tags = &node.job.resources.required_tags;
                                let matches = req_tags.iter().all(|t| worker_tags.contains(t));
                                (
                                    true,
                                    matches,
                                    node.job.resources.cores,
                                    node.job.resources.gpus,
                                )
                            }
                        } else {
                            (false, false, 0, 0)
                        };

                    let fits = req_cores <= cap_cores && req_gpus <= cap_gpus;

                    let mut pushed_back = false;
                    if runnable && tag_match && fits {
                        if let Some(node) = self.nodes.get_mut(&jid) {
                            node.inflight = true;
                            node.assigned_to = Some(wid.clone());
                            node.job.node_id = Some(wid.clone());
                            node.job.status = JobStatus::Running;

                            self.dirty_jobs.insert(jid);
                            grant_batch.push(node.job.clone());

                            cap_cores -= req_cores;
                            cap_gpus -= req_gpus;
                        }
                    } else {
                        pushed_back = true;
                    }

                    if pushed_back {
                        if let Some(node) = self.nodes.get_mut(&jid) {
                            node.enqueued = true;
                        }
                        self.ready_queue.push_back(jid);
                    }
                    rotated += 1;
                } else {
                    break;
                }
            }

            if !grant_batch.is_empty() {
                if let Some(w) = self.workers.get_mut(&wid) {
                    w.inflight_jobs += grant_batch.len();
                    w.wants_work = false;
                }
                let grant = WorkGrant {
                    worker_id: wid.clone(),
                    grant_id: format!("g_{}", Uuid::new_v4()),
                    jobs: grant_batch,
                };
                self.transport
                    .broadcast(EV_WORK_GRANT, serde_json::to_value(&grant)?)
                    .await?;
            }
        }
        Ok(())
    }

    fn maybe_checkpoint(&mut self) -> Result<()> {
        if self.last_ckpt.elapsed() < Duration::from_secs(5) || self.dirty_jobs.is_empty() {
            return Ok(());
        }

        let mut refs = Vec::new();
        for id in &self.dirty_jobs {
            if let Some(n) = self.nodes.get(id) {
                refs.push(&n.job);
            }
        }

        let w_snap: Vec<WorkerInfo> = self
            .workers
            .iter()
            .map(|(id, w)| WorkerInfo {
                worker_id: id.clone(),
                cores: w.available_cores,
                tasks: w.inflight_jobs,
                last_seen_ms: 0,
            })
            .collect();

        self.store.apply_batch(self.global_cursor, &refs, &w_snap)?;
        self.dirty_jobs.clear();
        self.last_ckpt = Instant::now();
        Ok(())
    }

    fn rebuild_ready_queue(&mut self) {
        self.ready_queue.clear();
        for (id, node) in &mut self.nodes {
            node.enqueued = false;
            if node.is_state_runnable() {
                self.ready_queue.push_back(*id);
                node.enqueued = true;
            }
        }
    }

    fn ingest_submission(&mut self, sub: JobSubmit) {
        for job in sub.jobs {
            let completed = job.status == JobStatus::Completed;
            self.nodes.insert(
                job.id,
                NodeState {
                    job: job.clone(),
                    parents_total: 0,
                    parents_done: 0,
                    blocked: false,
                    inflight: false,
                    enqueued: false,
                    assigned_to: None,
                },
            );
            self.dirty_jobs.insert(job.id);
            if completed {
                let finger = Self::fingerprint_job(&job.config);
                self.landscape_registry.insert(finger, job.id);
            }
            if !self.workflow.id_map.contains_key(&job.id) {
                let n_type = job
                    .flow_context
                    .get("node_type")
                    .and_then(|v| serde_json::from_value(v.clone()).ok())
                    .unwrap_or(NodeType::Compute);
                let _ = self
                    .workflow
                    .add_smart_node(job.clone(), n_type, vec![], 50, true);
            }
        }
        for (pid, cid) in sub.deps {
            if let Some(child) = self.nodes.get_mut(&cid) {
                child.parents_total += 1;
                if !child.job.parent_ids.contains(&pid) {
                    child.job.parent_ids.push(pid);
                }
            }
        }
        let completed_or_failed: HashSet<Uuid> = self
            .nodes
            .values()
            .filter(|n| matches!(n.job.status, JobStatus::Completed | JobStatus::Failed))
            .map(|n| n.job.id)
            .collect();
        for (_id, node) in &mut self.nodes {
            if node.job.status == JobStatus::Pending || node.job.status == JobStatus::Blocked {
                node.parents_done = node
                    .job
                    .parent_ids
                    .iter()
                    .filter(|pid| completed_or_failed.contains(pid))
                    .count();
                if node.parents_total > node.parents_done {
                    node.blocked = true;
                    node.job.status = JobStatus::Blocked;
                } else {
                    node.blocked = false;
                    node.job.status = JobStatus::Pending;
                }
            }
        }
        self.rebuild_ready_queue();
    }
}
