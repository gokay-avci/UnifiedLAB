// src/resources.rs
//
// =============================================================================
// UNIFIEDLAB: RESOURCE LEDGER & TOPOLOGY (v 0.1 )
// =============================================================================
//
// The Inventory.
//
// Responsibilities:
// 1. Detect Topology (Local vs Slurm vs PBS).
// 2. Manage Resource Bitmasks (Track specific Core/GPU IDs).
// 3. Issue "Sandboxes" (Allocations) to jobs.
// 4. Generate Isolation Env Vars (CUDA_VISIBLE_DEVICES, OMP_NUM_THREADS).
//
// TO DO :
//  A) Expansıon towards edge case sandbox environments
//  B) Improve on who leads the MPI ranks and OMP / MPI Hybrid workflow management

use serde::{Deserialize, Serialize};
use std::env;
use sysinfo::{MemoryRefreshKind, RefreshKind, System};
use tokio::process::Command;

// ============================================================================
// 1. DATA STRUCTURES
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ClusterType {
    Local,
    Slurm,
    Pbs,
}

/// A specific allocation of hardware.
/// Acts as a "Receipt". Used to apply isolation constraints to processes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Sandbox {
    pub cores: Vec<usize>, // Logical Core IDs (e.g., [0, 1, 2, 3])
    pub gpus: Vec<usize>,  // GPU Device IDs (e.g., [0])
    pub memory_mb_limit: Option<usize>,
}

impl Sandbox {
    /// Applies this sandbox to a Tokio Command (Environment & Affinity).
    pub fn apply(&self, cmd: &mut Command) {
        // 1. Thread Constraints
        // Stop MKL/OpenMP from spawning threads for every core on the machine
        let thread_count = self.cores.len().to_string();
        cmd.env("OMP_NUM_THREADS", &thread_count);
        cmd.env("MKL_NUM_THREADS", &thread_count);
        cmd.env("RAYON_NUM_THREADS", &thread_count);
        cmd.env("OPENBLAS_NUM_THREADS", &thread_count);

        // 2. GPU Isolation (The Blinders)
        if !self.gpus.is_empty() {
            let gpu_list = self
                .gpus
                .iter()
                .map(|id| id.to_string())
                .collect::<Vec<_>>()
                .join(",");
            cmd.env("CUDA_VISIBLE_DEVICES", &gpu_list);
            cmd.env("ROCR_VISIBLE_DEVICES", &gpu_list); // AMD support
        } else {
            // If no GPUs allocated, explicitly hide all to prevent accidental usage
            cmd.env("CUDA_VISIBLE_DEVICES", "");
            cmd.env("ROCR_VISIBLE_DEVICES", "");
        }

        // 3. CPU Affinity Hint
        // We export this so a wrapper script (like 'taskset') can use it if needed.
        let core_list = self
            .cores
            .iter()
            .map(|id| id.to_string())
            .collect::<Vec<_>>()
            .join(",");
        cmd.env("ULAB_PINNED_CORES", core_list);
    }
}

// ============================================================================
// 2. THE LEDGER (State Tracker)
// ============================================================================

pub struct ResourceLedger {
    pub cluster_type: ClusterType,
    pub hostname: String,

    // Inventory Limits
    total_cores: usize,
    total_gpus: usize,
    total_mem_mb: u64,

    // Bitmasks (True = Busy)
    core_mask: Vec<bool>,
    gpu_mask: Vec<bool>,
}

impl ResourceLedger {
    /// Detects the environment and initializes the ledger.
    pub fn detect() -> Self {
        let (ctype, cores, mem) = Self::detect_cpu_mem();
        let gpus = Self::detect_gpus();
        let hostname = hostname::get()
            .map(|h| h.to_string_lossy().to_string())
            .unwrap_or_else(|_| "localhost".into());

        // In Local mode, reserve 1 core for the OS/Guardian if we have plenty
        let usable_cores = if ctype == ClusterType::Local && cores > 4 {
            cores - 1
        } else {
            cores
        };

        // Initialize masks as all False (Free)
        let mut core_mask = vec![false; cores];

        // If we reserved a core, mark the last one as busy permanently
        if usable_cores < cores {
            core_mask[cores - 1] = true;
        }

        log::info!(
            "Detected resources on {}: Type={:?}, Cores={} (Usable={}), GPUs={}, Mem={}MB",
            hostname,
            ctype,
            cores,
            usable_cores,
            gpus,
            mem
        );

        Self {
            cluster_type: ctype,
            hostname,
            total_cores: cores,
            total_gpus: gpus,
            total_mem_mb: mem,
            core_mask,
            gpu_mask: vec![false; gpus],
        }
    }

    /// Try to allocate a specific amount of resources.
    /// Returns a Sandbox if successful, None if not enough resources.
    pub fn try_allocate(&mut self, req_cores: usize, req_gpus: usize) -> Option<Sandbox> {
        // 1. Check GPU Availability
        let free_gpus = self.find_free_indices(&self.gpu_mask, req_gpus);
        if free_gpus.len() < req_gpus {
            return None;
        }

        // 2. Check Core Availability
        let free_cores = self.find_free_indices(&self.core_mask, req_cores);
        if free_cores.len() < req_cores {
            return None;
        }

        // 3. Commit Allocation (Mark Busy)
        for &idx in &free_gpus {
            self.gpu_mask[idx] = true;
        }
        for &idx in &free_cores {
            self.core_mask[idx] = true;
        }

        Some(Sandbox {
            cores: free_cores,
            gpus: free_gpus,
            memory_mb_limit: None,
        })
    }

    /// Returns resources to the pool.
    pub fn free(&mut self, sandbox: &Sandbox) {
        for &idx in &sandbox.gpus {
            if idx < self.gpu_mask.len() {
                self.gpu_mask[idx] = false;
            }
        }
        for &idx in &sandbox.cores {
            if idx < self.core_mask.len() {
                self.core_mask[idx] = false;
            }
        }
    }

    pub fn total_cores(&self) -> usize {
        self.total_cores
    }

    // --- ACCESSORS FOR HEARTBEAT ---

    /// Returns the count of currently available CPU cores.
    pub fn free_cores(&self) -> usize {
        self.core_mask.iter().filter(|&&busy| !busy).count()
    }

    /// Returns the count of currently available GPUs.
    pub fn free_gpus(&self) -> usize {
        self.gpu_mask.iter().filter(|&&busy| !busy).count()
    }

    /// Helper: Find N contiguous free indices if possible, or fragmented.
    fn find_free_indices(&self, mask: &[bool], count: usize) -> Vec<usize> {
        let mut indices = Vec::with_capacity(count);
        for (i, &is_busy) in mask.iter().enumerate() {
            if !is_busy {
                indices.push(i);
                if indices.len() == count {
                    break;
                }
            }
        }
        indices
    }
}

// ============================================================================
// 3. DETECTION LOGIC
// ============================================================================

impl ResourceLedger {
    fn detect_cpu_mem() -> (ClusterType, usize, u64) {
        // 1. Slurm Check
        if env::var("SLURM_JOB_ID").is_ok() {
            let cores = env::var("SLURM_CPUS_ON_NODE")
                .ok()
                .and_then(|s| {
                    s.split(|c: char| !c.is_numeric())
                        .next()?
                        .parse::<usize>()
                        .ok()
                })
                .unwrap_or_else(num_cpus::get);

            let mut sys = System::new_with_specifics(
                RefreshKind::nothing().with_memory(MemoryRefreshKind::everything()),
            );
            sys.refresh_memory();
            let mem = sys.total_memory() / 1024 / 1024;

            return (ClusterType::Slurm, cores, mem);
        }

        // 2. PBS Check
        if env::var("PBS_JOBID").is_ok() {
            let cores = env::var("NCPUS")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or_else(num_cpus::get);
            let mut sys = System::new();
            sys.refresh_memory();
            return (ClusterType::Pbs, cores, sys.total_memory() / 1024 / 1024);
        }

        // 3. Local Fallback
        let cores = num_cpus::get();
        let mut sys = System::new();
        sys.refresh_memory();
        (ClusterType::Local, cores, sys.total_memory() / 1024 / 1024)
    }

    fn detect_gpus() -> usize {
        // 1. NVIDIA Check
        if let Ok(output) = std::process::Command::new("nvidia-smi")
            .args(&["--query-gpu=name", "--format=csv,noheader"])
            .output()
        {
            let count = String::from_utf8_lossy(&output.stdout)
                .trim()
                .lines()
                .filter(|l| !l.is_empty())
                .count();
            if count > 0 {
                return count;
            }
        }

        // 2. Apple Silicon Check (M1/M2/M3)
        if std::env::consts::OS == "macos" && std::env::consts::ARCH == "aarch64" {
            return 1;
        }

        0
    }
}

// ============================================================================
// 4. SYSTEM MONITOR HELPER (For TUI)
// ============================================================================

pub struct SystemMonitor;

impl SystemMonitor {
    pub fn new() -> Self {
        Self
    }

    // Quick snapshot for TUI Sidebar
    pub fn snapshot(&mut self) -> crate::resources::ResourceLedger {
        ResourceLedger::detect()
    }
}
