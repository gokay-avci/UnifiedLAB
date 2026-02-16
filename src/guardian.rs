// src/guardian.rs
//
// =============================================================================
// UNIFIEDLAB: NODE GUARDIAN (v 0.1 )
// =============================================================================
//
// The Local Scheduler.
//
// Responsibilities:
// 1. Owns the hardware (ResourceLedger).
// 2. Plays "Tetris" with jobs (fitting them onto available cores/GPUs).
// 3. Manages the lifecycle of Drivers (Setup -> Run -> Teardown).
// 4. Updates the Checkpoint DB with final results.

use crate::checkpoint::CheckpointStore;
use crate::core::{Job, JobStatus};
use crate::drivers::DriverFactory;
use crate::provenance::ArtifactStore;
use crate::resources::{ResourceLedger, Sandbox};

use anyhow::Result;
use chrono::Utc;
use std::path::Path;
use std::sync::Arc;
use tokio::fs;
use tokio::sync::{Mutex, Semaphore};

// ============================================================================
// 1. THE GUARDIAN
// ============================================================================

#[derive(Clone)]
pub struct NodeGuardian {
    pub id: String,

    // Hardware Inventory (Protected for concurrent access)
    // We lock this briefly only to Allocate/Free resources.
    ledger: Arc<Mutex<ResourceLedger>>,

    // Persistence
    artifact_store: Arc<ArtifactStore>,
    db_store: Arc<CheckpointStore>,

    // Concurrency Limit
    // Prevents the OS from OOMing if we try to spawn 10,000 threads for
    // 10,000 tiny jobs. Limits active tasks to roughly 2x core count.
    task_limiter: Arc<Semaphore>,
}

impl NodeGuardian {
    pub async fn boot(
        id: String,
        root_path: impl AsRef<Path>,
        db_store: CheckpointStore,
    ) -> Result<Self> {
        let root = root_path.as_ref();

        // 1. Detect Topology
        let ledger = ResourceLedger::detect();

        // 2. Init Artifact Store (CAS)
        let artifact_path = root.join("store");
        let artifact_store = ArtifactStore::new(&artifact_path)?;

        // 3. Init Concurrency
        // Allow slightly more tasks than cores to handle I/O bound agents
        let total_cores = ledger.total_cores();
        let max_tasks = (total_cores * 2).max(4);

        log::info!("Guardian {} ready. Max concurrent tasks: {}", id, max_tasks);

        Ok(Self {
            id,
            ledger: Arc::new(Mutex::new(ledger)),
            artifact_store: Arc::new(artifact_store),
            db_store: Arc::new(db_store),
            task_limiter: Arc::new(Semaphore::new(max_tasks)),
        })
    }

    /// **NEW:** Helper to get current resource availability for Heartbeats.
    /// This prevents the "Lying Heartbeat" bug by reporting ACTUAL free count.
    pub async fn get_capacity(&self) -> (usize, usize) {
        let ledger = self.ledger.lock().await;
        (ledger.free_cores(), ledger.free_gpus())
    }

    /// The Main Entry Point.
    /// Tries to accept a job. Returns true if accepted (spawned), false if rejected (no resources).
    pub async fn try_accept_job(&self, job: Job) -> bool {
        // 1. Check Concurrency Limit (Fail fast if system overloaded)
        let permit = match self.task_limiter.clone().try_acquire_owned() {
            Ok(p) => p,
            Err(_) => return false,
        };

        // 2. Check Hardware Resources (The Tetris Step)
        let sandbox = {
            let mut ledger = self.ledger.lock().await;
            ledger.try_allocate(job.resources.cores, job.resources.gpus)
        };

        match sandbox {
            Some(sb) => {
                log::info!(
                    "✅ Job {} accepted. Assigned: {}",
                    job.id.to_string().chars().take(8).collect::<String>(),
                    self.fmt_sandbox(&sb)
                );

                // Spawn the execution task detached from the main loop
                let guardian_ref = self.clone();
                tokio::spawn(async move {
                    guardian_ref.execute_lifecycle(job, sb).await;
                    drop(permit); // Release semaphore only after job finishes
                });

                true
            }
            None => {
                // Not enough cores/GPUs right now.
                // The caller (main loop) should keep it in the backlog.
                false
            }
        }
    }

    fn fmt_sandbox(&self, sb: &Sandbox) -> String {
        let c = if sb.cores.len() > 4 {
            format!(
                "Cores[{}..{}]",
                sb.cores.first().unwrap(),
                sb.cores.last().unwrap()
            )
        } else {
            format!("Cores{:?}", sb.cores)
        };
        format!("{} GPUs{:?}", c, sb.gpus)
    }
}

// ============================================================================
// 2. THE EXECUTION LIFECYCLE
// ============================================================================

impl NodeGuardian {
    async fn execute_lifecycle(&self, mut job: Job, sandbox: Sandbox) {
        let job_id = job.id;

        // A. SETUP WORKSPACE
        // Use a temp directory for the execution duration.
        // On HPC, this usually maps to /tmp or $TMPDIR (often local NVMe).
        let work_dir_name = format!("ulab_{}", job_id);
        let work_dir = std::env::temp_dir().join(&work_dir_name);

        if let Err(e) = fs::create_dir_all(&work_dir).await {
            self.fail_job(job, "Workspace Creation Failed", e.to_string())
                .await;
            self.free_resources(&sandbox).await;
            return;
        }

        // Update DB: Running
        // We do this optimistically. If DB fails, we log but continue.
        job.status = JobStatus::Running;
        job.node_id = Some(self.id.clone());
        job.updated_at = Utc::now();

        if let Err(e) = self.db_store.apply_batch(0, &[&job], &[]) {
            log::warn!("Failed to mark job {} as running: {}", job_id, e);
        }

        // B. EXECUTE DRIVER
        let result = async {
            let driver = DriverFactory::get(&job.config.engine)?;
            driver.execute(&job, &sandbox, &work_dir).await
        }
        .await;

        // C. FINALIZE & CLEANUP
        match result {
            Ok(calc_res) => {
                job.status = JobStatus::Completed;
                job.result = Some(calc_res);
                job.updated_at = Utc::now();

                // Save complete state to DB
                if let Err(e) = self.db_store.apply_batch(0, &[&job], &[]) {
                    log::error!("Failed to save result for Job {} to DB: {}", job_id, e);
                } else {
                    log::info!(
                        "🏁 Job {} Finished. Time: {:.2}s",
                        job_id,
                        job.result.as_ref().unwrap().t_total_ms / 1000.0
                    );
                }
            }
            Err(e) => {
                self.fail_job(job, "Driver Error", e.to_string()).await;
            }
        }

        // D. TEARDOWN
        // 1. Free Hardware (CRITICAL: Must happen even on panic/error)
        self.free_resources(&sandbox).await;

        // 2. Remove Workspace (Cleanup)
        // We only clean up if successful or if configured to always clean.
        if let Err(e) = fs::remove_dir_all(&work_dir).await {
            log::warn!("Failed to cleanup workspace {:?}: {}", work_dir, e);
        }
    }

    async fn free_resources(&self, sandbox: &Sandbox) {
        let mut ledger = self.ledger.lock().await;
        ledger.free(sandbox);
    }

    async fn fail_job(&self, mut job: Job, reason: &str, details: String) {
        log::error!(
            "💥 Job {} Failed: {} - {}",
            job.id.to_string().chars().take(8).collect::<String>(),
            reason,
            details
        );

        job.status = JobStatus::Failed;
        job.error_log = Some(format!("{}: {}", reason, details));
        job.updated_at = Utc::now();

        if let Err(e) = self.db_store.apply_batch(0, &[&job], &[]) {
            log::error!(
                "Failed to save failure state for Job {} to DB: {}",
                job.id,
                e
            );
        }
    }
}
