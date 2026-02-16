// src/main.rs
//
// =============================================================================
// UNIFIEDLAB: COMMANDER & ENTRY POINT (v 0.1 )
// =============================================================================
//
// The wiring center of the entire architecture.
//
// Modes:
// 1. START:  Boots the NodeGuardian (Resource Manager) and Coordinator (Lighthouse).
// 2. DEPLOY: Parses Blueprint (.drawio), injects params, submits to Cluster.
// 3. TUI:    Launches the Terminal Dashboard.
//
// Key Features:
// - Auto-Detection of Roles (Rank 0 vs Rank N).
// - Smart Tagging (Brain vs Muscle).
// - Graceful Shutdown handling.
// - True Capacity Heartbeats (Prevent over-scheduling).

use anyhow::{anyhow, Context, Result};
use clap::{Parser, Subcommand};
use serde_json::Value;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::{Duration, Instant};
use tokio::signal;
use tokio::time::sleep;

// --- MODULES ---
mod checkpoint;
mod core;
mod drivers;
mod eventlog;
mod guardian;
mod logs;
mod marketplace;
mod physics;
mod provenance;
mod resources;
mod transport;
mod tui;
mod workflow;

use crate::checkpoint::CheckpointStore;
use crate::core::{Job, JobStatus};
use crate::guardian::NodeGuardian;
use crate::logs::{LogBuffer, TuiLogger};
use crate::marketplace::{
    JobSubmit, MarketplaceCoordinator, WorkGrant, WorkRequest, EV_JOB_SUBMIT, EV_WORK_GRANT,
    MSG_WORK_REQUEST,
};
use crate::resources::{ClusterType, ResourceLedger};
use crate::transport::{FileTransport, Role, Transport};
use crate::workflow::importer::DrawIoLoader;
use crate::workflow::NodeType;

// ============================================================================
// 1. CLI DEFINITION
// ============================================================================

#[derive(Parser)]
#[command(
    name = "unifiedlab",
    version = "6.2",
    about = "HPC Scientific Orchestrator"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Start the Node Service (Guardian + Coordinator).
    Start {
        /// Root directory for DB and Logs.
        #[arg(long, default_value = ".")]
        root: String,

        /// Force execution on local machine (safety check).
        #[arg(long)]
        force_local: bool,

        /// Manually force a Worker ID (default: hostname_rank).
        #[arg(long)]
        id: Option<String>,

        /// Manual tags (e.g. "gpu", "highmem"). Overrides auto-detection.
        /// Use: --tags brain --tags muscle
        #[arg(long, num_args = 1..)]
        tags: Vec<String>,
    },

    /// Deploy a Blueprint (.drawio) to the cluster.
    Deploy {
        /// Path to .drawio file.
        #[arg(long)]
        file: String,

        /// Root directory.
        #[arg(long, default_value = ".")]
        root: String,

        /// JSON string to override params (e.g. '{"gen_limit": 50}').
        #[arg(long)]
        params: Option<String>,
    },

    /// Launch Monitoring Dashboard.
    Tui {
        #[arg(long, default_value = "checkpoint.db")]
        checkpoint: String,
    },
}

// ============================================================================
// 2. ENTRY POINT
// ============================================================================

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Init Logger (standard env_logger unless TUI mode)
    if !matches!(cli.command, Commands::Tui { .. }) {
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    }

    match cli.command {
        Commands::Start {
            root,
            force_local,
            id,
            tags,
        } => run_node_service(root, force_local, id, tags).await,
        Commands::Deploy { file, root, params } => run_deployer(file, root, params).await,
        Commands::Tui { checkpoint } => run_tui(checkpoint),
    }
}

// ============================================================================
// 3. RUNTIME: NODE SERVICE (Guardian + Coordinator)
// ============================================================================

async fn run_node_service(
    root: String,
    force_local: bool,
    manual_id: Option<String>,
    manual_tags: Vec<String>,
) -> Result<()> {
    let root_path = PathBuf::from(&root);
    let shutdown_signal = Arc::new(AtomicBool::new(false));

    // A. DETECT ENVIRONMENT & TOPOLOGY
    // We use the Ledger to see where we are (Slurm vs Local).
    let ledger = ResourceLedger::detect();

    // Safety: Prevent accidental heavy runs on laptops without --force-local
    if ledger.cluster_type == ClusterType::Local && !force_local {
        return Err(anyhow!(
            "SAFETY: Local execution detected. Use --force-local to run on workstation/laptop."
        ));
    }

    // Identify Rank (0 = Coordinator/Lighthouse)
    // In Slurm, SLURM_PROCID is reliable. Locally, we default to 0.
    let rank = std::env::var("SLURM_PROCID").unwrap_or_else(|_| "0".into());
    let is_coordinator = rank == "0";

    let worker_id = manual_id.unwrap_or_else(|| format!("{}_r{}", ledger.hostname, rank));

    // B. SMART TAGGING STRATEGY
    // Brain = Can run Agents/Generators. Muscle = Can run heavy physics.
    let tags = if !manual_tags.is_empty() {
        manual_tags
    } else if ledger.cluster_type == ClusterType::Local {
        // Local: Must be everything
        vec!["brain".into(), "muscle".into(), "gpu".into()]
    } else if is_coordinator {
        // Rank 0: The Brain (manages DB, runs light Agents)
        vec!["brain".into()]
    } else {
        // Rank N: The Muscle (runs heavy Physics)
        vec!["muscle".into(), "gpu".into()] // Assumes GPU nodes
    };

    log::info!(
        "🚀 Booting Node {} | Role: Guardian {}",
        worker_id,
        if is_coordinator {
            "+ Lighthouse 👑"
        } else {
            ""
        }
    );
    log::info!("🏷️  Capabilities: {:?}", tags);

    // C. BOOT COORDINATOR (If Rank 0)
    let db_path = root_path.join("checkpoint.db");
    let store = CheckpointStore::open(&db_path).context("DB Init")?;

    if is_coordinator {
        let coord_root = root_path.clone();
        let coord_sig = shutdown_signal.clone();
        let coord_store = CheckpointStore::open(&db_path)?; // Clone connection

        tokio::spawn(async move {
            log::info!("👑 Lighthouse Service Starting...");
            if let Err(e) = run_coordinator_loop(coord_root, coord_store, coord_sig).await {
                log::error!("👑 Lighthouse CRASHED: {}", e);
                std::process::exit(1); // Fatal
            }
        });
        // Give DB a moment to settle
        sleep(Duration::from_millis(500)).await;
    }

    // D. BOOT GUARDIAN (The Local Scheduler)
    let guardian = NodeGuardian::boot(worker_id.clone(), &root_path, store).await?;

    // Transport for this worker (Inbox Reader)
    let mut transport = FileTransport::new(&root_path, Role::Worker, Some(&worker_id)).await?;

    // E. SIGNAL HANDLING
    let sig_term = shutdown_signal.clone();
    tokio::spawn(async move {
        signal::ctrl_c().await.ok();
        log::warn!("🛑 Interrupt received. Stopping...");
        sig_term.store(true, Ordering::SeqCst);
    });

    // F. MAIN EVENT LOOP
    log::info!("🛡️ Guardian Active. Polling inbox...");

    // Local Backlog: Jobs accepted by protocol but waiting for Guardian resources
    let mut backlog: VecDeque<Job> = VecDeque::new();
    let mut last_heartbeat = Instant::now();
    let hb_interval = Duration::from_secs(10);

    while !shutdown_signal.load(Ordering::SeqCst) {
        // 1. HEARTBEAT
        if last_heartbeat.elapsed() > hb_interval {
            // FIX: Ask Guardian for REAL capacity.
            // This ensures we report what is actually free in the Ledger bitmask.
            let (free_cores, free_gpus) = guardian.get_capacity().await;

            let req = WorkRequest {
                worker_id: worker_id.clone(),
                available_cores: free_cores,
                available_gpus: free_gpus,
                max_jobs: 64, // Queue depth limit
                tags: tags.clone(),
            };

            // We write to our own output log which Coordinator reads
            if let Err(e) = transport
                .send_to_coordinator(MSG_WORK_REQUEST, serde_json::to_value(&req)?)
                .await
            {
                log::error!("Heartbeat failed: {}", e);
            }
            last_heartbeat = Instant::now();
        }

        // 2. PROCESS BACKLOG (Try to shove queued jobs into Guardian)
        let mut rotated = 0;
        let q_len = backlog.len();
        while rotated < q_len {
            if let Some(job) = backlog.pop_front() {
                if guardian.try_accept_job(job.clone()).await {
                    // Success: Guardian took it
                } else {
                    // Fail: Resources still full, rotate back
                    backlog.push_back(job);
                }
            }
            rotated += 1;
        }

        // 3. CHECK INBOX (New Grants)
        let events = transport.recv_broadcasts().await.unwrap_or_default();
        for env in events {
            if env.record.kind == EV_WORK_GRANT {
                if let Ok(grant) = serde_json::from_value::<WorkGrant>(env.record.payload) {
                    if grant.worker_id == worker_id {
                        log::info!(
                            "📨 Received Grant {} ({} jobs)",
                            grant.grant_id,
                            grant.jobs.len()
                        );

                        for job in grant.jobs {
                            if !guardian.try_accept_job(job.clone()).await {
                                log::debug!("⏳ Job {} queued locally (Busy)", job.id);
                                backlog.push_back(job);
                            }
                        }
                    }
                }
            }
        }

        // 4. PREVENT BUSY LOOP
        sleep(Duration::from_millis(200)).await; // this section is critical as it defines how long each operation awaits for min
    }

    log::info!("👋 Node Shutdown Complete.");
    Ok(())
}

// Logic for Rank 0
async fn run_coordinator_loop(
    root: PathBuf,
    store: CheckpointStore,
    stop_signal: Arc<AtomicBool>,
) -> Result<()> {
    let transport = FileTransport::new(&root, Role::Coordinator, None)
        .await
        .context("Coord Transport")?;

    let mut coord = MarketplaceCoordinator::open(Box::new(transport), store).await?;
    log::info!("✅ Coordinator Logic Active.");

    while !stop_signal.load(Ordering::SeqCst) {
        if let Err(e) = coord.tick().await {
            log::error!("Coordinator Tick Error: {}", e);
        }
        sleep(Duration::from_millis(100)).await;
    }
    Ok(())
}

// ============================================================================
// 4. DEPLOYER: THE ARCHITECT
// ============================================================================

async fn run_deployer(file: String, root: String, overrides: Option<String>) -> Result<()> {
    let root_path = PathBuf::from(&root);
    log::info!("📐 Parsing Blueprint: {}", file);

    // 1. Load Blueprint
    let mut loader = DrawIoLoader::load_from_file(&file).context("Failed to load Draw.io")?;

    // FIX: Access internal graph structure via .graph.graph
    log::info!("   Found {} nodes.", loader.graph.graph.node_count());

    // 2. Apply Overrides
    if let Some(ov) = overrides {
        let ov_json: Value = serde_json::from_str(&ov).context("Invalid overrides JSON")?;
        log::info!("   Applying overrides: {}", ov);

        for idx in loader.graph.graph.node_indices() {
            let node = &mut loader.graph.graph[idx];
            if matches!(node.node_type, NodeType::Generator { .. }) {
                if let Some(params) = node.job.config.params.as_object_mut() {
                    if let Some(ov_obj) = ov_json.as_object() {
                        for (k, v) in ov_obj {
                            params.insert(k.clone(), v.clone());
                        }
                    }
                }
            }
        }
    }

    // 3. Setup Transport (As Architect)
    // The architect acts like a "Worker" who only sends EV_JOB_SUBMIT
    let arch_id = format!(
        "architect_{}",
        uuid::Uuid::new_v4()
            .to_string()
            .chars()
            .take(8)
            .collect::<String>()
    );
    let mut transport = FileTransport::new(&root_path, Role::Worker, Some(&arch_id)).await?;

    // 4. Construct Payload
    let mut jobs = Vec::new();
    let mut deps = Vec::new();

    for idx in loader.graph.graph.node_indices() {
        let node = &loader.graph.graph[idx];
        let mut job = node.job.clone();

        // Critical: Inject Flow Context so Coordinator knows Node Type
        job.flow_context
            .insert("node_type".into(), serde_json::to_value(&node.node_type)?);
        job.status = JobStatus::Pending;
        jobs.push(job);
    }

    // Extract Edges
    use petgraph::visit::EdgeRef;
    for edge in loader.graph.graph.edge_references() {
        let src = loader.graph.graph[edge.source()].job.id;
        let dst = loader.graph.graph[edge.target()].job.id;
        deps.push((src, dst));
    }

    // 5. Submit
    let submit = JobSubmit { jobs, deps };
    transport
        .send_to_coordinator(EV_JOB_SUBMIT, serde_json::to_value(&submit)?)
        .await?;

    log::info!("🚀 Blueprint Deployed to Inbox!");
    Ok(())
}

// ============================================================================
// 5. TUI: THE DASHBOARD
// ============================================================================

fn run_tui(checkpoint: String) -> Result<()> {
    if !Path::new(&checkpoint).exists() {
        return Err(anyhow!("DB not found at: {}", checkpoint));
    }

    // Redirect logs to memory buffer so they don't break TUI
    let log_buf = LogBuffer::new(200); // does this have to match with 200 ms timing default?
    TuiLogger::init(log_buf.clone()).ok();

    crate::tui::TuiApp::new(&checkpoint, log_buf).run()?;
    Ok(())
}
