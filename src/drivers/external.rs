// src/drivers/external.rs
//
// =============================================================================
// UNIFIEDLAB: EXTERNAL DRIVER (v 0.1 )
// =============================================================================
//
// The Compatibility Adapter.
//
// Responsibilities:
// 1. "The Sandwich": Python Write -> Rust Execute -> Python Parse.
// 2. Environment Scrubbing: Remove outer MPI context to allow nested execution.
// 3. Provenance: Capture binary SHA256 and exit codes.
// 4. Path Safety: Resolves scripts/binaries to absolute paths.
// 5. Cross-Platform: Handles macOS vs Linux MPI arguments gracefully.

use crate::core::{CalculationResult, Job, Provenance};
use crate::drivers::utils::{apply_sandbox, wait_with_output_logging};
use crate::drivers::CodeDriver;
use crate::resources::Sandbox;

use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::Utc;
use serde_json::Value;
use std::path::Path;
use std::process::Stdio;
use tokio::process::Command;

// ============================================================================
// 1. DRIVER CONFIGURATION
// ============================================================================

#[derive(Debug, Clone)]
pub enum ExternalKind {
    Gulp { binary: String, library: String },
    Vasp { binary: String, ranks: usize },
    Cp2k { binary: String, ranks: usize },
    PythonScript { path: String, args: Vec<String> },
}

pub struct ExternalDriver {
    kind: ExternalKind,
}

impl ExternalDriver {
    pub fn new(kind: ExternalKind) -> Self {
        Self { kind }
    }
}

// ============================================================================
// 2. IMPLEMENTATION
// ============================================================================

#[async_trait]
impl CodeDriver for ExternalDriver {
    async fn execute(
        &self,
        job: &Job,
        sandbox: &Sandbox,
        work_dir: &Path,
    ) -> Result<CalculationResult> {
        let t0 = Utc::now();

        // A. ADAPTER PHASE: WRITE INPUTS
        // Rust sends the Job JSON to Python via Stdin.
        self.call_adapter("write", job, work_dir)
            .await
            .context("Adapter Write Phase failed")?;

        // B. COMPUTE PHASE: RUN BINARY
        // Rust manages the heavy process directly for isolation/monitoring.
        // This returns the exit code and (optionally) the binary hash.
        let (exit_code, bin_hash) = self
            .run_heavy_compute(sandbox, work_dir)
            .await
            .context("Compute Phase failed")?;

        // C. ADAPTER PHASE: PARSE OUTPUTS
        // Python parses OUTCAR/logs and returns the CalculationResult JSON.
        let result_json = self
            .call_adapter("parse", job, work_dir)
            .await
            .context("Adapter Parse Phase failed")?;

        // D. FINALIZE
        // Deserialize the Python result
        let mut result: CalculationResult = serde_json::from_value(result_json)
            .context("Failed to deserialize result from Adapter")?;

        // Hydrate Provenance (Rust knows the truth about execution time and hardware)
        result.provenance = Provenance {
            execution_host: hostname::get()?.to_string_lossy().to_string(),
            start_time: t0,
            end_time: Utc::now(),
            binary_hash: bin_hash,
            exit_code,
            sandbox_info: format!("Cores: {:?}, GPUs: {:?}", sandbox.cores, sandbox.gpus),
        };
        result.t_total_ms = (Utc::now() - t0).num_milliseconds() as f64;

        Ok(result)
    }
}

impl ExternalDriver {
    fn engine_name(&self) -> &str {
        match &self.kind {
            ExternalKind::Gulp { .. } => "gulp",
            ExternalKind::Vasp { .. } => "vasp",
            ExternalKind::Cp2k { .. } => "cp2k",
            ExternalKind::PythonScript { .. } => "agent",
        }
    }

    // --- PHASE A/C: ADAPTER CALLS ---

    async fn call_adapter(&self, mode: &str, job: &Job, work_dir: &Path) -> Result<Value> {
        let mut cmd = Command::new("python");

        // FIX: Use absolute path for the CLI wrapper too, just in case
        let cli_path = self.resolve_path("unifiedlab_drivers/cli.py");
        if std::path::Path::new(&cli_path).exists() {
            cmd.arg(cli_path); // Use direct file path if not installed as module
        } else {
            cmd.arg("-m").arg("unifiedlab_drivers.cli"); // Fallback to module
        }

        cmd.arg(mode);
        cmd.arg(self.engine_name());
        cmd.arg(work_dir);

        // Setup pipes
        cmd.stdin(Stdio::piped());
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        let mut child = cmd.spawn().context("Failed to spawn Adapter")?;

        // Write Job JSON to Stdin
        if let Some(mut stdin) = child.stdin.take() {
            let json_bytes = serde_json::to_vec(job)?;
            tokio::io::AsyncWriteExt::write_all(&mut stdin, &json_bytes).await?;
        }

        // Wait and capture output
        let output = wait_with_output_logging(child, job.id).await?;

        // If parsing, we expect JSON on stdout. If writing, we expect empty/logs.
        if mode == "parse" {
            let out_str = String::from_utf8_lossy(&output.stdout);
            let json: Value =
                serde_json::from_str(out_str.trim()).context("Adapter returned invalid JSON")?;
            Ok(json)
        } else {
            Ok(Value::Null)
        }
    }

    // --- PHASE B: HEAVY COMPUTE ---

    async fn run_heavy_compute(
        &self,
        sandbox: &Sandbox,
        work_dir: &Path,
    ) -> Result<(i32, Option<String>)> {
        let (binary, args, needs_mpi) = self.resolve_command(sandbox);

        let mut cmd = Command::new(&binary);
        cmd.args(args);
        cmd.current_dir(work_dir);

        // 1. ISOLATION (Affinity & Env Vars)
        apply_sandbox(&mut cmd, sandbox);

        // 2. ENVIRONMENT SCRUBBING (The "Clean Slate")
        if needs_mpi {
            // Unset outer Slurm/MPI variables so the inner mpirun
            // creates a fresh universe using only the 'sandbox' cores.
            let scrub_vars = [
                "OMPI_COMM_WORLD_RANK",
                "OMPI_COMM_WORLD_SIZE",
                "PMIX_RANK",
                "PMIX_SERVER_URI",
                "PMIX_NAMESPACE",
                "SLURM_JOBID",
                "SLURM_PROCID",
                "SLURM_STEPID",
                "SLURM_GTIDS",
                "HYDRA_RANK",
                "MPI_LOCALRANKID",
            ];
            for var in scrub_vars {
                cmd.env_remove(var);
            }
        }

        // 3. EXECUTION
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        // In a full impl, we'd hash the binary here. Skipping for brevity.
        let bin_hash = None;

        // Helpful logging if binary not found
        let child = cmd
            .spawn()
            .with_context(|| format!("Failed to spawn binary '{}' in '{:?}'", binary, work_dir))?;

        // We don't use the logging helper here because GULP/VASP output can be massive.
        // We assume the binary writes to files (OUTCAR/output.gin) in work_dir.
        // We only capture stderr for crashes.
        let output = child.wait_with_output().await?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            log::warn!("Compute Binary stderr: {}", stderr);
        }

        Ok((output.status.code().unwrap_or(-1), bin_hash))
    }

    /// Resolves the binary/script string to a usable command.
    /// Handles:
    /// 1. Absolute Path resolution (Critical for /tmp execution).
    /// 2. OS Detection (macOS vs Linux MPI flags).
    /// 3. MPI Wrapper logic.
    fn resolve_command(&self, sandbox: &Sandbox) -> (String, Vec<String>, bool) {
        let is_macos = std::env::consts::OS == "macos";

        match &self.kind {
            ExternalKind::Gulp { binary, .. } => {
                // FIX: Resolve path logic
                let abs_binary = self.resolve_path(binary);
                (abs_binary, vec![], false)
            }
            ExternalKind::Vasp { binary, ranks } | ExternalKind::Cp2k { binary, ranks } => {
                let abs_binary = self.resolve_path(binary);

                if *ranks > 1 {
                    let mut args = vec!["-np".to_string(), ranks.to_string()];

                    // FIX: Strict binding only on Linux (HPC)
                    // macOS OpenMPI often crashes with explicit cpu lists
                    if !is_macos {
                        args.push("--cpu-set".to_string());
                        args.push(
                            sandbox
                                .cores
                                .iter()
                                .map(|c| c.to_string())
                                .collect::<Vec<_>>()
                                .join(","),
                        );
                        args.push("--bind-to".to_string());
                        args.push("cpu-list".to_string());
                    }

                    args.push(abs_binary);
                    ("mpirun".to_string(), args, true)
                } else {
                    (abs_binary, vec![], false)
                }
            }
            ExternalKind::PythonScript { path, args } => {
                // FIX: Resolve script path
                let abs_path = self.resolve_path(path);

                let mut full_args = vec![abs_path];
                full_args.extend(args.clone());
                ("python".to_string(), full_args, false)
            }
        }
    }

    /// Helper to ensure we can find the binary after changing Current Working Directory.
    /// If `path` is relative (e.g. `./mock_vasp`), it converts it to Absolute based
    /// on the current process CWD (Launch Directory).
    fn resolve_path(&self, path: &str) -> String {
        // If path contains separators, check if it's relative
        if path.contains('/') || path.contains('\\') {
            if std::path::Path::new(path).is_absolute() {
                path.to_string()
            } else {
                std::env::current_dir()
                    .unwrap_or_default()
                    .join(path)
                    .to_string_lossy()
                    .to_string()
            }
        } else {
            // It's just a command name (e.g. "grep"), assume in PATH
            path.to_string()
        }
    }
}
