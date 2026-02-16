// src/drivers.rs
//
// =============================================================================
// UNIFIEDLAB: DRIVER MODULE & INTERFACE (v 0.1 )
// =============================================================================
//
// The Hexagonal Port.
//
// Responsibilities:
// 1. Define the `CodeDriver` trait (The Contract).
// 2. Dispatch `Engine` enums to concrete implementations.
// 3. Provide standardized utilities for process isolation (Sandboxing).

use crate::core::{CalculationResult, Engine, Job};
use crate::resources::Sandbox;
use anyhow::Result;
use async_trait::async_trait;
use std::path::Path;

// Declare the concrete implementations
pub mod external;
pub mod janus;

// ============================================================================
// 1. THE DRIVER TRAIT (The Contract)
// ============================================================================

#[async_trait]
pub trait CodeDriver: Send + Sync {
    /// The primary entry point.
    ///
    /// Arguments:
    /// - `job`: Contains the Structure (atoms) and JobConfig (params).
    /// - `sandbox`: Contains the assigned Cores/GPUs (Isolation).
    /// - `work_dir`: Where to write input/output files (Isolation).
    ///
    /// Returns:
    /// - `CalculationResult`: The standardized scientific output + provenance.
    async fn execute(
        &self,
        job: &Job,
        sandbox: &Sandbox,
        work_dir: &Path,
    ) -> Result<CalculationResult>;
}

// ============================================================================
// 2. THE DISPATCHER (The Factory)
// ============================================================================

pub struct DriverFactory;

impl DriverFactory {
    /// Returns a boxed driver capable of executing the requested engine.
    /// This is where the "Switch" happens between Daemon mode and Binary mode.
    pub fn get(engine: &Engine) -> Result<Box<dyn CodeDriver>> {
        match engine {
            // 1. Janus (Machine Learning Potentials)
            // Handled by the Persistent Daemon
            Engine::Janus {
                arch,
                device_preference,
                model_path,
            } => Ok(Box::new(janus::JanusDriver::new(
                arch.clone(),
                device_preference.clone(),
                model_path.clone(),
            ))),

            // 2. GULP (Classical Forcefields)
            // Handled by Clean-Slate Process
            Engine::Gulp {
                binary,
                potential_library,
            } => Ok(Box::new(external::ExternalDriver::new(
                external::ExternalKind::Gulp {
                    binary: binary.clone(),
                    library: potential_library.clone(),
                },
            ))),

            // 3. VASP (DFT)
            // Handled by Clean-Slate Process with MPI
            Engine::Vasp { binary, mpi_ranks } => Ok(Box::new(external::ExternalDriver::new(
                external::ExternalKind::Vasp {
                    binary: binary.clone(),
                    ranks: *mpi_ranks,
                },
            ))),

            // 4. CP2K (DFT)
            Engine::Cp2k { binary, mpi_ranks } => Ok(Box::new(external::ExternalDriver::new(
                external::ExternalKind::Cp2k {
                    binary: binary.clone(),
                    ranks: *mpi_ranks,
                },
            ))),

            // 5. Active Learning Agent
            // Handled as a Python script execution via shell/uv
            Engine::Agent {
                script_path,
                strategy,
            } => Ok(Box::new(external::ExternalDriver::new(
                external::ExternalKind::PythonScript {
                    path: script_path.clone(),
                    args: vec![format!("--strategy={}", strategy)],
                },
            ))),
        }
    }
}

// ============================================================================
// 3. HELPER: STANDARDIZED COMMAND EXECUTION
// ============================================================================

/// Helper for drivers to prepare commands with sandbox isolation.
/// This ensures consistent application of affinity/env vars across all drivers.
pub mod utils {
    use super::*;
    use tokio::process::Command; // FIXED: Using Tokio Command

    pub fn apply_sandbox(cmd: &mut Command, sandbox: &Sandbox) {
        // Delegate to the Sandbox logic defined in resources.rs
        sandbox.apply(cmd);
    }

    /// Helper to capture Stdout/Stderr and format errors nicely.
    /// Used by ExternalDriver.
    pub async fn wait_with_output_logging(
        child: tokio::process::Child,
        job_id: uuid::Uuid,
    ) -> Result<std::process::Output> {
        let output = child.wait_with_output().await?;

        if !output.status.success() {
            let _stdout = String::from_utf8_lossy(&output.stdout); // Prefixed with _ to silence unused warning
            let stderr = String::from_utf8_lossy(&output.stderr);

            // Log a snippet for visibility in the TUI
            log::error!(
                "Job {} Failed. Exit: {:?}\nSTDERR tail:\n{}",
                job_id,
                output.status.code(),
                stderr.lines().rev().take(10).collect::<Vec<_>>().join("\n")
            );

            // Return error so the Guardian marks job as Failed
            return Err(anyhow::anyhow!(
                "Process exited with error code {:?}. Stderr: {}",
                output.status.code(),
                stderr
            ));
        }

        Ok(output)
    }
}
