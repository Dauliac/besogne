/// Process tracing — collects metrics from command execution.
/// Uses fork + exec + wait4(rusage) for per-process CPU, memory, I/O metrics.

use std::collections::HashMap;
use std::io::Read;

/// Events emitted by the tracer during command execution
#[derive(Debug, Clone)]
pub enum TraceEvent {
    CommandStart {
        pid: u32,
        cmd: Vec<String>,
    },
    CommandEnd {
        pid: u32,
        exit_code: i32,
        wall_ms: u64,
        user_ms: u64,
        sys_ms: u64,
        max_rss_kb: u64,
    },
}

/// Top-level execution result with metrics from wait4
#[derive(Debug, Clone)]
pub struct CommandResult {
    pub exit_code: i32,
    pub wall_ms: u64,
    pub user_ms: u64,
    pub sys_ms: u64,
    pub max_rss_kb: u64,
    pub voluntary_cs: u64,
    pub involuntary_cs: u64,
    pub processes_spawned: u64,
    pub disk_read_bytes: u64,
    pub disk_write_bytes: u64,
    pub net_read_bytes: u64,
    pub net_write_bytes: u64,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub events: Vec<TraceEvent>,
}

/// Execute a command with wait4 for rusage metrics collection
pub fn execute_traced(
    args: &[String],
    env: &HashMap<String, String>,
    env_isolation: &crate::ir::EnvSandboxResolved,
) -> Result<CommandResult, String> {
    if args.is_empty() {
        return Err("empty command".into());
    }

    // Try the wait4 path on Linux, fall back to std::process::Command
    #[cfg(target_os = "linux")]
    {
        execute_with_wait4(args, env, env_isolation)
    }

    #[cfg(not(target_os = "linux"))]
    {
        execute_simple(args, env, env_isolation)
    }
}

/// Metrics collected by the background scanner thread
#[cfg(target_os = "linux")]
struct ScannerMetrics {
    pids_seen: std::collections::HashSet<u32>,
    /// Last-seen (read_bytes, write_bytes) per PID from /proc/<pid>/io
    io_per_pid: HashMap<u32, (u64, u64)>,
}

/// Collect all descendant PIDs of a given root PID by walking /proc.
/// Returns the set of unique PIDs seen (excluding the root itself).
#[cfg(target_os = "linux")]
fn collect_descendants(root_pid: i32) -> std::collections::HashSet<u32> {
    let mut descendants = std::collections::HashSet::new();
    let Ok(entries) = std::fs::read_dir("/proc") else { return descendants };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name_str) = name.to_str() else { continue };
        let Ok(pid) = name_str.parse::<u32>() else { continue };
        if pid == root_pid as u32 { continue; }
        // Walk the ppid chain to see if this process descends from root_pid
        let mut cur = pid;
        for _ in 0..64 {
            let Ok(stat) = std::fs::read_to_string(format!("/proc/{cur}/stat")) else { break };
            // ppid is field 4; field 2 is (comm) which may contain spaces/parens,
            // so find the closing ')' first then parse remaining fields
            let Some(close_paren) = stat.rfind(')') else { break };
            let rest = &stat[close_paren + 2..]; // skip ") "
            let fields: Vec<&str> = rest.split_whitespace().collect();
            // fields[0] = state, fields[1] = ppid (field 4 in original)
            let Some(ppid_str) = fields.get(1) else { break };
            let Ok(ppid) = ppid_str.parse::<u32>() else { break };
            if ppid == root_pid as u32 {
                descendants.insert(pid);
                break;
            }
            if ppid <= 1 { break; }
            cur = ppid;
        }
    }
    descendants
}

/// Read disk I/O bytes from /proc/<pid>/io.
/// Returns (read_bytes, write_bytes) or (0, 0) if unavailable.
#[cfg(target_os = "linux")]
fn read_proc_io(pid: u32) -> (u64, u64) {
    let Ok(content) = std::fs::read_to_string(format!("/proc/{pid}/io")) else {
        return (0, 0);
    };
    let mut read_bytes = 0u64;
    let mut write_bytes = 0u64;
    for line in content.lines() {
        if let Some(val) = line.strip_prefix("read_bytes: ") {
            read_bytes = val.trim().parse().unwrap_or(0);
        } else if let Some(val) = line.strip_prefix("write_bytes: ") {
            write_bytes = val.trim().parse().unwrap_or(0);
        }
    }
    (read_bytes, write_bytes)
}

/// Read total network rx/tx bytes from /proc/net/dev, summing all non-loopback interfaces.
/// Returns (received_bytes, transmitted_bytes).
#[cfg(target_os = "linux")]
fn read_net_dev() -> (u64, u64) {
    let Ok(content) = std::fs::read_to_string("/proc/net/dev") else {
        return (0, 0);
    };
    let mut rx_total = 0u64;
    let mut tx_total = 0u64;
    for line in content.lines().skip(2) {
        // Format: "  iface: rx_bytes rx_packets ... tx_bytes tx_packets ..."
        let Some((iface, rest)) = line.split_once(':') else { continue };
        if iface.trim() == "lo" { continue; }
        let fields: Vec<&str> = rest.split_whitespace().collect();
        if fields.len() >= 9 {
            rx_total += fields[0].parse::<u64>().unwrap_or(0);
            tx_total += fields[8].parse::<u64>().unwrap_or(0);
        }
    }
    (rx_total, tx_total)
}

/// Linux: fork + exec + wait4 for rusage
#[cfg(target_os = "linux")]
fn execute_with_wait4(
    args: &[String],
    env: &HashMap<String, String>,
    env_isolation: &crate::ir::EnvSandboxResolved,
) -> Result<CommandResult, String> {
    use std::os::unix::io::FromRawFd;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Instant;

    let start = Instant::now();
    let (net_rx_before, net_tx_before) = read_net_dev();

    // Create pipes for stdout and stderr
    let (stdout_read, stdout_write) = pipe().map_err(|e| format!("pipe failed: {e}"))?;
    let (stderr_read, stderr_write) = pipe().map_err(|e| format!("pipe failed: {e}"))?;

    let pid = unsafe { libc::fork() };

    if pid < 0 {
        return Err("fork failed".into());
    }

    if pid == 0 {
        // Child process
        unsafe {
            // Redirect stdout/stderr to pipes
            libc::close(stdout_read);
            libc::close(stderr_read);
            libc::dup2(stdout_write, 1);
            libc::dup2(stderr_write, 2);
            libc::close(stdout_write);
            libc::close(stderr_write);

            // Apply env isolation
            match env_isolation {
                crate::ir::EnvSandboxResolved::Strict => {
                    for (key, _) in std::env::vars() {
                        std::env::remove_var(&key);
                    }
                }
                crate::ir::EnvSandboxResolved::Inherit => {}
            }

            // Set env vars
            for (key, val) in env {
                std::env::set_var(key, val);
            }

            // Build C strings for execvp
            let c_args: Vec<std::ffi::CString> = args
                .iter()
                .map(|a| std::ffi::CString::new(a.as_str()).unwrap())
                .collect();
            let c_arg_ptrs: Vec<*const libc::c_char> = c_args
                .iter()
                .map(|a| a.as_ptr())
                .chain(std::iter::once(std::ptr::null()))
                .collect();

            libc::execvp(c_args[0].as_ptr(), c_arg_ptrs.as_ptr());

            // If we get here, exec failed
            let err = std::io::Error::last_os_error();
            eprintln!("exec failed: {err}");
            libc::_exit(127);
        }
    }

    // Parent process
    unsafe {
        libc::close(stdout_write);
        libc::close(stderr_write);
    }

    // Spawn a background thread to poll /proc for descendant processes and disk I/O
    let stop = Arc::new(AtomicBool::new(false));
    let metrics = Arc::new(Mutex::new(ScannerMetrics {
        pids_seen: std::collections::HashSet::new(),
        io_per_pid: HashMap::new(),
    }));
    let scanner = {
        let stop = Arc::clone(&stop);
        let metrics = Arc::clone(&metrics);
        let root_pid = pid as u32;
        std::thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                let found = collect_descendants(pid);
                if let Ok(mut m) = metrics.lock() {
                    // Snapshot I/O for root child + all descendants
                    m.io_per_pid.insert(root_pid, read_proc_io(root_pid));
                    for &dpid in &found {
                        m.io_per_pid.insert(dpid, read_proc_io(dpid));
                    }
                    m.pids_seen.extend(found);
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        })
    };

    // Read stdout and stderr
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    {
        let mut stdout_file = unsafe { std::fs::File::from_raw_fd(stdout_read) };
        let mut stderr_file = unsafe { std::fs::File::from_raw_fd(stderr_read) };
        // Read both — for simplicity, sequential (TODO: use poll/epoll for interleaving)
        let _ = stdout_file.read_to_end(&mut stdout);
        let _ = stderr_file.read_to_end(&mut stderr);
    }

    // wait4 with rusage
    let mut status: libc::c_int = 0;
    let mut rusage: libc::rusage = unsafe { std::mem::zeroed() };

    let waited = unsafe {
        libc::wait4(pid, &mut status, 0, &mut rusage)
    };

    let wall_ms = start.elapsed().as_millis() as u64;
    let (net_rx_after, net_tx_after) = read_net_dev();
    let net_read_bytes = net_rx_after.saturating_sub(net_rx_before);
    let net_write_bytes = net_tx_after.saturating_sub(net_tx_before);

    // Stop the scanner thread and collect metrics
    stop.store(true, Ordering::Relaxed);
    let _ = scanner.join();
    let (processes_spawned, disk_read_bytes, disk_write_bytes) = metrics
        .lock()
        .map(|m| {
            let (r, w) = m.io_per_pid.values().fold((0u64, 0u64), |(ar, aw), (r, w)| (ar + r, aw + w));
            (m.pids_seen.len() as u64, r, w)
        })
        .unwrap_or((0, 0, 0));

    if waited < 0 {
        return Err(format!("wait4 failed: {}", std::io::Error::last_os_error()));
    }

    let exit_code = if libc::WIFEXITED(status) {
        libc::WEXITSTATUS(status)
    } else if libc::WIFSIGNALED(status) {
        128 + libc::WTERMSIG(status)
    } else {
        -1
    };

    // Extract rusage metrics
    let user_ms = (rusage.ru_utime.tv_sec as u64) * 1000
        + (rusage.ru_utime.tv_usec as u64) / 1000;
    let sys_ms = (rusage.ru_stime.tv_sec as u64) * 1000
        + (rusage.ru_stime.tv_usec as u64) / 1000;
    let max_rss_kb = rusage.ru_maxrss as u64; // Linux: in KB

    Ok(CommandResult {
        exit_code,
        wall_ms,
        user_ms,
        sys_ms,
        max_rss_kb,
        voluntary_cs: rusage.ru_nvcsw as u64,
        involuntary_cs: rusage.ru_nivcsw as u64,
        processes_spawned,
        disk_read_bytes,
        disk_write_bytes,
        net_read_bytes,
        net_write_bytes,
        stdout,
        stderr,
        events: vec![
            TraceEvent::CommandStart {
                pid: pid as u32,
                cmd: args.to_vec(),
            },
            TraceEvent::CommandEnd {
                pid: pid as u32,
                exit_code,
                wall_ms,
                user_ms,
                sys_ms,
                max_rss_kb,
            },
        ],
    })
}

#[cfg(target_os = "linux")]
fn pipe() -> Result<(libc::c_int, libc::c_int), std::io::Error> {
    let mut fds = [0 as libc::c_int; 2];
    let ret = unsafe { libc::pipe(fds.as_mut_ptr()) };
    if ret < 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok((fds[0], fds[1]))
    }
}

/// Non-Linux fallback: std::process::Command
#[cfg(not(target_os = "linux"))]
fn execute_simple(
    args: &[String],
    env: &HashMap<String, String>,
    env_isolation: &crate::ir::EnvSandboxResolved,
) -> Result<CommandResult, String> {
    use std::process::{Command, Stdio};
    use std::time::Instant;

    let start = Instant::now();

    let mut cmd = Command::new(&args[0]);
    cmd.args(&args[1..]);

    match env_isolation {
        crate::ir::EnvSandboxResolved::Strict => {
            cmd.env_clear();
            cmd.envs(env);
        }
        crate::ir::EnvSandboxResolved::Inherit => {
            cmd.envs(env);
        }
    }

    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    let output = cmd
        .output()
        .map_err(|e| format!("cannot execute '{}': {e}", args[0]))?;

    let wall_ms = start.elapsed().as_millis() as u64;

    Ok(CommandResult {
        exit_code: output.status.code().unwrap_or(-1),
        wall_ms,
        user_ms: 0,
        sys_ms: 0,
        max_rss_kb: 0,
        voluntary_cs: 0,
        involuntary_cs: 0,
        processes_spawned: 0,
        disk_read_bytes: 0,
        disk_write_bytes: 0,
        net_read_bytes: 0,
        net_write_bytes: 0,
        stdout: output.stdout,
        stderr: output.stderr,
        events: vec![],
    })
}
