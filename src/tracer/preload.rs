//! Shared memory preload interposer — fast process telemetry.
//!
//! Uses mmap(MAP_SHARED|MAP_ANONYMOUS) for a lock-free ring buffer.
//! The LD_PRELOAD interposer writes events (~10ns each, zero syscalls).
//! After wait4(), the parent reads all events from the shared mapping.

use std::collections::HashSet;
use std::path::PathBuf;

/// Compiled interposer library path (set by build.rs).
const PRELOAD_LIB_PATH: &str = env!("BESOGNE_PRELOAD_LIB");

/// Default shared memory buffer size (64KB — holds ~6000 events).
const SHM_SIZE: usize = 64 * 1024;

/// Header at the start of the shared memory region.
#[repr(C)]
struct ShmHeader {
    write_pos: u32,
    buf_size: u32,
}

/// Event tags matching the C interposer.
const TAG_ENV: u8 = b'E';
const TAG_EXEC: u8 = b'X';
const TAG_FORK: u8 = b'F';
const TAG_EXIT: u8 = b'Q';
const TAG_CONNECT: u8 = b'C';

/// A parsed event from the ring buffer.
#[derive(Debug)]
pub enum PreloadEvent {
    Env { pid: u32, name: String },
    Exec { pid: u32, path: String },
    Fork { pid: u32, child_pid: u32 },
    Exit { pid: u32, code: i32 },
    Connect { pid: u32, addr: String },
}

/// Collected results from the preload interposer.
#[derive(Debug, Default, Clone)]
pub struct PreloadResults {
    /// Env vars accessed via getenv()
    pub accessed_env: HashSet<String>,
    /// Binary paths passed to execve()
    pub executed_binaries: HashSet<String>,
    /// Fork events (parent_pid → child_pid)
    pub forks: Vec<(u32, u32)>,
    /// Network connect targets (ip:port)
    pub connections: HashSet<String>,
}

/// Check if the preload interposer is available.
pub fn is_available() -> bool {
    !PRELOAD_LIB_PATH.is_empty() && std::path::Path::new(PRELOAD_LIB_PATH).exists()
}

/// Preload context — manages the shared memory mapping.
pub struct Preload {
    /// Pointer to the shared memory region
    shm_ptr: *mut u8,
    /// Total size of the mapping
    shm_size: usize,
    /// File descriptor for the memfd (child inherits this)
    pub shm_fd: i32,
    /// Path to the interposer library
    pub lib_path: PathBuf,
}

// SAFETY: the shared memory is used via atomic operations in C,
// and only read from Rust after the child process exits.
unsafe impl Send for Preload {}

impl Preload {
    /// Set up shared memory and prepare the interposer.
    pub fn setup() -> Option<Self> {
        if !is_available() {
            return None;
        }

        // Create anonymous shared memory via memfd_create (Linux) or shm_open (macOS)
        let fd = create_shm_fd()?;

        // Set size
        let size = SHM_SIZE;
        if unsafe { libc::ftruncate(fd, size as libc::off_t) } != 0 {
            unsafe { libc::close(fd); }
            return None;
        }

        // Map it
        let ptr = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                size,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                fd,
                0,
            )
        };

        if ptr == libc::MAP_FAILED {
            unsafe { libc::close(fd); }
            return None;
        }

        // Initialize header
        let header = ptr as *mut ShmHeader;
        unsafe {
            (*header).write_pos = 0;
            (*header).buf_size = (size - 8) as u32;
        }

        Some(Self {
            shm_ptr: ptr as *mut u8,
            shm_size: size,
            shm_fd: fd,
            lib_path: PathBuf::from(PRELOAD_LIB_PATH),
        })
    }

    /// Get the preload env var name for the current platform.
    pub fn preload_env_var() -> &'static str {
        if cfg!(target_os = "macos") {
            "DYLD_INSERT_LIBRARIES"
        } else {
            "LD_PRELOAD"
        }
    }

    /// Collect all events from the shared memory after the child exits.
    pub fn collect(self) -> PreloadResults {
        let header = self.shm_ptr as *const ShmHeader;
        let write_pos = unsafe { (*header).write_pos } as usize;
        let buf_size = unsafe { (*header).buf_size } as usize;
        let used = write_pos.min(buf_size);

        let buf = unsafe {
            std::slice::from_raw_parts(self.shm_ptr.add(8), used)
        };

        let mut results = PreloadResults::default();
        let mut pos = 0;

        while pos + 7 <= used {
            let tag = buf[pos];
            let pid = u32::from_le_bytes([buf[pos+1], buf[pos+2], buf[pos+3], buf[pos+4]]);
            let payload_len = u16::from_le_bytes([buf[pos+5], buf[pos+6]]) as usize;
            pos += 7;

            if pos + payload_len > used { break; }
            let payload = &buf[pos..pos + payload_len];
            pos += payload_len;

            match tag {
                TAG_ENV => {
                    if let Ok(name) = std::str::from_utf8(payload) {
                        results.accessed_env.insert(name.to_string());
                    }
                }
                TAG_EXEC => {
                    if let Ok(path) = std::str::from_utf8(payload) {
                        results.executed_binaries.insert(path.to_string());
                    }
                }
                TAG_FORK if payload_len >= 4 => {
                    let child_pid = u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]);
                    results.forks.push((pid, child_pid));
                }
                TAG_EXIT if payload_len >= 4 => {
                    let _code = i32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]);
                    // Exit events tracked for completeness; process tree uses wait4 rusage
                }
                TAG_CONNECT if payload_len >= 8 => {
                    let af = u16::from_le_bytes([payload[0], payload[1]]);
                    let port = u16::from_be_bytes([payload[2], payload[3]]); // network byte order
                    match af as i32 {
                        libc::AF_INET if payload_len >= 8 => {
                            let addr = format!("{}.{}.{}.{}:{}",
                                payload[4], payload[5], payload[6], payload[7], port);
                            results.connections.insert(addr);
                        }
                        libc::AF_INET6 if payload_len >= 20 => {
                            // Simplified: just show port for IPv6
                            results.connections.insert(format!("[::]:{}",  port));
                        }
                        _ => {}
                    }
                }
                _ => {} // Unknown tag — skip
            }
        }

        results
    }
}

impl Drop for Preload {
    fn drop(&mut self) {
        if !self.shm_ptr.is_null() {
            unsafe { libc::munmap(self.shm_ptr as *mut libc::c_void, self.shm_size); }
        }
        if self.shm_fd >= 0 {
            unsafe { libc::close(self.shm_fd); }
        }
    }
}

/// Create an anonymous shared memory file descriptor.
fn create_shm_fd() -> Option<i32> {
    #[cfg(target_os = "linux")]
    {
        let name = std::ffi::CString::new("besogne_preload").ok()?;
        let fd = unsafe { libc::memfd_create(name.as_ptr(), libc::MFD_CLOEXEC) };
        if fd < 0 { return None; }
        // Clear CLOEXEC so child inherits it
        unsafe {
            let flags = libc::fcntl(fd, libc::F_GETFD);
            libc::fcntl(fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC);
        }
        Some(fd)
    }

    #[cfg(target_os = "macos")]
    {
        // macOS: use shm_open with a unique name
        let name = format!("/besogne_{}", std::process::id());
        let c_name = std::ffi::CString::new(name.as_str()).ok()?;
        let fd = unsafe {
            libc::shm_open(c_name.as_ptr(), libc::O_CREAT | libc::O_RDWR, 0o600)
        };
        if fd < 0 { return None; }
        // Unlink immediately — fd keeps it alive
        unsafe { libc::shm_unlink(c_name.as_ptr()); }
        Some(fd)
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        None
    }
}

/// Compare accessed env vars against declared ones.
pub fn find_undeclared_env(
    accessed: &HashSet<String>,
    declared: &HashSet<String>,
) -> Vec<String> {
    let essential: HashSet<&str> = [
        "PATH", "HOME", "USER", "SHELL", "TERM", "TMPDIR", "LANG", "LC_ALL",
        "LC_CTYPE", "LC_MESSAGES", "TZ", "PWD", "OLDPWD", "SHLVL", "LOGNAME",
        "XDG_CACHE_HOME", "XDG_CONFIG_HOME", "XDG_DATA_HOME", "XDG_RUNTIME_DIR",
        "HOSTNAME", "DISPLAY", "COLORTERM", "TERM_PROGRAM",
        "NIX_PATH", "NIX_PROFILES", "NIX_SSL_CERT_FILE",
        "EDITOR", "VISUAL", "PAGER", "LESS", "LESSOPEN",
        "BESOGNE_PRELOAD_FD", "BESOGNE_RUN_MODE", "BESOGNE_COMPONENTS_DIR",
    ].into_iter().collect();

    let mut undeclared: Vec<String> = accessed.iter()
        .filter(|v| !declared.contains(v.as_str()))
        .filter(|v| !essential.contains(v.as_str()))
        .filter(|v| !v.starts_with('_'))
        .cloned()
        .collect();
    undeclared.sort();
    undeclared
}

/// Compare executed binaries against declared ones.
pub fn find_undeclared_binaries(
    executed: &HashSet<String>,
    declared: &HashSet<String>,
) -> Vec<String> {
    let skip = ["bash", "sh", "dash", "zsh", "fish", "csh", "tcsh", "env"];

    let mut undeclared: Vec<String> = executed.iter()
        .filter_map(|path| {
            let basename = path.rsplit('/').next().unwrap_or(path);
            if declared.contains(basename) { return None; }
            if skip.contains(&basename) { return None; }
            if basename.chars().all(|c| c.is_ascii_hexdigit()) { return None; }
            if path.contains("besogne") { return None; }
            Some(basename.to_string())
        })
        .collect();
    undeclared.sort();
    undeclared.dedup();
    undeclared
}
