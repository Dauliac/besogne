//! Environment variable access tracking via LD_PRELOAD/DYLD_INSERT_LIBRARIES.
//!
//! At build time, `build.rs` compiles `envtrack.c` into a shared library.
//! At runtime, we write it to a temp file, create a pipe for tracking,
//! and set the appropriate preload env var before exec.
//! After the command completes, we read the pipe to get accessed var names.

use std::collections::HashSet;
use std::io::Read;
use std::path::PathBuf;

/// The compiled interposer library (embedded at build time).
/// Empty string if compilation failed (cc not available).
const ENVTRACK_LIB_PATH: &str = env!("BESOGNE_ENVTRACK_LIB");

/// Check if env tracking is available (interposer was compiled).
pub fn is_available() -> bool {
    !ENVTRACK_LIB_PATH.is_empty() && std::path::Path::new(ENVTRACK_LIB_PATH).exists()
}

/// Tracking context — manages the pipe and temp library file.
pub struct EnvTracker {
    /// Read end of the tracking pipe
    read_fd: i32,
    /// Write end (passed to child via BESOGNE_ENVTRACK_FD)
    pub write_fd: i32,
    /// Path to the deployed interposer library
    pub lib_path: PathBuf,
}

impl EnvTracker {
    /// Set up env tracking: create pipe, deploy library.
    /// Returns None if tracking is not available.
    pub fn setup() -> Option<Self> {
        if !is_available() {
            return None;
        }

        // Create pipe for tracking
        let mut fds = [0i32; 2];
        let ret = unsafe { libc::pipe(fds.as_mut_ptr()) };
        if ret != 0 {
            return None;
        }

        // Set read end to non-blocking (we'll read after command completes)
        unsafe {
            let flags = libc::fcntl(fds[0], libc::F_GETFL);
            libc::fcntl(fds[0], libc::F_SETFL, flags | libc::O_NONBLOCK);
        }

        Some(Self {
            read_fd: fds[0],
            write_fd: fds[1],
            lib_path: PathBuf::from(ENVTRACK_LIB_PATH),
        })
    }

    /// Get the preload env var name for the current platform.
    pub fn preload_var() -> &'static str {
        if cfg!(target_os = "macos") {
            "DYLD_INSERT_LIBRARIES"
        } else {
            "LD_PRELOAD"
        }
    }

    /// Read accessed env var names from the tracking pipe.
    /// Call this AFTER the child process has exited.
    pub fn collect(mut self) -> HashSet<String> {
        // Close write end (child inherited it, now it's closed on their side too)
        unsafe { libc::close(self.write_fd); }
        self.write_fd = -1;

        // Read all data from the pipe — file takes ownership of fd
        let mut data = Vec::new();
        let mut file: std::fs::File = unsafe { std::os::unix::io::FromRawFd::from_raw_fd(self.read_fd) };
        self.read_fd = -1; // file now owns this fd
        let _ = file.read_to_end(&mut data);

        // Parse: one var name per line
        String::from_utf8_lossy(&data)
            .lines()
            .filter(|l| !l.is_empty())
            .map(|l| l.to_string())
            .collect()
    }
}

impl Drop for EnvTracker {
    fn drop(&mut self) {
        if self.read_fd >= 0 { unsafe { libc::close(self.read_fd); } }
        if self.write_fd >= 0 { unsafe { libc::close(self.write_fd); } }
    }
}

/// Compare accessed env vars against declared ones.
/// Returns the set of env vars that were accessed but not declared.
pub fn find_undeclared(
    accessed: &HashSet<String>,
    declared: &HashSet<String>,
) -> Vec<String> {
    // Essential vars that don't need declaration
    let essential: HashSet<&str> = [
        "PATH", "HOME", "USER", "SHELL", "TERM", "TMPDIR", "LANG", "LC_ALL",
        "LC_CTYPE", "LC_MESSAGES", "TZ", "PWD", "OLDPWD", "SHLVL", "LOGNAME",
        "XDG_CACHE_HOME", "XDG_CONFIG_HOME", "XDG_DATA_HOME", "XDG_RUNTIME_DIR",
        "HOSTNAME", "DISPLAY", "COLORTERM", "TERM_PROGRAM",
        // Nix-specific
        "NIX_PATH", "NIX_PROFILES", "NIX_SSL_CERT_FILE",
        // Common runtime vars
        "EDITOR", "VISUAL", "PAGER", "LESS", "LESSOPEN",
        // Besogne internal
        "BESOGNE_ENVTRACK_FD", "BESOGNE_RUN_MODE", "BESOGNE_COMPONENTS_DIR",
    ].into_iter().collect();

    let mut undeclared: Vec<String> = accessed.iter()
        .filter(|v| !declared.contains(v.as_str()))
        .filter(|v| !essential.contains(v.as_str()))
        // Skip vars starting with _ (internal/private)
        .filter(|v| !v.starts_with('_'))
        .cloned()
        .collect();
    undeclared.sort();
    undeclared
}
