// src/lib.rs
//
// =============================================================================
// UNIFIEDLAB: LIBRARY ROOT
// =============================================================================
//
// This file declares the module tree and exports public types.

// 1. Declare Modules
pub mod checkpoint;
pub mod core;
pub mod drivers;
pub mod eventlog;
pub mod guardian;
pub mod logs;
pub mod marketplace;
pub mod physics;
pub mod provenance;
pub mod resources;
pub mod transport;
pub mod tui;
pub mod workflow;

pub mod dsl;

// 2. Re-exports (The Public API)
// These allow `use crate::Job` or `use crate::LogBuffer` to work elsewhere.

pub use core::{Job, JobConfig, Structure};
pub use logs::LogBuffer;
pub use resources::ResourceLedger;
pub use workflow::{NodeType, WorkflowEngine};
