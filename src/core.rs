// src/core.rs
//
// =============================================================================
// UNIFIEDLAB: CORE SCHEMA AUTHORITY (v 0.1 )
// =============================================================================
//
// The "Esperanto" of the laboratory.
// This file defines the strict data contracts between Rust (Orchestrator)
// and the execution layer (Python/Binaries).
//
// Design Principles:
// 1. Newtype Pattern: Prevent unit errors (eV vs Joules).
// 2. Polymorphic Engines: Explicitly typed calculation backends.
// 3. Provenance: Every result carries its history.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use uuid::Uuid;

// ============================================================================
// 1. TYPE-SAFE UNITS (The "Newtype" Pattern)
// ============================================================================

/// Energy in Electron Volts (eV).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct ElectronVolts(pub f64);

/// Distance in Angstroms (Å).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct Angstroms(pub f64);

/// Forces in eV/Å.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct Force(pub f64);

// ============================================================================
// 2. THE UNIFIED LAB OBJECT (Structure Definition)
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Atom {
    pub symbol: String,
    pub position: [f64; 3], // [x, y, z] in Angstroms
    #[serde(default)]
    pub charge: Option<f64>,
    #[serde(default)]
    pub magnetic_moment: Option<f64>,
    // Arbitrary tags for specific engines (e.g. "spin=high")
    #[serde(default)]
    pub tags: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Lattice {
    pub vectors: [[f64; 3]; 3], // 3x3 Matrix
    pub pbc: [bool; 3],         // Periodic Boundary Conditions
}

impl Lattice {
    pub fn volume(&self) -> f64 {
        let a = self.vectors[0];
        let b = self.vectors[1];
        let c = self.vectors[2];
        let cross_x = a[1] * b[2] - a[2] * b[1];
        let cross_y = a[2] * b[0] - a[0] * b[2];
        let cross_z = a[0] * b[1] - a[1] * b[0];
        (cross_x * c[0] + cross_y * c[1] + cross_z * c[2]).abs()
    }
}

/// The Universal Structure Definition.
/// Compatible with ASE (Python) via JSON serialization.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Structure {
    #[serde(default = "Uuid::new_v4")]
    pub id: Uuid,

    pub atoms: Vec<Atom>,
    pub lattice: Option<Lattice>,

    #[serde(default)]
    pub source: String, // e.g., "generated_by_agent_007"

    #[serde(default)]
    pub metadata: HashMap<String, Value>,
}

impl Structure {
    pub fn new(atoms: Vec<Atom>, lattice: Option<Lattice>, source: String) -> Self {
        Self {
            id: Uuid::new_v4(),
            atoms,
            lattice,
            source,
            metadata: HashMap::new(),
        }
    }

    /// Approximate mass calculation for density checks.
    pub fn mass(&self) -> f64 {
        // Placeholder: Real implementation would use a lookup table.
        // Used primarily for heuristic density checks.
        self.atoms.len() as f64 * 10.0
    }
}

// ============================================================================
// 3. ENGINE DEFINITIONS (The Hexagonal Ports)
// ============================================================================

/// Defines WHICH scientific engine will execute the workload.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "engine_type", content = "spec")]
pub enum Engine {
    /// Machine Learning Interatomic Potentials (Janus-Core).
    /// Runs via the Persistent Daemon.
    #[serde(rename = "janus")]
    Janus {
        arch: String,                      // e.g., "mace_mp", "chgnet"
        device_preference: Option<String>, // "cuda", "cpu", "mps"
        model_path: Option<PathBuf>,       // Optional local override
    },

    /// Classical Forcefields (GULP).
    /// Runs via Clean-Slate External Process.
    #[serde(rename = "gulp")]
    Gulp {
        binary: String,            // "gulp" or "/path/to/gulp"
        potential_library: String, // e.g., "reaxff", "buckingham"
    },

    /// DFT Codes (VASP).
    /// Runs via Clean-Slate MPI.
    #[serde(rename = "vasp")]
    Vasp {
        binary: String,   // "vasp_std"
        mpi_ranks: usize, // Specific rank request
    },

    /// DFT Codes (CP2K).
    #[serde(rename = "cp2k")]
    Cp2k {
        binary: String, // "cp2k.popt"
        mpi_ranks: usize,
    },

    /// Python Active Learning Agent.
    /// Runs via Shell/Uv/Conda.
    #[serde(rename = "agent")]
    Agent {
        script_path: String,
        strategy: String, // "autoemulate", "bayesian_opt"
    },
}

impl Default for Engine {
    fn default() -> Self {
        Engine::Agent {
            script_path: "agent.py".into(),
            strategy: "default".into(),
        }
    }
}

// ============================================================================
// 4. JOB CONFIGURATION (The Blueprint)
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobConfig {
    /// The engine that drives this job.
    pub engine: Engine,

    /// The "Blueprint": Engine-specific parameters.
    /// VASP -> INCAR tags.
    /// GULP -> Keywords.
    /// Janus -> Inference settings.
    pub params: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceReq {
    pub nodes: usize,
    pub cores: usize,
    pub gpus: usize,
    pub time_limit_min: usize,
    #[serde(default)]
    pub required_tags: Vec<String>,
}

impl Default for ResourceReq {
    fn default() -> Self {
        Self {
            nodes: 1,
            cores: 1,
            gpus: 0,
            time_limit_min: 60,
            required_tags: vec![],
        }
    }
}

// ============================================================================
// 5. PROVENANCE & RESULTS (The Vault)
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Provenance {
    pub execution_host: String, // Hostname
    pub start_time: DateTime<Utc>,
    pub end_time: DateTime<Utc>,
    pub binary_hash: Option<String>, // SHA256 of executable or model weights
    pub exit_code: i32,
    pub sandbox_info: String, // e.g., "Rank 0, Cores 0-7, GPU 0"
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CalculationResult {
    // Scientific Data (Strongly Typed)
    pub energy: Option<ElectronVolts>,
    pub forces: Option<Vec<[Force; 3]>>,
    pub stress: Option<[[f64; 3]; 3]>,

    // Performance Data
    pub t_total_ms: f64,

    // Outcome
    pub final_structure: Option<Structure>,

    // Trust
    pub provenance: Provenance,

    // Active Learning Specifics
    pub next_generation: Option<Vec<Value>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobSummary {
    pub id: String,
    pub status: String,
    pub code: String,
    pub node_id: String,
    pub updated_at: i64,
    pub t_total: f64,
}

// ============================================================================
// 6. JOB STATE (The Lifecycle)
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum JobStatus {
    Pending,
    Blocked, // Waiting on parents
    Queued,  // In local Guardian queue (Accepted but waiting for cores)
    Running, // Assigned to cores and executing
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Job {
    #[serde(default = "Uuid::new_v4")]
    pub id: Uuid,
    pub status: JobStatus,

    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,

    // Inputs
    pub structure: Structure,
    pub config: JobConfig,
    pub resources: ResourceReq,

    // Outputs
    pub result: Option<CalculationResult>,
    pub error_log: Option<String>,

    // Topology
    #[serde(default)]
    pub parent_ids: Vec<Uuid>,
    pub node_id: Option<String>, // Who ran me?

    // Workflow Metadata (DAG logic)
    #[serde(default)]
    pub flow_context: HashMap<String, Value>,
}

impl Job {
    pub fn new(structure: Structure, config: JobConfig, resources: ResourceReq) -> Self {
        Self {
            id: Uuid::new_v4(),
            status: JobStatus::Pending,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            structure,
            config,
            resources,
            result: None,
            error_log: None,
            parent_ids: Vec::new(),
            node_id: None,
            flow_context: HashMap::new(),
        }
    }
}
