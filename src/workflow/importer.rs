// src/workflow/importer.rs
//
// =============================================================================
// UNIFIEDLAB: SCENARIO GENERATOR (v7.0 - ULTIMATE TEST SUITE)
// =============================================================================
//
// Generates complex DAG topologies programmatically to stress-test:
// 1. Dependency Resolution (Chains).
// 2. Concurrency/Throttling (Fan-Out).
// 3. Driver Isolation (Mixed Engines).
// 4. Resource Bin-Packing (Variable Core/GPU reqs).
//  TODO
//  The graph functionality is not properly used, still on decision progress in how to best implement the DSL structure

use crate::core::{Atom, Engine, Job, JobConfig, Lattice, ResourceReq, Structure};
use crate::workflow::{NodeType, WorkflowEngine};
use anyhow::{anyhow, Result};
use petgraph::graph::NodeIndex;
use std::collections::HashMap;

pub struct DrawIoLoader {
    pub graph: WorkflowEngine,
}

impl DrawIoLoader {
    pub fn load_from_file(scenario_sig: &str) -> Result<Self> {
        let mut engine = WorkflowEngine::new();
        let sig = std::path::Path::new(scenario_sig)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(scenario_sig);

        log::info!("🧪 Generating Scenario: '{}'", sig);

        // PARSE SCENARIO SIGNATURE
        let parts: Vec<&str> = sig.split('_').collect();
        match parts[0] {
            // "chain_10_janus" -> A -> B -> C ...
            "chain" => {
                let depth = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(5);
                let mode = parts.get(2).unwrap_or(&"janus");
                build_linear_chain(&mut engine, depth, mode)?;
            }

            // "fanout_100_gulp" -> 1 Gen -> 100 Workers -> 1 Aggregator
            "fanout" => {
                let width = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(20);
                let mode = parts.get(2).unwrap_or(&"janus");
                build_fan_out(&mut engine, width, mode)?;
            }

            // "mixed_50" -> Random mix of VASP, GULP, Janus, Agents
            "mixed" => {
                let count = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(20);
                build_mixed_chaos(&mut engine, count)?;
            }

            // "gpu_starve" -> Requests more GPUs than exist
            "starve" => {
                build_resource_starvation(&mut engine)?;
            }

            _ => {
                // Default: Single Job
                let job = make_job("Single_Test", get_engine("janus"), 1, 0);
                engine.add_smart_node(job, NodeType::Compute, vec![], 50, true)?;
            }
        }

        Ok(Self { graph: engine })
    }
}

// ============================================================================
// SCENARIO BUILDERS
// ============================================================================

fn build_linear_chain(engine: &mut WorkflowEngine, depth: usize, mode: &str) -> Result<()> {
    let mut prev_idx = None;

    for i in 0..depth {
        let name = format!("{}_Step_{}", mode, i);
        let job = make_job(&name, get_engine(mode), 1, 0);

        // Link to previous (Hard Dependency)
        let parents = if let Some(p) = prev_idx {
            vec![p]
        } else {
            vec![]
        };

        let idx = engine.add_smart_node(job, NodeType::Compute, parents, 50, true)?;
        prev_idx = Some(idx);
    }
    log::info!("🔗 Built Linear Chain (Depth: {})", depth);
    Ok(())
}

fn build_fan_out(engine: &mut WorkflowEngine, width: usize, mode: &str) -> Result<()> {
    // 1. Root Node (Generator)
    let root_job = make_job("Root_Generator", get_engine("agent"), 1, 0);
    let root_idx = engine.add_smart_node(
        root_job,
        NodeType::Generator {
            strategy: "seed".into(),
        },
        vec![],
        100,
        true,
    )?;

    // 2. Workers (The Bag of Tasks)
    let mut workers = Vec::new();
    for i in 0..width {
        let name = format!("Worker_{}_{}", mode, i);

        // Variance: Every 5th job requests a GPU (if janus)
        let gpus = if mode == "janus" && i % 5 == 0 { 1 } else { 0 };

        let job = make_job(&name, get_engine(mode), 1, gpus);
        let idx = engine.add_smart_node(job, NodeType::Compute, vec![root_idx], 50, true)?;
        workers.push(idx);
    }

    // 3. Aggregator (The Collector)
    let agg_job = make_job("Final_Aggregator", get_engine("agent"), 1, 0);
    engine.add_smart_node(agg_job, NodeType::Aggregator, workers, 10, true)?;

    log::info!("🪭 Built Fan-Out (Width: {})", width);
    Ok(())
}

fn build_mixed_chaos(engine: &mut WorkflowEngine, count: usize) -> Result<()> {
    // A stress test for Driver Dispatching and Resource Isolation
    for i in 0..count {
        let (mode, cores, gpus) = match i % 4 {
            0 => ("janus", 1, 1), // GPU intensive
            1 => ("gulp", 4, 0),  // Multi-core CPU
            2 => ("vasp", 2, 0),  // MPI
            _ => ("agent", 1, 0), // Lightweight
        };

        let name = format!("Chaos_{}_{}", mode, i);
        let job = make_job(&name, get_engine(mode), cores, gpus);

        // No dependencies, just pure scheduling load
        engine.add_smart_node(job, NodeType::Compute, vec![], 50, true)?;
    }
    log::info!("🌪️ Built Mixed Chaos (Count: {})", count);
    Ok(())
}

fn build_resource_starvation(engine: &mut WorkflowEngine) -> Result<()> {
    // Job 1: Hog (Needs 100 GPUs) - Should Block
    let hog = make_job("Black_Hole", get_engine("janus"), 1, 100);
    engine.add_smart_node(hog, NodeType::Compute, vec![], 100, true)?;

    // Job 2: Tiny (Needs 1 Core) - Should Run (if Scheduler is good)
    let mouse = make_job("Mouse", get_engine("agent"), 1, 0);
    engine.add_smart_node(mouse, NodeType::Compute, vec![], 50, true)?;

    Ok(())
}

// ============================================================================
// HELPERS
// ============================================================================

fn get_engine(mode: &str) -> Engine {
    match mode {
        "janus" => Engine::Janus {
            arch: "lennard_jones".into(),
            device_preference: Some("mps".into()),
            model_path: None,
        },
        "gulp" => Engine::Gulp {
            binary: "./mock_gulp".into(),
            potential_library: "reaxff".into(),
        },
        "vasp" => Engine::Vasp {
            binary: "./mock_vasp".into(),
            mpi_ranks: 2,
        },
        _ => Engine::Agent {
            script_path: "unifiedlab_drivers/agent_shim.py".into(),
            strategy: "test".into(),
        },
    }
}

fn make_job(name: &str, engine: Engine, cores: usize, gpus: usize) -> Job {
    // Standard Silicon Unit Cell
    let structure = Structure::new(
        vec![
            Atom {
                symbol: "Si".into(),
                position: [0.0, 0.0, 0.0],
                ..Default::default()
            },
            Atom {
                symbol: "Si".into(),
                position: [1.3, 1.3, 1.3],
                ..Default::default()
            },
        ],
        Some(Lattice {
            vectors: [[5.4, 0.0, 0.0], [0.0, 5.4, 0.0], [0.0, 0.0, 5.4]],
            pbc: [true; 3],
        }),
        name.into(),
    );

    Job::new(
        structure,
        JobConfig {
            engine,
            params: serde_json::json!({"test_id": name}),
        },
        ResourceReq {
            nodes: 1,
            cores,
            gpus,
            time_limit_min: 60,
            required_tags: vec![], // Tags handled by main.rs logic mostly
        },
    )
}

impl Default for Atom {
    fn default() -> Self {
        Self {
            symbol: "X".into(),
            position: [0.0, 0.0, 0.0],
            charge: None,
            magnetic_moment: None,
            tags: HashMap::new(),
        }
    }
}
