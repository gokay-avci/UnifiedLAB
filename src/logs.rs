// src/logs.rs
//
// =============================================================================
// UNIFIEDLAB: MEMORY LOGGER (v 0.1 )
// =============================================================================
//
// A thread-safe circular buffer that captures `log::info!` macros
// and stores them for display in the TUI (Dashboard).
//
// It decouples log generation (Drivers/Guardian) from log rendering (TUI).

use chrono::Local;
use log::{Level, LevelFilter, Metadata, Record, SetLoggerError};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

// ============================================================================
// 1. THE BUFFER (State)
// ============================================================================

#[derive(Clone)]
pub struct LogBuffer {
    // Protected by Mutex for concurrent writes (Logger) and reads (TUI)
    lines: Arc<Mutex<VecDeque<String>>>,
    capacity: usize,
}

impl LogBuffer {
    pub fn new(capacity: usize) -> Self {
        Self {
            lines: Arc::new(Mutex::new(VecDeque::with_capacity(capacity))),
            capacity,
        }
    }

    /// Adds a line to the buffer, dropping the oldest if full.
    pub fn push(&self, msg: String) {
        let mut lines = self.lines.lock().unwrap();
        if lines.len() >= self.capacity {
            lines.pop_front();
        }
        lines.push_back(msg);
    }

    /// Returns a snapshot of current logs for rendering.
    pub fn get_lines(&self) -> Vec<String> {
        self.lines.lock().unwrap().iter().cloned().collect()
    }
}

// ============================================================================
// 2. THE LOGGER (Integration)
// ============================================================================

pub struct TuiLogger {
    buffer: LogBuffer,
}

impl TuiLogger {
    /// Initializes the global logger.
    /// This captures standard `log::info!`, `log::warn!` calls.
    pub fn init(buffer: LogBuffer) -> Result<(), SetLoggerError> {
        let logger = Box::new(TuiLogger { buffer });
        // Leak the box to create a static reference required by the 'log' crate singleton
        log::set_logger(Box::leak(logger)).map(|()| log::set_max_level(LevelFilter::Info))
    }
}

impl log::Log for TuiLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        // Only capture Info and above (Warn, Error) to prevent noise
        metadata.level() <= Level::Info
    }

    fn log(&self, record: &Record) {
        if self.enabled(record.metadata()) {
            let timestamp = Local::now().format("%H:%M:%S");

            // Clean up target names (e.g. "unifiedlab::guardian" -> "guardian")
            let target_full = record.target();
            let target = target_full.split("::").last().unwrap_or(target_full);

            // Color/Format hints could be added here, but raw strings are safer for now
            self.buffer
                .push(format!("[{} {}] {}", timestamp, target, record.args()));
        }
    }

    fn flush(&self) {
        // No-op: Memory buffer flushes immediately
    }
}
