// src/workflow/importer.rs
//
// =============================================================================
// UNIFIEDLAB: SCENARIO GENERATOR (v7.0 - ULTIMATE TEST SUITE)
// =============================================================================

use crate::core::{Atom, Engine, Job, JobConfig, Lattice, ResourceReq, Structure};
use crate::workflow::{NodeType, WorkflowEngine};
use anyhow::{anyhow, Result};
use base64::{engine::general_purpose, Engine as _};
use flate2::read::DeflateDecoder;
use petgraph::graph::NodeIndex;
use quick_xml::events::Event;
use quick_xml::reader::Reader;
use std::collections::HashMap;
use std::fs;
use std::io::Read;

// Internal parsing structures
struct ParsedNode {
    #[allow(dead_code)]
    id: String,
    label: String,
    #[allow(dead_code)]
    shape: String,
}

struct ParsedEdge {
    source: String,
    target: String,
}

pub struct DrawIoLoader {
    pub graph: WorkflowEngine,
}

impl DrawIoLoader {
    pub fn load_from_file(path_or_sig: &str) -> Result<Self> {
        // 1. Try to read as actual file
        if let Ok(content) = fs::read_to_string(path_or_sig) {
            // Check for uncompressed XML or compressed XML (both start with <mxfile usually)
            if content.trim().starts_with("<mxfile") {
                return Self::parse_xml(&content);
            }
        }

        // 2. Fallback: Scenario Generator (Legacy/Testing)
        let mut engine = WorkflowEngine::new();
        let sig = std::path::Path::new(path_or_sig)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(path_or_sig);

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

    fn parse_xml(content: &str) -> Result<Self> {
        let mut engine = WorkflowEngine::new();
        let mut nodes: HashMap<String, ParsedNode> = HashMap::new();
        let mut edges: Vec<ParsedEdge> = Vec::new();

        // Parse the provided content
        Self::parse_graph_content(content, &mut nodes, &mut edges)?;

        let mut node_indices: HashMap<String, NodeIndex> = HashMap::new();

        // Add Nodes to Engine
        for (id, node) in &nodes {
            let job_name = if node.label.is_empty() {
                format!("Job_{}", id)
            } else {
                node.label.clone()
            };
            // Infer engine from label or default
            let engine_type = if job_name.to_lowercase().contains("janus") {
                get_engine("janus")
            } else {
                get_engine("agent") // Default
            };

            let job = make_job(&job_name, engine_type, 1, 0);
            let idx = engine.add_smart_node(job, NodeType::Compute, vec![], 50, true)?;
            node_indices.insert(id.clone(), idx);
        }

        // Add Edges
        for edge in &edges {
            if let (Some(&src), Some(&dst)) = (
                node_indices.get(&edge.source),
                node_indices.get(&edge.target),
            ) {
                engine
                    .graph
                    .add_edge(src, dst, crate::workflow::EdgeType::HardDependency);
            }
        }

        log::info!(
            "📂 Parsed Draw.io XML: {} nodes, {} edges",
            nodes.len(),
            edges.len()
        );
        Ok(Self { graph: engine })
    }

    fn parse_graph_content(
        content: &str,
        nodes: &mut HashMap<String, ParsedNode>,
        edges: &mut Vec<ParsedEdge>,
    ) -> Result<()> {
        let mut reader = Reader::from_str(content);
        reader.trim_text(true);
        let mut buf = Vec::new();
        let mut in_diagram = false;

        loop {
            match reader.read_event_into(&mut buf) {
                Ok(Event::Start(e)) => {
                    let name = e.name();
                    if name.as_ref() == b"diagram" {
                        in_diagram = true;
                    } else if name.as_ref() == b"mxCell" {
                        Self::parse_cell_attributes(e.attributes(), nodes, edges)?;
                    }
                }
                Ok(Event::Empty(e)) => {
                    if e.name().as_ref() == b"mxCell" {
                        Self::parse_cell_attributes(e.attributes(), nodes, edges)?;
                    }
                }
                Ok(Event::Text(e)) => {
                    if in_diagram {
                        let text = e.unescape()?;
                        if !text.trim().is_empty() {
                            // Try to decode compressed diagram data
                            if let Ok(decoded_xml) = Self::decode_diagram_data(&text) {
                                // Recursively parse the decoded XML
                                Self::parse_graph_content(&decoded_xml, nodes, edges)?;
                            }
                        }
                    }
                }
                Ok(Event::End(e)) => {
                    if e.name().as_ref() == b"diagram" {
                        in_diagram = false;
                    }
                }
                Ok(Event::Eof) => break,
                Err(e) => return Err(anyhow!("XML Error: {}", e)),
                _ => (),
            }
            buf.clear();
        }
        Ok(())
    }

    fn parse_cell_attributes(
        attributes: quick_xml::events::attributes::Attributes,
        nodes: &mut HashMap<String, ParsedNode>,
        edges: &mut Vec<ParsedEdge>,
    ) -> Result<()> {
        let mut id = String::new();
        let mut value = String::new();
        let mut style = String::new();
        let mut vertex = false;
        let mut edge = false;
        let mut source = String::new();
        let mut target = String::new();

        for attr in attributes {
            let attr = attr?;
            match attr.key.as_ref() {
                b"id" => id = String::from_utf8_lossy(&attr.value).to_string(),
                b"value" => value = String::from_utf8_lossy(&attr.value).to_string(),
                b"style" => style = String::from_utf8_lossy(&attr.value).to_string(),
                b"vertex" => vertex = attr.value.as_ref() == b"1",
                b"edge" => edge = attr.value.as_ref() == b"1",
                b"source" => source = String::from_utf8_lossy(&attr.value).to_string(),
                b"target" => target = String::from_utf8_lossy(&attr.value).to_string(),
                _ => (),
            }
        }

        if vertex {
            nodes.insert(
                id.clone(),
                ParsedNode {
                    id,
                    label: value,
                    shape: style,
                },
            );
        } else if edge {
            if !source.is_empty() && !target.is_empty() {
                edges.push(ParsedEdge { source, target });
            }
        }
        Ok(())
    }

    fn decode_diagram_data(data: &str) -> Result<String> {
        // 1. Base64 Decode
        let compressed = general_purpose::STANDARD.decode(data.trim())?;

        // 2. Inflate (Raw Deflate)
        let mut decoder = DeflateDecoder::new(&compressed[..]);
        let mut s = String::new();
        decoder.read_to_string(&mut s)?;

        // 3. URL Decode
        // Draw.io often URL-encodes the XML before compression
        let decoded = urlencoding::decode(&s)?;

        Ok(decoded.into_owned())
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
