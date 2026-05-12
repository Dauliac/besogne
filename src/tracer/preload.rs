//! Shared memory preload interposer — fast process telemetry.
//!
//! Uses mmap(MAP_SHARED) for a lock-free ring buffer.
//! The LD_PRELOAD interposer writes events (~10ns each, zero syscalls).
//! After wait4(), the parent reads all events from the shared mapping.

use std::collections::HashSet;
use std::path::PathBuf;

/// Compiled interposer library path (set by build.rs).
const PRELOAD_LIB_PATH: &str = env!("BESOGNE_PRELOAD_LIB");

/// Shared memory buffer size (256KB — holds ~20K events with file tracking).
const SHM_SIZE: usize = 256 * 1024;

#[repr(C)]
struct ShmHeader {
    write_pos: u32,
    buf_size: u32,
}

// Event tags — must match besogne_preload.c
const TAG_ENV: u8 = b'E';
const TAG_EXEC: u8 = b'X';
const TAG_FORK: u8 = b'F';
const TAG_EXIT: u8 = b'Q';
const TAG_CONNECT: u8 = b'C';
const TAG_OPEN: u8 = b'O';
const TAG_DNS: u8 = b'D';
const TAG_UNLINK: u8 = b'U';
const TAG_RENAME: u8 = b'R';
const TAG_DLOPEN: u8 = b'L';

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
    /// Files opened for reading
    pub read_files: HashSet<String>,
    /// Files opened for writing (created/truncated/appended)
    pub written_files: HashSet<String>,
    /// Files deleted (unlink)
    pub deleted_files: HashSet<String>,
    /// Files renamed (old → new)
    pub renamed_files: Vec<(String, String)>,
    /// DNS hostnames resolved via getaddrinfo()
    pub dns_lookups: HashSet<String>,
    /// Shared libraries loaded via dlopen()
    pub loaded_libraries: HashSet<String>,
}

pub fn is_available() -> bool {
    !PRELOAD_LIB_PATH.is_empty() && std::path::Path::new(PRELOAD_LIB_PATH).exists()
}

/// Preload context — manages the shared memory mapping.
pub struct Preload {
    shm_ptr: *mut u8,
    shm_size: usize,
    pub shm_fd: i32,
    pub lib_path: PathBuf,
}

unsafe impl Send for Preload {}

impl Preload {
    pub fn setup() -> Option<Self> {
        if !is_available() { return None; }

        let fd = create_shm_fd()?;
        let size = SHM_SIZE;

        if unsafe { libc::ftruncate(fd, size as libc::off_t) } != 0 {
            unsafe { libc::close(fd); }
            return None;
        }

        let ptr = unsafe {
            libc::mmap(std::ptr::null_mut(), size,
                libc::PROT_READ | libc::PROT_WRITE, libc::MAP_SHARED, fd, 0)
        };
        if ptr == libc::MAP_FAILED {
            unsafe { libc::close(fd); }
            return None;
        }

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

    pub fn preload_env_var() -> &'static str {
        if cfg!(target_os = "macos") { "DYLD_INSERT_LIBRARIES" } else { "LD_PRELOAD" }
    }

    /// Parse all events from shared memory after child exits.
    pub fn collect(self) -> PreloadResults {
        let header = self.shm_ptr as *const ShmHeader;
        let write_pos = unsafe { (*header).write_pos } as usize;
        let buf_size = unsafe { (*header).buf_size } as usize;
        let used = write_pos.min(buf_size);

        let buf = unsafe { std::slice::from_raw_parts(self.shm_ptr.add(8), used) };

        let mut r = PreloadResults::default();
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
                    if let Ok(s) = std::str::from_utf8(payload) {
                        r.accessed_env.insert(s.to_string());
                    }
                }
                TAG_EXEC => {
                    if let Ok(s) = std::str::from_utf8(payload) {
                        r.executed_binaries.insert(s.to_string());
                    }
                }
                TAG_FORK if payload_len >= 4 => {
                    let child = u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]);
                    r.forks.push((pid, child));
                }
                TAG_EXIT => { /* tracked for completeness */ }
                TAG_CONNECT if payload_len >= 8 => {
                    let af = u16::from_le_bytes([payload[0], payload[1]]);
                    let port = u16::from_be_bytes([payload[2], payload[3]]);
                    match af as i32 {
                        libc::AF_INET if payload_len >= 8 => {
                            r.connections.insert(format!("{}.{}.{}.{}:{}",
                                payload[4], payload[5], payload[6], payload[7], port));
                        }
                        libc::AF_INET6 if payload_len >= 20 => {
                            r.connections.insert(format!("[::]:{}",  port));
                        }
                        _ => {}
                    }
                }
                TAG_OPEN if payload_len >= 2 => {
                    let flags = payload[0];
                    if let Ok(path) = std::str::from_utf8(&payload[1..]) {
                        if flags & 1 != 0 {
                            r.written_files.insert(path.to_string());
                        } else {
                            r.read_files.insert(path.to_string());
                        }
                    }
                }
                TAG_DNS => {
                    if let Ok(s) = std::str::from_utf8(payload) {
                        r.dns_lookups.insert(s.to_string());
                    }
                }
                TAG_UNLINK => {
                    if let Ok(s) = std::str::from_utf8(payload) {
                        r.deleted_files.insert(s.to_string());
                    }
                }
                TAG_RENAME if payload_len >= 3 => {
                    let old_len = u16::from_le_bytes([payload[0], payload[1]]) as usize;
                    if old_len + 2 <= payload_len {
                        let old = std::str::from_utf8(&payload[2..2+old_len]).unwrap_or("");
                        let new = std::str::from_utf8(&payload[2+old_len..]).unwrap_or("");
                        if !old.is_empty() {
                            r.renamed_files.push((old.to_string(), new.to_string()));
                        }
                    }
                }
                TAG_DLOPEN => {
                    if let Ok(s) = std::str::from_utf8(payload) {
                        r.loaded_libraries.insert(s.to_string());
                    }
                }
                _ => {}
            }
        }

        r
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

fn create_shm_fd() -> Option<i32> {
    #[cfg(target_os = "linux")]
    {
        let name = std::ffi::CString::new("besogne_preload").ok()?;
        let fd = unsafe { libc::memfd_create(name.as_ptr(), libc::MFD_CLOEXEC) };
        if fd < 0 { return None; }
        unsafe {
            let flags = libc::fcntl(fd, libc::F_GETFD);
            libc::fcntl(fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC);
        }
        Some(fd)
    }

    #[cfg(target_os = "macos")]
    {
        let name = format!("/besogne_{}", std::process::id());
        let c_name = std::ffi::CString::new(name.as_str()).ok()?;
        let fd = unsafe {
            libc::shm_open(c_name.as_ptr(), libc::O_CREAT | libc::O_RDWR, 0o600)
        };
        if fd < 0 { return None; }
        unsafe { libc::shm_unlink(c_name.as_ptr()); }
        Some(fd)
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    { None }
}

/// Compare accessed env vars against declared ones, filtered by static analysis.
///
/// Only flags env vars that are BOTH:
/// 1. Accessed at runtime (via getenv() interposition)
/// 2. Referenced in `run:` scripts ($VAR patterns from static analysis)
///
/// This eliminates false positives from shell/runtime initialization that
/// eagerly reads all env vars (bash environ iteration, Node.js process.env, etc.).
pub fn find_undeclared_env(
    accessed: &HashSet<String>,
    declared: &HashSet<String>,
    statically_referenced: &HashSet<String>,
) -> Vec<String> {
    let mut undeclared: Vec<String> = accessed.iter()
        .filter(|v| !declared.contains(v.as_str()))
        .filter(|v| statically_referenced.contains(v.as_str()))
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
