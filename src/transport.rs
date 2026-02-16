// src/transport.rs
//
// The Nervous System (Debug Edition).
//
// Changes:
// - Added file metadata checks to confirm data availability.
// - Added verbose trace logging for the read loop.

use crate::eventlog::{EventEnvelope, EventLogReader, EventLogWriter, EventLogConfig};
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tokio::fs;

#[async_trait]
pub trait Transport: Send + Sync {
    async fn send_to_coordinator(&mut self, kind: &str, payload: Value) -> Result<()>;
    async fn broadcast(&mut self, kind: &str, payload: Value) -> Result<u64>;
    async fn recv_broadcasts(&mut self) -> Result<Vec<EventEnvelope>>;
    async fn recv_worker_messages(&mut self) -> Result<Vec<EventEnvelope>>;
    async fn seek(&mut self, offset: u64) -> Result<()>;
}

pub struct FileTransport {
    role: Role,
    root_path: PathBuf,
    my_writer: EventLogWriter,
    global_reader: Option<EventLogReader>, 
    inbox_readers: HashMap<String, EventLogReader>,
    next_discovery: Instant,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Role { Coordinator, Worker }

impl FileTransport {
    pub async fn new(root_path: impl AsRef<Path>, role: Role, worker_id: Option<&str>) -> Result<Self> {
        let root = root_path.as_ref().to_path_buf();
        let inbox_dir = root.join("inbox");
        fs::create_dir_all(&inbox_dir).await?;

        let (writer, global_reader) = match role {
            Role::Coordinator => {
                let w = EventLogWriter::open(root.join("events.log"), EventLogConfig { fsync: true })?;
                (w, None)
            }
            Role::Worker => {
                let wid = worker_id.ok_or_else(|| anyhow!("Worker role requires worker_id"))?;
                let w = EventLogWriter::open(inbox_dir.join(format!("worker_{}.log", wid)), EventLogConfig { fsync: true })?;
                let r = EventLogReader::open(root.join("events.log"))?;
                (w, Some(r))
            }
        };

        Ok(Self {
            role, root_path: root, my_writer: writer, global_reader,
            inbox_readers: HashMap::new(), next_discovery: Instant::now(),
        })
    }
}

#[async_trait]
impl Transport for FileTransport {
    async fn send_to_coordinator(&mut self, kind: &str, payload: Value) -> Result<()> {
        if self.role == Role::Coordinator { return Err(anyhow!("Coordinator cannot send to self")); }
        self.my_writer.append(kind, payload)?;
        Ok(())
    }

    async fn broadcast(&mut self, kind: &str, payload: Value) -> Result<u64> {
        if self.role == Role::Worker { return Err(anyhow!("Worker cannot broadcast")); }
        Ok(self.my_writer.append(kind, payload)?)
    }

    async fn recv_broadcasts(&mut self) -> Result<Vec<EventEnvelope>> {
        if self.role == Role::Coordinator { return Ok(vec![]); }
        let reader = self.global_reader.as_mut().ok_or_else(|| anyhow!("No global reader"))?;
        let mut events = Vec::new();
        while let Ok(Some(env)) = reader.next() {
            events.push(env);
            if events.len() > 1000 { break; }
        }
        Ok(events)
    }

    async fn recv_worker_messages(&mut self) -> Result<Vec<EventEnvelope>> {
        if self.role == Role::Worker { return Ok(vec![]); }

        let mut events = Vec::new();
        
        // 1. Throttled Discovery
        if Instant::now() >= self.next_discovery {
            let inbox_dir = self.root_path.join("inbox");
            if let Ok(mut entries) = fs::read_dir(&inbox_dir).await {
                while let Ok(Some(entry)) = entries.next_entry().await {
                    let path = entry.path();
                    if path.extension().and_then(|s| s.to_str()) != Some("log") { continue; }
                    
                    if let Some(fname) = path.file_name().and_then(|s| s.to_str()) {
                        if !self.inbox_readers.contains_key(fname) {
                            log::info!("Discovered new worker inbox: {}", fname);
                            // FORCE metadata check on open
                            if let Ok(meta) = std::fs::metadata(&path) {
                                log::info!("Inbox {} size on disk: {} bytes", fname, meta.len());
                            }
                            if let Ok(r) = EventLogReader::open(&path) {
                                self.inbox_readers.insert(fname.to_string(), r);
                            }
                        }
                    }
                }
            }
            self.next_discovery = Instant::now() + Duration::from_secs(2);
        }

        // 2. Harvest
        for (wid, reader) in self.inbox_readers.iter_mut() {
            // DEBUG: Check if file has grown beyond our cursor
            // This is slightly expensive but necessary to debug "stuck" state
            // In production you might remove this check.
            /*
            if let Ok(meta) = std::fs::metadata(reader.path()) {
                if meta.len() > reader.cursor() {
                    log::debug!("Inbox {} has data! Size: {}, Cursor: {}", wid, meta.len(), reader.cursor());
                }
            }
            */

            let mut count = 0;
            loop {
                match reader.next() {
                    Ok(Some(env)) => {
                        log::info!("Read msg [{}] from {}", env.record.kind, wid); // LOG SUCCESS
                        events.push(env);
                        count += 1;
                        if count > 100 { break; } 
                    }
                    Ok(None) => {
                        // EOF - this is where it stops silently
                        break; 
                    }, 
                    Err(e) => {
                        log::warn!("Error reading inbox {}: {}", wid, e);
                        break;
                    }
                }
            }
        }

        Ok(events)
    }

    async fn seek(&mut self, offset: u64) -> Result<()> {
        if self.role == Role::Coordinator && self.global_reader.is_none() {
             let r = EventLogReader::open(self.root_path.join("events.log"))?;
             self.global_reader = Some(r);
        }
        if let Some(r) = &mut self.global_reader {
            r.seek(offset)?;
        }
        Ok(())
    }
}