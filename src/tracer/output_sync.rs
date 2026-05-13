//! Synchronized output for parallel command execution.
//!
//! Buffers lines per command name and flushes them in blocks with headers,
//! preventing chaotic interleaving. Blocks are flushed every FLUSH_INTERVAL_MS
//! or when a command completes.
//!
//! Sequential mode: lines pass through immediately (no buffering).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

const FLUSH_INTERVAL_MS: u64 = 200;

/// Shared output synchronizer for parallel commands.
#[derive(Clone)]
pub struct OutputSync {
    inner: Arc<Mutex<OutputSyncInner>>,
}

struct OutputSyncInner {
    /// Buffered lines per command name
    buffers: HashMap<String, Vec<String>>,
    /// Which command was last flushed (for header elision)
    last_flushed: Option<String>,
}

impl OutputSync {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(OutputSyncInner {
                buffers: HashMap::new(),
                last_flushed: None,
            })),
        }
    }

    /// Add a line from a specific command.
    pub fn push_line(&self, name: &str, line: &str) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.buffers.entry(name.to_string())
                .or_default()
                .push(line.to_string());
        }
    }

    /// Flush all buffered lines, grouped by command with headers.
    pub fn flush_all(&self) {
        if let Ok(mut inner) = self.inner.lock() {
            let names: Vec<String> = inner.buffers.keys().cloned().collect();
            for name in names {
                let last = inner.last_flushed.clone();
                if let Some(lines) = inner.buffers.get_mut(&name) {
                    if lines.is_empty() { continue; }

                    if last.as_deref() != Some(&name) {
                        eprintln!("    \x1b[2m── {} ──\x1b[0m", name);
                    }

                    for line in lines.drain(..) {
                        eprintln!("    {line}");
                    }

                    inner.last_flushed = Some(name);
                }
            }
        }
    }

    /// Flush lines for a specific command (on completion).
    pub fn flush_command(&self, name: &str) {
        if let Ok(mut inner) = self.inner.lock() {
            let last = inner.last_flushed.clone();
            if let Some(lines) = inner.buffers.get_mut(name) {
                if lines.is_empty() { return; }

                if last.as_deref() != Some(name) {
                    eprintln!("    \x1b[2m── {} ──\x1b[0m", name);
                }

                for line in lines.drain(..) {
                    eprintln!("    {line}");
                }

                inner.last_flushed = Some(name.to_string());
            }
        }
    }

    /// Start a background flusher thread. Returns a handle to stop it.
    pub fn start_flusher(&self) -> FlushHandle {
        let sync = self.clone();
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let stop_clone = Arc::clone(&stop);

        let handle = std::thread::spawn(move || {
            while !stop_clone.load(std::sync::atomic::Ordering::Relaxed) {
                std::thread::sleep(std::time::Duration::from_millis(FLUSH_INTERVAL_MS));
                sync.flush_all();
            }
            // Final flush
            sync.flush_all();
        });

        FlushHandle { stop, handle: Some(handle) }
    }
}

/// Handle to stop the background flusher.
pub struct FlushHandle {
    stop: Arc<std::sync::atomic::AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl FlushHandle {
    pub fn stop(mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

impl Drop for FlushHandle {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}
