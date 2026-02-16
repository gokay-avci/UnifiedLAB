//! UnifiedLab Hybrid Workflow DSL (YAML ⇄ Draw.io)
//!
//! # Philosophy
//! This module defines the **canonical, human-readable** workflow description
//! used by UnifiedLab. The canonical form is **YAML** (VCS-friendly) and can be
//! round-tripped to a **Draw.io** diagram for visual authoring and monitoring.
//!
//! The design is guided by workflow reproducibility and reuse requirements
//! (typed interfaces, hierarchical composition, and explicit compute
//! environments).
//!
//! # Notes
//! - In this iteration we provide: YAML schema types, parsing, validation, and
//!   deterministic macro expansion.
//! - Draw.io conversion is added in a later iteration as a separate module to
//!   keep concerns clean and allow strict testing.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// DSL schema version supported by this implementation.
pub const SUPPORTED_DSL_VERSION: u32 = 1;

// =============================================================================
// Errors
// =============================================================================

/// A user-facing error for DSL loading/validation.
///
/// We keep these errors *human-readable* and *actionable* (what to fix, where).
#[derive(Debug)]
pub struct DslError {
    pub kind: DslErrorKind,
    pub context: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DslErrorKind {
    Io,
    Parse,
    Version,
    Validation,
}

impl DslError {
    pub fn io(err: impl fmt::Display, path: impl Into<String>) -> Self {
        let path = path.into();
        Self {
            kind: DslErrorKind::Io,
            context: vec![format!("I/O error while reading {path}: {err}")],
        }
    }

    pub fn parse(err: impl fmt::Display) -> Self {
        Self {
            kind: DslErrorKind::Parse,
            context: vec![format!("Failed to parse workflow YAML: {err}")],
        }
    }

    pub fn version(found: u32) -> Self {
        Self {
            kind: DslErrorKind::Version,
            context: vec![format!(
                "Unsupported DSL version: {found}. This UnifiedLab build supports version {SUPPORTED_DSL_VERSION}."
            )],
        }
    }

    pub fn validation(msg: impl Into<String>) -> Self {
        Self {
            kind: DslErrorKind::Validation,
            context: vec![msg.into()],
        }
    }

    pub fn push_context(mut self, msg: impl Into<String>) -> Self {
        self.context.push(msg.into());
        self
    }
}

impl fmt::Display for DslError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "{:?}", self.kind)?;
        for (i, line) in self.context.iter().enumerate() {
            if i == 0 {
                writeln!(f, "- {line}")?;
            } else {
                writeln!(f, "  {line}")?;
            }
        }
        Ok(())
    }
}

impl std::error::Error for DslError {}

// =============================================================================
// Core DSL Types
// =============================================================================

/// Top-level YAML document.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowSpec {
    pub version: u32,
    pub metadata: Metadata,
    #[serde(default)]
    pub environment: Option<EnvironmentSpec>,
    #[serde(default)]
    pub types: BTreeMap<String, TypeSpec>,
    pub nodes: Vec<NodeSpec>,
    #[serde(default)]
    pub edges: Vec<EdgeSpec>,
    #[serde(default)]
    pub macros: Vec<MacroSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Metadata {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub authors: Vec<Author>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Author {
    pub name: String,
    #[serde(default)]
    pub orcid: Option<String>,
}

/// Compute environment descriptor.
///
/// This is intentionally generic: UnifiedLab can interpret this as
/// Conda/uv, Docker, Apptainer, module-load stacks, etc.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EnvironmentSpec {
    /// A uv-managed project directory (contains pyproject.toml + uv.lock).
    UvProject { path: String },
    /// A Docker image reference.
    DockerImage { image: String },
    /// An Apptainer/Singularity image.
    ApptainerImage { image: String },
    /// A plain string descriptor for HPC module systems.
    Modules { modules: Vec<String> },
}

/// A declared type for ports.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TypeSpec {
    /// A file path (relative to work dir) or handle into artifact store.
    File,
    /// Scalar values.
    Float,
    Int,
    Bool,
    String,
    /// Domain primitives.
    Structure,
    Json,
    /// A list/array of another type.
    Array { of: Box<TypeSpec> },
}

/// A workflow node.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeSpec {
    pub id: String,
    #[serde(rename = "type")]
    pub node_type: NodeKind,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub engine: Option<EngineSpec>,
    #[serde(default)]
    pub params: serde_json::Value,
    #[serde(default)]
    pub resources: Option<ResourceSpec>,
    #[serde(default)]
    pub environment: Option<EnvironmentSpec>,
    #[serde(default)]
    pub inputs: Vec<PortSpec>,
    #[serde(default)]
    pub outputs: Vec<PortSpec>,
    #[serde(default)]
    pub cache: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NodeKind {
    Compute,
    Generator,
    Switch,
    Aggregator,
    Verifier,
    Sentinel,
    Subworkflow,
}

/// Engine configuration at DSL level.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EngineSpec {
    Janus,
    Gulp,
    Vasp,
    Cp2k,
    /// A Python agent shim.
    Agent {
        script: String,
        #[serde(default)]
        strategy: Option<String>,
    },
}

/// Resource requirements for a node.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceSpec {
    #[serde(default = "default_one")]
    pub nodes: u32,
    #[serde(default = "default_one")]
    pub cores: u32,
    #[serde(default)]
    pub gpus: u32,
    #[serde(default = "default_time_limit")]
    pub time_limit_min: u64,
    #[serde(default)]
    pub required_tags: Vec<String>,
}

fn default_one() -> u32 {
    1
}
fn default_time_limit() -> u64 {
    30
}

/// A typed port.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortSpec {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: PortTypeRef,
    /// Optional source reference (e.g. `relax.outputs.energy`).
    #[serde(default)]
    pub source: Option<String>,
}

/// References either an inline type or a named type.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PortTypeRef {
    Named(String),
    Inline(TypeSpec),
}

/// An edge (dependency) between nodes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EdgeSpec {
    pub from: String,
    pub to: String,
    #[serde(default)]
    pub kind: EdgeKind,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EdgeKind {
    #[serde(alias = "hard")]
    Hard,
    #[serde(alias = "soft")]
    Soft,
    /// A dataflow edge with optional parameter mappings.
    Dataflow {
        #[serde(default)]
        map: BTreeMap<String, String>,
    },
}

impl Default for EdgeKind {
    fn default() -> Self {
        EdgeKind::Hard
    }
}

// =============================================================================
// Macro system (graph expansion strategies)
// =============================================================================

/// High-level graph generator.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MacroSpec {
    pub id: String,
    #[serde(rename = "type")]
    pub macro_type: MacroKind,
    #[serde(default)]
    pub anchor: Option<String>,
    #[serde(default)]
    pub into: Option<String>,
    #[serde(default)]
    pub params: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MacroKind {
    /// Create a chain of N nodes.
    Chain,
    /// Create fanout of N parallel nodes from an anchor.
    Fanout,
}

/// Result of expanding macros into concrete nodes/edges.
#[derive(Debug, Clone)]
pub struct ExpandedWorkflow {
    pub spec: WorkflowSpec,
    /// Maps macro IDs to the node IDs they generated.
    pub macro_map: HashMap<String, Vec<String>>,
}

// =============================================================================
// Public API
// =============================================================================

/// Load and parse a YAML workflow file.
///
/// This does **not** perform macro expansion; call [`expand_macros`] after.
pub fn load_yaml(path: impl AsRef<Path>) -> Result<WorkflowSpec, DslError> {
    let path = path.as_ref();
    let raw =
        fs::read_to_string(path).map_err(|e| DslError::io(e, path.display().to_string()))?;

    let spec: WorkflowSpec = serde_yaml::from_str(&raw).map_err(DslError::parse)?;

    if spec.version != SUPPORTED_DSL_VERSION {
        return Err(DslError::version(spec.version));
    }

    validate(&spec).map_err(|e| e.push_context(format!("in file: {}", path.display())))?;
    Ok(spec)
}

/// Validate a workflow spec (IDs, references, types).
///
/// This is intentionally strict: we prefer failing fast with actionable errors
/// rather than letting malformed workflows reach the scheduler.
pub fn validate(spec: &WorkflowSpec) -> Result<(), DslError> {
    if spec.metadata.name.trim().is_empty() {
        return Err(DslError::validation("metadata.name must not be empty"));
    }
    if spec.nodes.is_empty() {
        return Err(DslError::validation("workflow must contain at least one node"));
    }

    // Node ID uniqueness.
    let mut ids = HashSet::new();
    for n in &spec.nodes {
        if n.id.trim().is_empty() {
            return Err(DslError::validation("node.id must not be empty"));
        }
        if !ids.insert(n.id.clone()) {
            return Err(DslError::validation(format!("duplicate node id: '{}'", n.id)));
        }
    }

    // Validate edges reference known nodes.
    for e in &spec.edges {
        if !ids.contains(&e.from) {
            return Err(DslError::validation(format!(
                "edge.from references unknown node: '{}'",
                e.from
            )));
        }
        if !ids.contains(&e.to) {
            return Err(DslError::validation(format!(
                "edge.to references unknown node: '{}'",
                e.to
            )));
        }
        if e.from == e.to {
            return Err(DslError::validation(format!(
                "self-edge is not allowed: '{}' -> '{}'",
                e.from, e.to
            )));
        }
    }

    // Validate port type refs: named types must exist.
    for n in &spec.nodes {
        for p in n.inputs.iter().chain(n.outputs.iter()) {
            if let PortTypeRef::Named(name) = &p.ty {
                if !spec.types.contains_key(name) {
                    return Err(DslError::validation(format!(
                        "node '{}' port '{}' references unknown type '{}'",
                        n.id, p.name, name
                    )));
                }
            }
        }
    }

    // Validate macro anchors.
    for m in &spec.macros {
        if m.id.trim().is_empty() {
            return Err(DslError::validation("macro.id must not be empty"));
        }
        if let Some(anchor) = &m.anchor {
            if !ids.contains(anchor) {
                return Err(DslError::validation(format!(
                    "macro '{}' references unknown anchor node '{}'",
                    m.id, anchor
                )));
            }
        }
    }

    Ok(())
}

/// Expand macros into concrete nodes/edges.
///
/// Macro expansion is deterministic and VCS-friendly: generated node IDs are stable.
pub fn expand_macros(spec: &WorkflowSpec) -> Result<ExpandedWorkflow, DslError> {
    let mut out = spec.clone();
    let mut macro_map: HashMap<String, Vec<String>> = HashMap::new();

    let mut existing: HashSet<String> = out.nodes.iter().map(|n| n.id.clone()).collect();

    for m in &spec.macros {
        match m.macro_type {
            MacroKind::Chain => {
                let len = m
                    .params
                    .get("length")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(1) as usize;

                let engine = m
                    .params
                    .get("engine")
                    .and_then(|v| v.as_str())
                    .unwrap_or("janus");

                let anchor = m.anchor.clone();

                let mut created = Vec::new();
                let mut prev = anchor;
                for i in 0..len {
                    let id = format!("{}_{}", m.id, i + 1);
                    if existing.contains(&id) {
                        return Err(DslError::validation(format!(
                            "macro '{}' would create duplicate node id '{}'",
                            m.id, id
                        )));
                    }
                    existing.insert(id.clone());

                    let node = NodeSpec {
                        id: id.clone(),
                        node_type: NodeKind::Compute,
                        title: Some(format!("{} step {}", m.id, i + 1)),
                        engine: Some(parse_engine(engine)),
                        params: serde_json::Value::Object(serde_json::Map::new()),
                        resources: None,
                        environment: None,
                        inputs: Vec::new(),
                        outputs: Vec::new(),
                        cache: None,
                    };
                    out.nodes.push(node);

                    if let Some(p) = prev.clone() {
                        out.edges.push(EdgeSpec {
                            from: p,
                            to: id.clone(),
                            kind: EdgeKind::Hard,
                        });
                    }
                    prev = Some(id.clone());
                    created.push(id);
                }
                macro_map.insert(m.id.clone(), created);
            }
            MacroKind::Fanout => {
                let width = m
                    .params
                    .get("width")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(1) as usize;
                let engine = m
                    .params
                    .get("engine")
                    .and_then(|v| v.as_str())
                    .unwrap_or("gulp");
                let anchor = m.anchor.clone().ok_or_else(|| {
                    DslError::validation(format!("macro '{}' fanout requires 'anchor'", m.id))
                })?;

                let mut created = Vec::new();
                for i in 0..width {
                    let id = format!("{}_{}", m.id, i + 1);
                    if existing.contains(&id) {
                        return Err(DslError::validation(format!(
                            "macro '{}' would create duplicate node id '{}'",
                            m.id, id
                        )));
                    }
                    existing.insert(id.clone());

                    let node = NodeSpec {
                        id: id.clone(),
                        node_type: NodeKind::Compute,
                        title: Some(format!("{} task {}", m.id, i + 1)),
                        engine: Some(parse_engine(engine)),
                        params: serde_json::Value::Object(serde_json::Map::new()),
                        resources: None,
                        environment: None,
                        inputs: Vec::new(),
                        outputs: Vec::new(),
                        cache: None,
                    };
                    out.nodes.push(node);

                    out.edges.push(EdgeSpec {
                        from: anchor.clone(),
                        to: id.clone(),
                        kind: EdgeKind::Hard,
                    });

                    created.push(id);
                }
                macro_map.insert(m.id.clone(), created);
            }
        }
    }

    validate(&out)?;
    Ok(ExpandedWorkflow { spec: out, macro_map })
}

fn parse_engine(s: &str) -> EngineSpec {
    match s.to_lowercase().as_str() {
        "janus" => EngineSpec::Janus,
        "gulp" => EngineSpec::Gulp,
        "vasp" => EngineSpec::Vasp,
        "cp2k" => EngineSpec::Cp2k,
        "agent" => EngineSpec::Agent {
            script: "unifiedlab_drivers/agent_shim.py".to_string(),
            strategy: None,
        },
        _ => EngineSpec::Janus,
    }
}

/// Emit YAML (canonical form).
pub fn to_yaml(spec: &WorkflowSpec) -> Result<String, DslError> {
    serde_yaml::to_string(spec).map_err(DslError::parse)
}

/// Resolve a path relative to the workflow file.
pub fn resolve_relative(workflow_file: &Path, referenced: &str) -> PathBuf {
    let p = PathBuf::from(referenced);
    if p.is_absolute() {
        p
    } else {
        workflow_file
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(p)
    }
}