// src/workflow.rs
//
// =============================================================================
// UNIFIEDLAB: WORKFLOW ENGINE (v 0.1 )
// =============================================================================
//
// The Graph Brain.
//
// Responsibilities:
// 1. Manage the DAG (Nodes & Dependencies).
// 2. Handle Logic Gates (Switch/If).
// 3. Expand Generators (Active Learning Recursion).
// 4. Content Hashing for Deduplication.

use crate::core::{Engine, Job, JobConfig, JobStatus, ResourceReq, Structure};
use anyhow::Result;
use petgraph::graph::{DiGraph, NodeIndex};
use petgraph::visit::Bfs;
use petgraph::Direction;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use uuid::Uuid;

// Sub-module for parsing Draw.io XML
pub mod importer;

// ============================================================================
// 1. NODE TYPES (Logic & Control Flow)
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum NodeType {
    /// Standard Physics/Inference Job
    Compute,

    /// Spawns child jobs based on results
    Generator { strategy: String },

    /// Prunes branches based on condition
    Switch { condition: LogicCondition },

    /// Collects results from parents
    Aggregator,

    /// Checks consistency between parents
    Verifier { tolerance: f64 },

    /// Start/End markers
    Sentinel,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum LogicCondition {
    AlwaysTrue,
    EnergyBelow(f64),  // eV
    BandGapAbove(f64), // eV
    ExternalScript(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EdgeType {
    HardDependency,
    SoftDependency,
    DataFlow { param_map: HashMap<String, String> },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SmartNode {
    pub job: Job,
    pub node_type: NodeType,
    pub content_hash: String,
    pub priority: u32,
    pub persist: bool,
    pub is_pruned: bool,
    pub is_expanded: bool,
}

// ============================================================================
// 2. THE ENGINE
// ============================================================================

pub struct WorkflowEngine {
    pub graph: DiGraph<SmartNode, EdgeType>,
    pub cache_map: HashMap<String, NodeIndex>,
    pub id_map: HashMap<Uuid, NodeIndex>,
}

impl Default for WorkflowEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl WorkflowEngine {
    pub fn new() -> Self {
        Self {
            graph: DiGraph::new(),
            cache_map: HashMap::new(),
            id_map: HashMap::new(),
        }
    }

    /// Adds a Node to the Graph with De-duplication (Merkle Hashing).
    pub fn add_smart_node(
        &mut self,
        job: Job,
        n_type: NodeType,
        parents: Vec<NodeIndex>,
        priority: u32,
        persist: bool,
    ) -> Result<NodeIndex> {
        // Calculate Content Hash
        // Includes: Job Config (Params + Engine) + Structure + Parent Hashes
        let mut hasher = Sha256::new();
        hasher.update(serde_json::to_string(&job.config)?);
        hasher.update(serde_json::to_string(&job.structure)?);

        // Sort parent hashes to ensure order independence
        let mut parent_hashes: Vec<String> = parents
            .iter()
            .map(|p| self.graph[*p].content_hash.clone())
            .collect();
        parent_hashes.sort();
        for ph in parent_hashes {
            hasher.update(ph);
        }

        let hash = format!("{:x}", hasher.finalize());

        // Deduplication Check
        if let Some(&existing_idx) = self.cache_map.get(&hash) {
            log::debug!("⚡ Merkle Cache Hit: Node {} deduplicated.", job.id);
            return Ok(existing_idx);
        }

        let node = SmartNode {
            job: job.clone(),
            node_type: n_type,
            content_hash: hash.clone(),
            priority,
            persist,
            is_pruned: false,
            is_expanded: false,
        };

        let idx = self.graph.add_node(node);
        self.cache_map.insert(hash, idx);
        self.id_map.insert(job.id, idx);

        for p in parents {
            self.graph.add_edge(p, idx, EdgeType::HardDependency);
        }

        Ok(idx)
    }

    /// Helper: Create an Active Learning Agent Node
    pub fn add_agent_generator(
        &mut self,
        script_path: String,
        strategy: String,
        params: Value,
        parents: Vec<NodeIndex>,
    ) -> Result<NodeIndex> {
        let config = JobConfig {
            engine: Engine::Agent {
                script_path: script_path.clone(),
                strategy: strategy.clone(),
            },
            params,
        };

        let job = Job::new(
            Structure::new(vec![], None, "Agent_Gen".into()),
            config,
            ResourceReq {
                nodes: 1,
                cores: 1,
                gpus: 0,
                time_limit_min: 30,
                required_tags: vec!["brain".into()],
            },
        );

        self.add_smart_node(job, NodeType::Generator { strategy }, parents, 100, true)
    }

    // ========================================================================
    // 3. LOGIC RESOLUTION
    // ========================================================================

    pub fn resolve_logic_branch(&mut self, switch_idx: NodeIndex, result_data: &Value) {
        if let NodeType::Switch { condition } = &self.graph[switch_idx].node_type {
            let passed = match condition {
                LogicCondition::EnergyBelow(threshold) => {
                    result_data
                        .get("energy")
                        .and_then(|v| v.as_f64())
                        .unwrap_or(0.0)
                        < *threshold
                }
                LogicCondition::BandGapAbove(threshold) => {
                    result_data
                        .get("band_gap")
                        .and_then(|v| v.as_f64())
                        .unwrap_or(0.0)
                        > *threshold
                }
                LogicCondition::AlwaysTrue => true,
                LogicCondition::ExternalScript(_) => true,
            };

            if !passed {
                log::info!("✂️ Switch Node Triggered: Pruning downstream branch.");
                self.prune_subgraph(switch_idx);
            }
        }
    }

    fn prune_subgraph(&mut self, start_idx: NodeIndex) {
        let mut bfs = Bfs::new(&self.graph, start_idx);
        while let Some(idx) = bfs.next(&self.graph) {
            if idx == start_idx {
                continue;
            }

            let node = &mut self.graph[idx];
            if !node.is_pruned {
                node.is_pruned = true;
                node.job.status = JobStatus::Failed;
                node.job.error_log = Some("Pruned by Logic Condition".into());
            }
        }
    }

    // ========================================================================
    // 4. RECURSIVE EXPANSION (The Active Learning Loop)
    // ========================================================================

    pub fn expand_generator(
        &mut self,
        generator_idx: NodeIndex,
        generated_candidates: Vec<Value>,
        physics_template: JobConfig,
        next_agent_config: Option<JobConfig>,
    ) -> Result<()> {
        if self.graph[generator_idx].is_expanded {
            return Ok(());
        }
        self.graph[generator_idx].is_expanded = true;

        let mut physics_indices = Vec::new();

        // 1. Spawn Child Jobs (Physics)
        for (i, cand) in generated_candidates.iter().enumerate() {
            let mut cfg = physics_template.clone();

            // Inject Candidate into Params
            if let Some(obj) = cfg.params.as_object_mut() {
                obj.insert("candidate".to_string(), cand.clone());
                // Provenance: Track who generated this
                obj.insert(
                    "generated_by".to_string(),
                    json!(self.graph[generator_idx].job.id),
                );
            }

            let job = Job::new(
                // Placeholder structure; driver will load real structure from params
                Structure::new(vec![], None, format!("Sim_{}_{}", generator_idx.index(), i)),
                cfg,
                // Using standard MLIP defaults (GPU) for Phase 6.
                ResourceReq {
                    cores: 8,
                    gpus: 1,
                    ..Default::default()
                },
            );

            let idx = self.add_smart_node(job, NodeType::Compute, vec![generator_idx], 50, true)?;
            physics_indices.push(idx);
        }

        // 2. Spawn Next Agent (Recursion)
        if let Some(agent_cfg) = next_agent_config {
            let gen_count = agent_cfg
                .params
                .get("gen_counter")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);

            let agent_job = Job::new(
                Structure::new(vec![], None, format!("Agent_Gen{}", gen_count)),
                agent_cfg,
                ResourceReq {
                    nodes: 1,
                    cores: 1,
                    gpus: 0,
                    required_tags: vec!["brain".into()],
                    ..Default::default()
                },
            );

            // Extract strategy for NodeType metadata
            let strategy = match &agent_job.config.engine {
                Engine::Agent { strategy, .. } => strategy.clone(),
                _ => "custom".into(),
            };

            self.add_smart_node(
                agent_job,
                NodeType::Generator { strategy },
                physics_indices, // Depends on the physics batch
                100,
                true,
            )?;
        }

        self.recalculate_priorities();
        Ok(())
    }

    pub fn recalculate_priorities(&mut self) {
        let mut topo_order = petgraph::algo::toposort(&self.graph, None).unwrap_or_default();
        topo_order.reverse();

        for idx in topo_order {
            let mut max_child_prio = 0;
            let mut children = self.graph.neighbors_directed(idx, Direction::Outgoing);

            while let Some(child_idx) = children.next() {
                max_child_prio = std::cmp::max(max_child_prio, self.graph[child_idx].priority);
            }

            if max_child_prio > 0 {
                self.graph[idx].priority = max_child_prio + 1;
            }
        }
    }
}
