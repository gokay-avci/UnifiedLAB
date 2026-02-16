// src/checkpoint.rs
//
// =============================================================================
// UNIFIEDLAB: STATE STORE (v 0.1 )
// =============================================================================
//
// The Persistence Layer.
//
// Architecture:
// - SQLite using "Hybrid Relational" pattern.
// - High-traffic fields (status, timestamp) are columns.
// - Complex data (Structure, JobConfig, Provenance) is JSON text.
// - TUI-optimized queries using partial JSON deserialization.
// - HPC-safe journaling (DELETE mode).

use crate::core::{Engine, Job, JobConfig, JobSummary};
use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use uuid::Uuid;

// -----------------------------------------------------------------------------
// View Models (Used by TUI / Tools)
// -----------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerInfo {
    pub worker_id: String,
    pub cores: usize,
    pub tasks: usize,
    pub last_seen_ms: i64,
}

// -----------------------------------------------------------------------------
// CheckpointStore
// -----------------------------------------------------------------------------

pub struct CheckpointStore {
    path: PathBuf,
}

impl CheckpointStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let store = Self {
            path: path.as_ref().to_path_buf(),
        };
        store.init()?;
        Ok(store)
    }

    /// Initialize the schema if it doesn't exist.
    /// Sets strict timeout/journaling pragmas for HPC shared filesystems.
    fn init(&self) -> Result<()> {
        let conn = self.conn()?;

        // HPC Optimization:
        // - DELETE journal mode avoids WAL files (locking issues on Lustre/GPFS).
        // - synchronous=NORMAL is safe enough given we have an Event Log for recovery.
        // - Busy timeout handles contention from TUI readers.
        conn.execute_batch(
            "PRAGMA journal_mode=DELETE;
             PRAGMA synchronous=NORMAL;
             PRAGMA busy_timeout=10000;",
        )?;

        conn.execute_batch(
            "BEGIN;
            CREATE TABLE IF NOT EXISTS meta (
                key TEXT PRIMARY KEY,
                value TEXT
            );
            
            CREATE TABLE IF NOT EXISTS workers (
                id TEXT PRIMARY KEY,
                last_seen_ms INTEGER,
                state_json TEXT
            );

            CREATE TABLE IF NOT EXISTS jobs (
                id TEXT PRIMARY KEY,
                status TEXT,
                updated_at_ms INTEGER,
                node_id TEXT,
                full_json TEXT
            );
            
            -- Indices for TUI filtering / sorting
            CREATE INDEX IF NOT EXISTS idx_jobs_status ON jobs(status);
            CREATE INDEX IF NOT EXISTS idx_jobs_updated ON jobs(updated_at_ms);
            COMMIT;",
        )?;

        Ok(())
    }

    fn conn(&self) -> Result<Connection> {
        Connection::open(&self.path).context("Failed to open Checkpoint DB")
    }

    // -------------------------------------------------------------------------
    // WRITE API (Used by Coordinator / Guardian)
    // -------------------------------------------------------------------------

    pub fn save_cursor(&self, offset: u64) -> Result<()> {
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO meta (key, value) VALUES ('cursor', ?1)
             ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            params![offset.to_string()],
        )?;
        Ok(())
    }

    /// Batch Upsert.
    /// Updates job states and worker heartbeats in a single transaction.
    pub fn apply_batch(
        &self,
        cursor: u64,
        updated_jobs: &[&Job],
        workers: &[WorkerInfo],
    ) -> Result<()> {
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;

        // 1. Update Cursor (if non-zero)
        if cursor > 0 {
            tx.execute(
                "INSERT INTO meta (key, value) VALUES ('cursor', ?1)
                 ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                params![cursor.to_string()],
            )?;
        }

        // 2. Upsert Workers
        {
            let mut stmt = tx.prepare(
                "INSERT INTO workers (id, last_seen_ms, state_json) 
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT(id) DO UPDATE SET 
                    last_seen_ms=excluded.last_seen_ms,
                    state_json=excluded.state_json",
            )?;
            for w in workers {
                let json = serde_json::to_string(w)?;
                stmt.execute(params![w.worker_id, w.last_seen_ms, json])?;
            }
        }

        // 3. Upsert Jobs
        {
            let mut stmt = tx.prepare(
                "INSERT INTO jobs (id, status, updated_at_ms, node_id, full_json)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(id) DO UPDATE SET
                    status=excluded.status,
                    updated_at_ms=excluded.updated_at_ms,
                    node_id=excluded.node_id,
                    full_json=excluded.full_json",
            )?;

            for job in updated_jobs {
                let json = serde_json::to_string(job)?;
                let status_str = format!("{:?}", job.status);
                let updated_ms = job.updated_at.timestamp_millis();

                stmt.execute(params![
                    job.id.to_string(),
                    status_str,
                    updated_ms,
                    job.node_id, // Option<String> handles NULL automatically
                    json
                ])?;
            }
        }

        tx.commit()?;
        Ok(())
    }

    // -------------------------------------------------------------------------
    // READ API (Restoration)
    // -------------------------------------------------------------------------

    pub fn get_cursor(&self) -> Result<u64> {
        let conn = self.conn()?;
        let val: Option<String> = conn
            .query_row("SELECT value FROM meta WHERE key = 'cursor'", [], |r| {
                r.get(0)
            })
            .optional()?;

        match val {
            Some(s) => Ok(s.parse().unwrap_or(0)),
            None => Ok(0),
        }
    }

    /// Full restoration of all jobs.
    /// Used on Coordinator startup to rebuild the in-memory graph.
    pub fn restore_jobs(&self) -> Result<HashMap<Uuid, Job>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare("SELECT full_json FROM jobs")?;

        let rows = stmt.query_map([], |row| {
            let json: String = row.get(0)?;
            Ok(json)
        })?;

        let mut map = HashMap::new();
        for r in rows {
            let json = r?;
            // Defensive deserialization: If schema evolved, skip bad records
            if let Ok(job) = serde_json::from_str::<Job>(&json) {
                map.insert(job.id, job);
            } else {
                log::warn!("Failed to deserialize a job record during restore.");
            }
        }
        Ok(map)
    }

    // -------------------------------------------------------------------------
    // READ API (TUI Optimized)
    // -------------------------------------------------------------------------

    pub fn get_active_workers(&self) -> Result<Vec<WorkerInfo>> {
        let conn = self.conn()?;
        // Fetch workers seen in last 5 minutes (approx) to filter ghosts?
        // For now, fetch all, TUI can sort.
        let mut stmt = conn.prepare("SELECT state_json FROM workers ORDER BY last_seen_ms DESC")?;

        let rows = stmt.query_map([], |row| {
            let json: String = row.get(0)?;
            Ok(json)
        })?;

        let mut out = Vec::new();
        for r in rows {
            if let Ok(w) = serde_json::from_str::<WorkerInfo>(&r?) {
                out.push(w);
            }
        }
        Ok(out)
    }

    /// Fast summary fetch for TUI.
    /// Manually extracts Engine type string from the JSON blob.
    /// CRITICAL: Does NOT deserialize the 'structure' field (heavy atoms).
    pub fn get_jobs_summary(&self) -> Result<Vec<JobSummary>> {
        let conn = self.conn()?;

        let mut stmt = conn.prepare(
            "SELECT id, status, node_id, updated_at_ms, full_json 
             FROM jobs 
             ORDER BY updated_at_ms DESC 
             LIMIT 1000",
        )?;

        // Lightweight struct to peek inside the full JSON without full deserialization
        #[derive(Deserialize)]
        struct PartialJob {
            config: PartialConfig,
            result: Option<PartialResult>,
        }
        #[derive(Deserialize)]
        struct PartialConfig {
            engine: Engine,
        }
        #[derive(Deserialize)]
        struct PartialResult {
            t_total_ms: f64,
        }

        let iter = stmt.query_map([], |row| {
            let id: String = row.get(0)?;
            let status: String = row.get(1)?;
            let node_id: Option<String> = row.get(2)?;
            let updated_at: i64 = row.get(3)?;
            let json: String = row.get(4)?;

            // Extract display code (e.g., "janus:mace_mp" or "vasp")
            // Default to "?" if parsing fails
            let (code, t_total) = match serde_json::from_str::<PartialJob>(&json) {
                Ok(p) => {
                    let code_str = match p.config.engine {
                        Engine::Janus { arch, .. } => format!("janus:{}", arch),
                        Engine::Gulp { .. } => "gulp".to_string(),
                        Engine::Vasp { mpi_ranks, .. } => format!("vasp:{}p", mpi_ranks),
                        Engine::Cp2k { mpi_ranks, .. } => format!("cp2k:{}p", mpi_ranks),
                        Engine::Agent { strategy, .. } => format!("agent:{}", strategy),
                    };
                    let time = p.result.map(|r| r.t_total_ms).unwrap_or(0.0);
                    (code_str, time)
                }
                Err(_) => ("?".to_string(), 0.0),
            };

            Ok(JobSummary {
                id,
                status,
                code,
                node_id: node_id.unwrap_or_default(),
                updated_at,
                t_total,
            })
        })?;

        let mut out = Vec::new();
        for i in iter {
            if let Ok(s) = i {
                out.push(s);
            }
        }
        Ok(out)
    }

    /// Fetch full details for the Inspector panel.
    pub fn get_job_details(&self, id: &str) -> Result<Job> {
        let conn = self.conn()?;
        let json: String = conn.query_row(
            "SELECT full_json FROM jobs WHERE id = ?1",
            params![id],
            |r| r.get(0),
        )?;
        let job: Job = serde_json::from_str(&json)?;
        Ok(job)
    }
}
