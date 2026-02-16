// src/drivers/janus.rs
//
// =============================================================================
// UNIFIEDLAB: JANUS-CORE DRIVER (v 0.1 )
// =============================================================================
//
// The Persistent Daemon.
//
// Responsibilities:
// 1. Maintain a long-running Python process (Kernel) to hold VRAM state.
// 2. Stream requests via Stdin/Stdout (JSON-RPC style).
// 3. Reboot the kernel if the assigned Sandbox changes (Context Switch).
// 4. Capture Stderr in real-time for debugging ("Glass Box").

use crate::core::{CalculationResult, ElectronVolts, Force, Job, Provenance, Structure};
use crate::drivers::CodeDriver;
use crate::physics::SanityCheck; // The Validator
use crate::provenance::{sha256_bytes, ModelNotary};
use crate::resources::Sandbox; // The Notary

use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::Mutex;

// ============================================================================
// 1. THE DRIVER STRUCT
// ============================================================================

pub struct JanusDriver {
    arch: String,
    device_preference: Option<String>,
    model_path: Option<PathBuf>,

    // The Persistent State
    // Protected by Async Mutex because we hold it across awaits (during execution)
    kernel: Mutex<Option<JanusKernel>>,
}

impl JanusDriver {
    pub fn new(arch: String, device: Option<String>, model_path: Option<PathBuf>) -> Self {
        Self {
            arch,
            device_preference: device,
            model_path,
            kernel: Mutex::new(None),
        }
    }
}

// ============================================================================
// 2. THE KERNEL (Running Process)
// ============================================================================

struct JanusKernel {
    process: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,

    // Context Tracking
    // We store the signature of the sandbox (e.g. "GPU-0") to detect if we need
    // to reboot the kernel when a new job arrives with different resource needs.
    sandbox_signature: String,
}

impl JanusKernel {
    /// Kills the process gracefully-ish.
    async fn kill(&mut self) {
        let _ = self.process.kill().await;
    }
}

// ============================================================================
// 3. IMPLEMENTATION
// ============================================================================

#[async_trait]
impl CodeDriver for JanusDriver {
    async fn execute(
        &self,
        job: &Job,
        sandbox: &Sandbox,
        _work_dir: &Path,
    ) -> Result<CalculationResult> {
        let t0 = Utc::now();

        // A. SEMANTIC VALIDATOR (The Gatekeeper)
        // Don't send garbage to the GPU.
        if let Err(e) = job.structure.validate_physics() {
            return Err(anyhow!("Physical Integrity Check Failed: {}", e));
        }

        // B. KERNEL MANAGEMENT (The Persistent Daemon)
        let mut kernel_guard = self.kernel.lock().await;

        // Generate signature: e.g. "GPUs[0]-Cores[0,1,2,3]"
        let sandbox_sig = format!("{:?}-{:?}", sandbox.gpus, sandbox.cores);

        // Check if we need to reboot (Dead kernel OR Sandbox mismatch)
        let needs_reboot = match &*kernel_guard {
            Some(k) => k.sandbox_signature != sandbox_sig,
            None => true,
        };

        if needs_reboot {
            if let Some(mut old_k) = kernel_guard.take() {
                log::info!(
                    "🔄 Rebooting Janus Kernel (Context Switch: {} -> {})",
                    old_k.sandbox_signature,
                    sandbox_sig
                );
                old_k.kill().await;
            }

            // Boot new kernel bound to THIS sandbox
            let new_k = self.boot_kernel(sandbox, &sandbox_sig).await?;
            *kernel_guard = Some(new_k);
        }

        let kernel = kernel_guard.as_mut().unwrap();

        // C. EXECUTION (The Stream)
        // 1. Serialize Request
        let req_json = serde_json::to_string(&JanusRequest {
            structure: job.structure.clone(),
            calc_mode: "single_point".into(),
        })?;

        // 2. Write to Stdin
        kernel
            .stdin
            .write_all(req_json.as_bytes())
            .await
            .context("Failed to write to daemon stdin")?;
        kernel.stdin.write_all(b"\n").await?;
        kernel.stdin.flush().await?;

        // 3. Read from Stdout
        let mut resp_line = String::new();
        let bytes_read = kernel
            .stdout
            .read_line(&mut resp_line)
            .await
            .context("Failed to read from daemon stdout")?;

        if bytes_read == 0 {
            // EOF = Daemon Crashed
            // Invalidate the kernel so next job reboots it
            let _ = kernel.process.kill().await;
            *kernel_guard = None;
            return Err(anyhow!(
                "Janus Daemon crashed unexpectedly (EOF on stdout). Check logs."
            ));
        }

        // 4. Parse Response
        let resp: JanusResponse = serde_json::from_str(&resp_line)
            .with_context(|| format!("Invalid JSON from daemon: '{}'", resp_line.trim()))?;

        if let Some(err) = resp.error {
            return Err(anyhow!("Janus Logic Error: {}", err));
        }

        // D. PROVENANCE (The Notary)
        // Validate Model Hash if local path provided
        let bin_hash = if let Some(p) = &self.model_path {
            ModelNotary::verify(p, None).ok()
        } else {
            None // Remote model (downloaded by Janus), hash unknown until we ask daemon (future feature)
        };

        Ok(CalculationResult {
            energy: resp.energy.map(ElectronVolts),
            forces: resp.forces.map(|vecs| {
                vecs.into_iter()
                    .map(|f| [Force(f[0]), Force(f[1]), Force(f[2])])
                    .collect()
            }),
            stress: resp.stress,
            t_total_ms: (Utc::now() - t0).num_milliseconds() as f64,
            final_structure: None, // Single point doesn't change structure
            provenance: Provenance {
                execution_host: hostname::get()?.to_string_lossy().to_string(),
                start_time: t0,
                end_time: Utc::now(),
                binary_hash: bin_hash,
                exit_code: 0,
                sandbox_info: sandbox_sig,
            },
            next_generation: None,
        })
    }
}

impl JanusDriver {
    async fn boot_kernel(&self, sandbox: &Sandbox, sig: &str) -> Result<JanusKernel> {
        // Expected location of the python driver
        let script_path = "unifiedlab_drivers/janus_daemon.py";

        // 1. Construct Command
        let mut cmd = Command::new("python");
        cmd.arg("-u"); // Unbuffered python stdout is CRITICAL for streaming
        cmd.arg(script_path);

        cmd.arg("--arch").arg(&self.arch);
        if let Some(d) = &self.device_preference {
            cmd.arg("--device").arg(d);
        }

        // 2. Apply Isolation (Env vars: CUDA_VISIBLE_DEVICES, etc.)
        // This is crucial: The Python process only sees the GPUs we give it.
        sandbox.apply(&mut cmd);

        // 3. Setup Pipes
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        // 4. Spawn
        let mut child = cmd.spawn().context("Failed to spawn Janus daemon")?;

        let stdin = child.stdin.take().expect("Failed to open stdin");

        // FIX: Take raw stdout first
        let raw_stdout = child.stdout.take().expect("Failed to open stdout");
        let stderr = child.stderr.take().expect("Failed to open stderr");

        // 5. Glass Box Logging (Stderr -> Rust Log)
        // This runs in the background for the lifetime of the process
        tokio::spawn(async move {
            let reader = BufReader::new(stderr);
            let mut lines = reader.lines();
            while let Ok(Some(line)) = lines.next_line().await {
                log::warn!("[JANUS-DAEMON] {}", line);
            }
        });

        // 6. Handshake (Wait for "READY")
        // This ensures the heavy model loading (PyTorch/MACE) is done before we consider it booted
        // FIX: Wrap raw_stdout in BufReader once
        let mut handshake_reader = BufReader::new(raw_stdout);
        let mut handshake = String::new();

        // Give it 60s to load the model (downloading takes time)
        match tokio::time::timeout(
            std::time::Duration::from_secs(60),
            handshake_reader.read_line(&mut handshake),
        )
        .await
        {
            Ok(Ok(n)) if n > 0 => {
                if !handshake.trim().contains("READY") {
                    let _ = child.kill().await;
                    return Err(anyhow!(
                        "Daemon boot failed. Expected 'READY', got: '{}'",
                        handshake.trim()
                    ));
                }
            }
            Ok(_) => return Err(anyhow!("Daemon closed stdout during boot")),
            Err(_) => {
                let _ = child.kill().await;
                return Err(anyhow!("Daemon timed out loading model (60s)"));
            }
        }

        Ok(JanusKernel {
            process: child,
            stdin,
            stdout: handshake_reader, // Pass ownership of the BufReader
            sandbox_signature: sig.to_string(),
        })
    }
}

// ============================================================================
// 4. PROTOCOL SCHEMA (Private)
// ============================================================================

#[derive(Serialize)]
struct JanusRequest {
    structure: Structure,
    calc_mode: String,
}

#[derive(Deserialize)]
struct JanusResponse {
    energy: Option<f64>,
    forces: Option<Vec<[f64; 3]>>,
    stress: Option<[[f64; 3]; 3]>,
    error: Option<String>,
}
