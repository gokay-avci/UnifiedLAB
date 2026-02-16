// src/eventlog.rs
//
// =============================================================================
// UNIFIEDLAB: EVENT LOGGING SUBSYSTEM (v 0.1)
// =============================================================================
//
// Responsibilities:
// - Immutable, Append-Only Storage.
// - Hybrid Serialization:
//   1. Container: Bincode (Fast, Compact, Type-Safe).
//   2. Payload: JSON stored as Vec<u8> (Flexible, Dynamic schema).
//
// Defensive Features:
// - Magic Headers: 0x554C4142 ("ULAB") anchors every record.
// - CRC32 Checksums: Detects bit-rot and partial writes.
// - Self-Healing: Reader scans byte-by-byte to recover from corruption.
// - Size Limits: Rejects records > 128MB to prevent OOM.
// - Path Access: Exposes file path for external metadata diagnostics.

use anyhow::{anyhow, Context, Result};
use crc32fast::Hasher;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs::{File, OpenOptions};
use std::io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

// -----------------------------------------------------------------------------
// CONSTANTS
// -----------------------------------------------------------------------------

// "ULAB" in ASCII / Big Endian
const MAGIC_BYTES: u32 = 0x554C4142;

// Hard limit to prevent memory exhaustion on corrupted length reads
const MAX_RECORD_SIZE: u32 = 128 * 1024 * 1024; // 128 MB

// -----------------------------------------------------------------------------
// DATA STRUCTURES
// -----------------------------------------------------------------------------

/// The high-level struct used by the application logic.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventRecord {
    pub ts_ms: i64,
    pub kind: String,
    pub payload: Value,
}

/// The low-level struct stored on disk.
/// We store the payload as raw JSON bytes to prevent Bincode from crashing
/// on dynamic `serde_json::Value` types.
#[derive(Serialize, Deserialize)]
struct DiskRecord {
    ts_ms: i64,
    kind: String,
    payload_json: Vec<u8>,
}

/// A wrapper returned to the reader containing position info.
#[derive(Debug, Clone)]
pub struct EventEnvelope {
    pub offset: u64,      // Absolute offset of the MAGIC bytes
    pub next_offset: u64, // Absolute offset of the start of the NEXT record
    pub record: EventRecord,
}

/// Configuration options for the writer.
#[derive(Debug, Clone)]
pub struct EventLogConfig {
    /// If true, calls `fsync` after every append.
    /// Recommended for Coordinators (Data Safety), optional for Workers (Speed).
    pub fsync: bool,
}

impl Default for EventLogConfig {
    fn default() -> Self {
        Self { fsync: false }
    }
}

// =============================================================================
// WRITER (Append-Only)
// =============================================================================

pub struct EventLogWriter {
    path: PathBuf,
    writer: BufWriter<File>,
    cfg: EventLogConfig,
}

impl EventLogWriter {
    /// Opens the log file in append mode. Creates directories if missing.
    pub fn open(path: impl AsRef<Path>, cfg: EventLogConfig) -> Result<Self> {
        let path = path.as_ref().to_path_buf();

        // Defensive: Ensure directory exists
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).ok();
        }

        // Open in Append mode.
        // Note: On HPC filesystems (Lustre/GPFS), O_APPEND is atomic for single-writer.
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .with_context(|| format!("Failed to open log writer: {:?}", path))?;

        Ok(Self {
            path,
            writer: BufWriter::new(file),
            cfg,
        })
    }

    /// Appends a new record to the log.
    /// Returns the offset where the record started.
    pub fn append(&mut self, kind: &str, payload: Value) -> Result<u64> {
        let ts_ms = chrono::Utc::now().timestamp_millis();

        // 1. Flatten JSON payload to bytes (Solves Bincode compatibility)
        let payload_bytes =
            serde_json::to_vec(&payload).context("Failed to serialize payload to JSON bytes")?;

        // 2. Create intermediate Disk Record
        let disk_rec = DiskRecord {
            ts_ms,
            kind: kind.to_string(),
            payload_json: payload_bytes,
        };

        // 3. Serialize Container to Binary (Bincode)
        let bytes = bincode::serialize(&disk_rec).context("Bincode serialization failed")?;

        let len = bytes.len() as u32;
        if len > MAX_RECORD_SIZE {
            return Err(anyhow!("Event exceeds 128MB limit: {} bytes", len));
        }

        // 4. Calculate Integrity Checksum (CRC32)
        let mut hasher = Hasher::new();
        hasher.update(&bytes);
        let crc = hasher.finalize();

        // 5. Write Frame: [MAGIC][CRC][LEN][DATA]
        let offset = self.writer.stream_position().unwrap_or(0);

        self.writer.write_all(&MAGIC_BYTES.to_le_bytes())?;
        self.writer.write_all(&crc.to_le_bytes())?;
        self.writer.write_all(&len.to_le_bytes())?;
        self.writer.write_all(&bytes)?;

        // 6. Flush to OS Cache
        self.writer.flush()?;

        // 7. Hardware Sync (Optional)
        if self.cfg.fsync {
            self.writer.get_ref().sync_data().ok();
        }

        Ok(offset)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

// =============================================================================
// READER (Tailing + Self-Healing)
// =============================================================================

pub struct EventLogReader {
    reader: BufReader<File>,
    cursor: u64,
    path: PathBuf,
}

impl EventLogReader {
    /// Opens a log file for reading.
    /// Defensive: Creates an empty file if it doesn't exist to prevent "File Not Found" errors.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();

        if !path.exists() {
            // Create empty file if missing so we can tail it immediately
            OpenOptions::new().create(true).write(true).open(path)?;
        }

        let file = OpenOptions::new()
            .read(true)
            .open(path)
            .with_context(|| format!("Failed to open log reader: {:?}", path))?;

        Ok(Self {
            reader: BufReader::new(file),
            cursor: 0,
            path: path.to_path_buf(),
        })
    }

    /// Moves the read head to a specific absolute offset.
    pub fn seek(&mut self, offset: u64) -> Result<()> {
        self.reader.seek(SeekFrom::Start(offset))?;
        self.cursor = offset;
        Ok(())
    }

    /// Accessor for the current read cursor position.
    pub fn cursor(&self) -> u64 {
        self.cursor
    }

    /// Accessor for the file path (Diagnostic Feature).
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Tries to read the next record.
    /// Returns:
    /// - `Ok(Some(Envelope))`: Valid record found.
    /// - `Ok(None)`: Reached End-Of-File (EOF).
    /// - `Ok(None)` (via Resync): Corruption found, skipped, but hit EOF before finding next valid record.
    pub fn next(&mut self) -> Result<Option<EventEnvelope>> {
        loop {
            // A. Mark Start Position
            let start_pos = self.cursor;
            self.reader.seek(SeekFrom::Start(start_pos))?;

            // B. Read Magic (4 bytes)
            let mut magic_buf = [0u8; 4];
            match self.reader.read_exact(&mut magic_buf) {
                Ok(_) => {}
                // Clean EOF is not an error
                Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
                Err(e) => return Err(e.into()),
            }

            // C. Validate Magic Header
            let magic = u32::from_le_bytes(magic_buf);
            if magic != MAGIC_BYTES {
                log::warn!(
                    "Corruption at offset {} in {:?}. Magic: {:x}. Scanning...",
                    start_pos,
                    self.path,
                    magic
                );
                // Self-Healing: Scan forward to find next valid record
                if let Some(new_offset) = self.scan_for_magic(start_pos + 1)? {
                    self.cursor = new_offset;
                    continue; // Retry read at new location
                } else {
                    return Ok(None); // Hit EOF while scanning
                }
            }

            // D. Read Metadata (CRC + Len = 8 bytes)
            let mut meta_buf = [0u8; 8];
            if self.reader.read_exact(&mut meta_buf).is_err() {
                return Ok(None); // Partial write at EOF
            }
            let expected_crc = u32::from_le_bytes(meta_buf[0..4].try_into()?);
            let len = u32::from_le_bytes(meta_buf[4..8].try_into()?);

            // E. Sanity Check Length
            if len > MAX_RECORD_SIZE {
                log::error!(
                    "Implausible record length {} at {}. Header corrupt.",
                    len,
                    start_pos
                );
                if let Some(new_offset) = self.scan_for_magic(start_pos + 1)? {
                    self.cursor = new_offset;
                    continue;
                } else {
                    return Ok(None);
                }
            }

            // F. Read Payload
            let mut payload = vec![0u8; len as usize];
            if self.reader.read_exact(&mut payload).is_err() {
                return Ok(None); // Partial payload write
            }

            // G. Validate Integrity (CRC32)
            let mut hasher = Hasher::new();
            hasher.update(&payload);
            if hasher.finalize() != expected_crc {
                log::error!("CRC Mismatch at {}. Data corrupted.", start_pos);
                if let Some(new_offset) = self.scan_for_magic(start_pos + 1)? {
                    self.cursor = new_offset;
                    continue;
                } else {
                    return Ok(None);
                }
            }

            // H. Deserialize Container (Bincode)
            let disk_rec: DiskRecord = match bincode::deserialize(&payload) {
                Ok(r) => r,
                Err(e) => {
                    log::error!("Bincode Error at {}: {}. Skipping.", start_pos, e);
                    self.cursor = start_pos + 12 + len as u64;
                    continue;
                }
            };

            // I. Inflate Payload (JSON Bytes -> Value)
            // Safe because we produced it in `append` via serde_json::to_vec
            let val: Value = match serde_json::from_slice(&disk_rec.payload_json) {
                Ok(v) => v,
                Err(e) => {
                    log::error!("Inner JSON Corrupt at {}: {}. Skipping.", start_pos, e);
                    self.cursor = start_pos + 12 + len as u64;
                    continue;
                }
            };

            let record = EventRecord {
                ts_ms: disk_rec.ts_ms,
                kind: disk_rec.kind,
                payload: val,
            };

            // Success: Update cursor to end of this record
            let next_offset = start_pos + 12 + len as u64;
            self.cursor = next_offset;

            return Ok(Some(EventEnvelope {
                offset: start_pos,
                next_offset,
                record,
            }));
        }
    }

    /// Brute-force scan: Moves forward 1 byte at a time looking for `0x554C4142`.
    /// Essential for recovering from partial writes during power loss/crash.
    fn scan_for_magic(&mut self, start_scan: u64) -> Result<Option<u64>> {
        self.reader.seek(SeekFrom::Start(start_scan))?;

        let mut byte = [0u8; 1];
        let mut buffer = [0u8; 4]; // Rolling window
        let mut valid_bytes = 0;
        let mut current_pos = start_scan;

        // Prime the window
        while valid_bytes < 4 {
            if self.reader.read(&mut byte)? == 0 {
                return Ok(None);
            }
            buffer[valid_bytes] = byte[0];
            valid_bytes += 1;
            current_pos += 1;
        }

        loop {
            if u32::from_le_bytes(buffer) == MAGIC_BYTES {
                // Found it! The magic started 4 bytes ago.
                return Ok(Some(current_pos - 4));
            }

            // Slide window
            if self.reader.read(&mut byte)? == 0 {
                return Ok(None); // EOF
            }

            // Shift left
            buffer[0] = buffer[1];
            buffer[1] = buffer[2];
            buffer[2] = buffer[3];
            buffer[3] = byte[0];
            current_pos += 1;
        }
    }
}
