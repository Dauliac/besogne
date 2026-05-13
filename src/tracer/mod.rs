/// Process tracing — collects metrics from command execution.
///
/// Uses fork + exec + wait4(rusage) for per-process CPU, memory, I/O metrics.
/// On Linux, uses PR_SET_CHILD_SUBREAPER to adopt all orphaned descendants,
/// then reaps each one individually with wait4(-1) to get per-process rusage.
/// A background /proc scanner captures command names and ppid for tree structure.

pub mod envtrack;
pub mod output_sync;
pub mod preload;

use std::collections::{HashMap, HashSet};

/// Per-process metrics collected via wait4 rusage + /proc scanning
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ProcessMetrics {
    pub pid: u32,
    pub ppid: u32,
    /// Command name (from /proc/<pid>/comm or exec args)
    pub comm: String,
    /// Full command line (from /proc/<pid>/cmdline), empty if unavailable
    pub cmdline: String,
    pub exit_code: i32,
    pub wall_ms: u64,
    pub user_ms: u64,
    pub sys_ms: u64,
    pub max_rss_kb: u64,
    pub voluntary_cs: u64,
    pub involuntary_cs: u64,
    pub read_bytes: u64,
    pub write_bytes: u64,
}

/// Events emitted by the tracer during command execution
#[derive(Debug, Clone)]
#[allow(dead_code)]
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
    #[allow(dead_code)]
    pub voluntary_cs: u64,
    #[allow(dead_code)]
    pub involuntary_cs: u64,
    pub processes_spawned: u64,
    pub disk_read_bytes: u64,
    pub disk_write_bytes: u64,
    pub net_read_bytes: u64,
    pub net_write_bytes: u64,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    #[allow(dead_code)]
    pub events: Vec<TraceEvent>,
    /// Per-process metrics for the entire process tree (root + all descendants)
    pub process_tree: Vec<ProcessMetrics>,
    /// Env vars accessed by the command (via LD_PRELOAD tracking). Empty if tracking unavailable.
    pub accessed_env: HashSet<String>,
    /// Full preload results (exec'd binaries, connections, etc.)
    pub preload: Option<preload::PreloadResults>,
}

/// Execute a command with wait4 for rusage metrics collection
/// Execute with synchronized output for parallel commands.
/// Lines are buffered in `sync` and flushed in blocks with headers.
pub fn execute_traced_parallel(
    args: &[String],
    env: &HashMap<String, String>,
    env_isolation: &crate::ir::EnvSandboxResolved,
    workdir: Option<&str>,
    sync: &output_sync::OutputSync,
    cmd_name: &str,
    resources: &crate::ir::ResourceLimits,
) -> Result<CommandResult, crate::error::BesogneError> {
    // Set thread-local state for parallel execution
    PARALLEL_MODE.with(|c| c.set(true));
    OUTPUT_SYNC.with(|cell| {
        *cell.borrow_mut() = Some((sync.clone(), cmd_name.to_string()));
    });
    let result = execute_traced(args, env, env_isolation, workdir, resources);
    OUTPUT_SYNC.with(|cell| { *cell.borrow_mut() = None; });
    PARALLEL_MODE.with(|c| c.set(false));
    // Final flush for this command
    sync.flush_command(cmd_name);
    result
}

// Thread-local output sync context (set by execute_traced_parallel)
thread_local! {
    static OUTPUT_SYNC: std::cell::RefCell<Option<(output_sync::OutputSync, String)>> = std::cell::RefCell::new(None);
}

/// Write a line to either OutputSync (parallel) or stderr (sequential).
#[allow(dead_code)]
fn emit_output_line(line: &str) {
    OUTPUT_SYNC.with(|cell| {
        let borrow = cell.borrow();
        if let Some((sync, name)) = borrow.as_ref() {
            sync.push_line(name, line);
        } else {
            eprintln!("    {line}");
        }
    });
}

// Whether we're in parallel mode (multiple commands in same tier).
// When true, skip PR_SET_CHILD_SUBREAPER and wait4(-1) to avoid cross-thread reaping.
thread_local! {
    static PARALLEL_MODE: std::cell::Cell<bool> = std::cell::Cell::new(false);
}

pub fn execute_traced(
    args: &[String],
    env: &HashMap<String, String>,
    env_isolation: &crate::ir::EnvSandboxResolved,
    workdir: Option<&str>,
    resources: &crate::ir::ResourceLimits,
) -> Result<CommandResult, crate::error::BesogneError> {
    if args.is_empty() {
        return Err(crate::error::BesogneError::Tracer("empty command".into()));
    }

    #[cfg(target_os = "linux")]
    {
        execute_with_wait4(args, env, env_isolation, workdir, resources)
    }

    #[cfg(not(target_os = "linux"))]
    {
        execute_simple(args, env, env_isolation, workdir, resources)
    }
}

/// Per-PID metadata captured by the /proc scanner
#[cfg(target_os = "linux")]
#[derive(Debug, Clone)]
struct ProcSnapshot {
    #[allow(dead_code)]
    pid: u32,
    ppid: u32,
    comm: String,
    cmdline: String,
    read_bytes: u64,
    write_bytes: u64,
    first_seen: std::time::Instant,
    last_seen: std::time::Instant,
}

/// Metrics collected by the background scanner thread
#[cfg(target_os = "linux")]
struct ScannerMetrics {
    snapshots: HashMap<u32, ProcSnapshot>,
}

/// Read comm from /proc/<pid>/comm
#[cfg(target_os = "linux")]
fn read_proc_comm(pid: u32) -> String {
    std::fs::read_to_string(format!("/proc/{pid}/comm"))
        .unwrap_or_default()
        .trim()
        .to_string()
}

/// Read cmdline from /proc/<pid>/cmdline (NUL-separated)
#[cfg(target_os = "linux")]
fn read_proc_cmdline(pid: u32) -> String {
    std::fs::read(format!("/proc/{pid}/cmdline"))
        .unwrap_or_default()
        .split(|&b| b == 0)
        .filter(|s| !s.is_empty())
        .map(|s| String::from_utf8_lossy(s).to_string())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Read ppid from /proc/<pid>/stat
#[cfg(target_os = "linux")]
fn read_proc_ppid(pid: u32) -> u32 {
    let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else { return 0 };
    let Some(close_paren) = stat.rfind(')') else { return 0 };
    let rest = &stat[close_paren + 2..];
    let fields: Vec<&str> = rest.split_whitespace().collect();
    fields.get(1).and_then(|s| s.parse().ok()).unwrap_or(0)
}

/// Collect all descendant PIDs by walking the process tree top-down.
///
/// Uses `/proc/<pid>/task/<pid>/children` (Linux 3.5+) which directly lists
/// child PIDs — O(tree_size) instead of O(all_procs_on_system).
/// Falls back to full /proc scan if the children file is unavailable.
#[cfg(target_os = "linux")]
fn collect_descendants(root_pid: i32) -> std::collections::HashSet<u32> {
    let mut descendants = std::collections::HashSet::new();
    let mut stack = vec![root_pid as u32];

    while let Some(pid) = stack.pop() {
        // Try fast path: /proc/<pid>/task/<pid>/children
        let children_path = format!("/proc/{pid}/task/{pid}/children");
        if let Ok(content) = std::fs::read_to_string(&children_path) {
            for token in content.split_whitespace() {
                if let Ok(child_pid) = token.parse::<u32>() {
                    if child_pid != root_pid as u32 && descendants.insert(child_pid) {
                        stack.push(child_pid);
                    }
                }
            }
        } else if pid == root_pid as u32 {
            // Fallback for root only: scan /proc for direct children by ppid
            if let Ok(entries) = std::fs::read_dir("/proc") {
                for entry in entries.flatten() {
                    let name = entry.file_name();
                    let Some(name_str) = name.to_str() else { continue };
                    let Ok(cpid) = name_str.parse::<u32>() else { continue };
                    if cpid == root_pid as u32 { continue; }
                    if read_proc_ppid(cpid) == root_pid as u32 {
                        if descendants.insert(cpid) {
                            stack.push(cpid);
                        }
                    }
                }
            }
        }
    }
    descendants
}

/// Read I/O bytes from /proc/<pid>/io (rchar/wchar).
#[cfg(target_os = "linux")]
fn read_proc_io(pid: u32) -> (u64, u64) {
    let Ok(content) = std::fs::read_to_string(format!("/proc/{pid}/io")) else {
        return (0, 0);
    };
    let mut rchar = 0u64;
    let mut wchar = 0u64;
    for line in content.lines() {
        if let Some(val) = line.strip_prefix("rchar: ") {
            rchar = val.trim().parse().unwrap_or(0);
        } else if let Some(val) = line.strip_prefix("wchar: ") {
            wchar = val.trim().parse().unwrap_or(0);
        }
    }
    (rchar, wchar)
}

/// Read total network rx/tx bytes from /proc/net/dev, summing all non-loopback interfaces.
#[cfg(target_os = "linux")]
fn read_net_dev() -> (u64, u64) {
    let Ok(content) = std::fs::read_to_string("/proc/net/dev") else {
        return (0, 0);
    };
    let mut rx_total = 0u64;
    let mut tx_total = 0u64;
    for line in content.lines().skip(2) {
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

/// Convert libc::rusage to per-process metrics
#[cfg(target_os = "linux")]
fn rusage_to_metrics(
    pid: u32,
    rusage: &libc::rusage,
    exit_code: i32,
    snapshot: Option<&ProcSnapshot>,
    wall_ms: u64,
) -> ProcessMetrics {
    let user_ms = (rusage.ru_utime.tv_sec as u64) * 1000
        + (rusage.ru_utime.tv_usec as u64) / 1000;
    let sys_ms = (rusage.ru_stime.tv_sec as u64) * 1000
        + (rusage.ru_stime.tv_usec as u64) / 1000;

    ProcessMetrics {
        pid,
        ppid: snapshot.map_or(0, |s| s.ppid),
        comm: snapshot.map_or_else(String::new, |s| s.comm.clone()),
        cmdline: snapshot.map_or_else(String::new, |s| s.cmdline.clone()),
        exit_code,
        wall_ms,
        user_ms,
        sys_ms,
        max_rss_kb: rusage.ru_maxrss as u64,
        voluntary_cs: rusage.ru_nvcsw as u64,
        involuntary_cs: rusage.ru_nivcsw as u64,
        read_bytes: snapshot.map_or(0, |s| s.read_bytes),
        write_bytes: snapshot.map_or(0, |s| s.write_bytes),
    }
}

/// Linux: fork + exec + wait4 for rusage, with PR_SET_CHILD_SUBREAPER
/// to reap all descendants and collect per-process metrics.
#[cfg(target_os = "linux")]
fn execute_with_wait4(
    args: &[String],
    env: &HashMap<String, String>,
    env_isolation: &crate::ir::EnvSandboxResolved,
    workdir: Option<&str>,
    resources: &crate::ir::ResourceLimits,
) -> Result<CommandResult, crate::error::BesogneError> {
    use std::os::unix::io::FromRawFd;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Instant;

    let start = Instant::now();
    let (net_rx_before, net_tx_before) = read_net_dev();

    // Become subreaper: all orphaned descendants will be reparented to us
    // so we can wait4 them and collect their rusage.
    // Skip in parallel mode: subreaper is per-process, wait4(-1) would steal
    // children from other threads.
    let is_parallel = PARALLEL_MODE.with(|c| c.get());
    if !is_parallel {
        unsafe { libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 1, 0, 0, 0); }
    }

    // Set up env tracking (LD_PRELOAD interposer)
    let env_tracker = envtrack::EnvTracker::setup();

    // Set up shared memory preload (fast telemetry: exec, env, connect)
    let preload_ctx = preload::Preload::setup();

    // Create pipes for stdout and stderr
    let (stdout_read, stdout_write) = pipe().map_err(|e| crate::error::BesogneError::Tracer(format!("pipe failed: {e}")))?;
    let (stderr_read, stderr_write) = pipe().map_err(|e| crate::error::BesogneError::Tracer(format!("pipe failed: {e}")))?;

    let pid = unsafe { libc::fork() };

    if pid < 0 {
        return Err(crate::error::BesogneError::Tracer("fork failed".into()));
    }

    if pid == 0 {
        // Child process
        unsafe {
            libc::setpgid(0, 0);
            // Allow parent to read /proc/<child>/io for disk I/O metrics
            libc::prctl(libc::PR_SET_DUMPABLE, 1, 0, 0, 0);

            // chdir to per-command workdir if specified
            if let Some(dir) = workdir {
                let c_dir = std::ffi::CString::new(dir).unwrap();
                if libc::chdir(c_dir.as_ptr()) != 0 {
                    eprintln!("chdir failed: {}: {}", dir, std::io::Error::last_os_error());
                    libc::_exit(126);
                }
            }

            libc::close(stdout_read);
            libc::close(stderr_read);
            libc::dup2(stdout_write, 1);
            libc::dup2(stderr_write, 2);
            libc::close(stdout_write);
            libc::close(stderr_write);

            match env_isolation {
                crate::ir::EnvSandboxResolved::Strict => {
                    for (key, _) in std::env::vars() {
                        std::env::remove_var(&key);
                    }
                }
                crate::ir::EnvSandboxResolved::Inherit => {}
            }

            for (key, val) in env {
                std::env::set_var(key, val);
            }

            // Set up preload interposer (shared memory ring buffer)
            if let Some(ref ctx) = preload_ctx {
                std::env::set_var(preload::Preload::preload_env_var(), &ctx.lib_path);
                std::env::set_var("BESOGNE_PRELOAD_FD", ctx.shm_fd.to_string());
            } else if let Some(ref tracker) = env_tracker {
                // Fallback to pipe-based env tracking if preload unavailable
                std::env::set_var(envtrack::EnvTracker::preload_var(), &tracker.lib_path);
                std::env::set_var("BESOGNE_ENVTRACK_FD", tracker.write_fd.to_string());
            }

            // Apply resource limits before exec
            apply_resource_limits(resources);

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

    // Spawn background scanner to capture per-process metadata from /proc
    let stop = Arc::new(AtomicBool::new(false));
    let metrics = Arc::new(Mutex::new(ScannerMetrics {
        snapshots: HashMap::new(),
    }));
    let scanner = {
        let stop = Arc::clone(&stop);
        let metrics = Arc::clone(&metrics);
        std::thread::spawn(move || {
            let root_pid = pid as u32;
            let scan_start = Instant::now();
            let mut scan_count = 0u64;
            while !stop.load(Ordering::Relaxed) {
                let now = Instant::now();
                let found = collect_descendants(pid);

                if let Ok(mut m) = metrics.lock() {
                    snapshot_pid(&mut m, root_pid, now);
                    for &dpid in &found {
                        snapshot_pid(&mut m, dpid, now);
                    }
                }

                // Adaptive frequency: fast at start to catch short-lived processes,
                // then slow down to reduce CPU overhead on long-running commands.
                // 500μs for first 200ms, 1ms until 1s, then 5ms steady-state.
                scan_count += 1;
                let elapsed = scan_start.elapsed().as_millis();
                let sleep_us = if elapsed < 200 { 500 }
                    else if elapsed < 1000 { 1000 }
                    else { 5000 };
                std::thread::sleep(std::time::Duration::from_micros(sleep_us));
            }
            let _ = scan_count; // suppress unused warning
        })
    };

    // Read stdout and stderr — stream to terminal while also capturing for cache.
    // Uses threads to read both pipes concurrently (prevents deadlock if child
    // fills one pipe buffer while we're blocked reading the other).
    // Capture output sync context before spawning (thread-local doesn't transfer)
    let output_ctx: Option<(output_sync::OutputSync, String)> = OUTPUT_SYNC.with(|cell| cell.borrow().clone());
    let output_ctx2 = output_ctx.clone();

    let stdout_handle = {
        let stdout_file = unsafe { std::fs::File::from_raw_fd(stdout_read) };
        std::thread::spawn(move || {
            use std::io::BufRead;
            let mut buf = Vec::new();
            let reader = std::io::BufReader::new(stdout_file);
            for line in reader.lines() {
                if let Ok(line) = line {
                    if let Some((ref sync, ref name)) = output_ctx {
                        sync.push_line(name, &line);
                    } else {
                        eprintln!("    {line}");
                    }
                    buf.extend(line.as_bytes());
                    buf.push(b'\n');
                }
            }
            buf
        })
    };
    let stderr_handle = {
        let stderr_file = unsafe { std::fs::File::from_raw_fd(stderr_read) };
        std::thread::spawn(move || {
            use std::io::BufRead;
            let mut buf = Vec::new();
            let reader = std::io::BufReader::new(stderr_file);
            for line in reader.lines() {
                if let Ok(line) = line {
                    if let Some((ref sync, ref name)) = output_ctx2 {
                        sync.push_line(name, &line);
                    } else {
                        eprintln!("    {line}");
                    }
                    buf.extend(line.as_bytes());
                    buf.push(b'\n');
                }
            }
            buf
        })
    };
    let stdout = stdout_handle.join().unwrap_or_default();
    let stderr = stderr_handle.join().unwrap_or_default();

    // Final /proc snapshot for the root child (while still a zombie)
    if let Ok(mut m) = metrics.lock() {
        snapshot_pid(&mut m, pid as u32, std::time::Instant::now());
    }

    // Reap the main child with wait4
    let mut status: libc::c_int = 0;
    let mut rusage: libc::rusage = unsafe { std::mem::zeroed() };
    let waited = unsafe { libc::wait4(pid, &mut status, 0, &mut rusage) };

    let main_wall_ms = start.elapsed().as_millis() as u64;

    if waited < 0 {
        return Err(crate::error::BesogneError::Tracer(format!("wait4 failed: {}", std::io::Error::last_os_error())));
    }

    let main_exit = extract_exit_code(status);

    // Build ProcessMetrics for the root child
    let root_snapshot = metrics.lock().ok().and_then(|m| m.snapshots.get(&(pid as u32)).cloned());
    let mut process_tree = vec![ProcessMetrics {
        comm: root_snapshot.as_ref().map_or_else(
            || args[0].clone(),
            |s| if s.comm.is_empty() { args[0].clone() } else { s.comm.clone() },
        ),
        cmdline: root_snapshot.as_ref().map_or_else(
            || args.join(" "),
            |s| if s.cmdline.is_empty() { args.join(" ") } else { s.cmdline.clone() },
        ),
        ppid: std::process::id(),
        ..rusage_to_metrics(pid as u32, &rusage, main_exit, root_snapshot.as_ref(), main_wall_ms)
    }];

    // Reap all orphaned descendants (adopted via PR_SET_CHILD_SUBREAPER).
    // Skip in parallel mode to avoid cross-thread child stealing.
    while !is_parallel {
        let mut child_status: libc::c_int = 0;
        let mut child_rusage: libc::rusage = unsafe { std::mem::zeroed() };
        let child_pid = unsafe {
            libc::wait4(-1, &mut child_status, libc::WNOHANG, &mut child_rusage)
        };
        if child_pid <= 0 { break; }

        let child_exit = extract_exit_code(child_status);
        let snap = metrics.lock().ok().and_then(|m| m.snapshots.get(&(child_pid as u32)).cloned());
        let child_wall = snap.as_ref().map_or(0, |s| {
            s.last_seen.duration_since(s.first_seen).as_millis() as u64
        });
        process_tree.push(rusage_to_metrics(
            child_pid as u32,
            &child_rusage,
            child_exit,
            snap.as_ref(),
            child_wall,
        ));
    }

    // Disable subreaper (only if we set it)
    if !is_parallel {
        unsafe { libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 0, 0, 0, 0); }
    }

    let (net_rx_after, net_tx_after) = read_net_dev();
    let net_read_bytes = net_rx_after.saturating_sub(net_rx_before);
    let net_write_bytes = net_tx_after.saturating_sub(net_tx_before);

    // Stop the scanner
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    let _ = scanner.join();

    // Add all scanner-captured descendants that weren't reaped via wait4.
    // Most children exit normally within their parent (not orphaned), so
    // the subreaper never sees them. The /proc scanner is the primary source.
    let reaped_pids: std::collections::HashSet<u32> = process_tree.iter().map(|p| p.pid).collect();
    if let Ok(m) = metrics.lock() {
        for (dpid, snap) in &m.snapshots {
            if reaped_pids.contains(dpid) { continue; }
            let wall = snap.last_seen.duration_since(snap.first_seen).as_millis() as u64;
            process_tree.push(ProcessMetrics {
                pid: *dpid,
                ppid: snap.ppid,
                comm: snap.comm.clone(),
                cmdline: snap.cmdline.clone(),
                exit_code: 0, // exited normally (not reaped = parent waited for it)
                wall_ms: wall,
                user_ms: 0,   // no rusage available (parent reaped it)
                sys_ms: 0,
                max_rss_kb: 0,
                voluntary_cs: 0,
                involuntary_cs: 0,
                read_bytes: snap.read_bytes,
                write_bytes: snap.write_bytes,
            });
        }
    }

    // Aggregate metrics across the whole tree
    let total_read = process_tree.iter().map(|p| p.read_bytes).sum::<u64>();
    let total_write = process_tree.iter().map(|p| p.write_bytes).sum::<u64>();
    let total_user = process_tree.iter().map(|p| p.user_ms).sum::<u64>();
    let total_sys = process_tree.iter().map(|p| p.sys_ms).sum::<u64>();
    let max_rss = process_tree.iter().map(|p| p.max_rss_kb).max().unwrap_or(0);
    let total_vcs = process_tree.iter().map(|p| p.voluntary_cs).sum::<u64>();
    let total_ics = process_tree.iter().map(|p| p.involuntary_cs).sum::<u64>();

    Ok(CommandResult {
        exit_code: main_exit,
        wall_ms: main_wall_ms,
        user_ms: total_user,
        sys_ms: total_sys,
        max_rss_kb: max_rss,
        voluntary_cs: total_vcs,
        involuntary_cs: total_ics,
        processes_spawned: process_tree.len() as u64 - 1,
        disk_read_bytes: total_read,
        disk_write_bytes: total_write,
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
                exit_code: main_exit,
                wall_ms: main_wall_ms,
                user_ms: total_user,
                sys_ms: total_sys,
                max_rss_kb: max_rss,
            },
        ],
        process_tree,
        accessed_env: if preload_ctx.is_some() {
            HashSet::new() // preload results have env data
        } else {
            env_tracker.map(|t| t.collect()).unwrap_or_default()
        },
        preload: preload_ctx.map(|p| p.collect()),
    })
}

/// Update or insert a /proc snapshot for a given PID
#[cfg(target_os = "linux")]
fn snapshot_pid(m: &mut ScannerMetrics, pid: u32, now: std::time::Instant) {
    let (r, w) = read_proc_io(pid);
    let entry = m.snapshots.entry(pid).or_insert_with(|| ProcSnapshot {
        pid,
        ppid: read_proc_ppid(pid),
        comm: read_proc_comm(pid),
        cmdline: read_proc_cmdline(pid),
        read_bytes: 0,
        write_bytes: 0,
        first_seen: now,
        last_seen: now,
    });
    entry.read_bytes = entry.read_bytes.max(r);
    entry.write_bytes = entry.write_bytes.max(w);
    entry.last_seen = now;
}

#[cfg(target_os = "linux")]
fn extract_exit_code(status: libc::c_int) -> i32 {
    if libc::WIFEXITED(status) {
        libc::WEXITSTATUS(status)
    } else if libc::WIFSIGNALED(status) {
        128 + libc::WTERMSIG(status)
    } else {
        -1
    }
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
    workdir: Option<&str>,
    resources: &crate::ir::ResourceLimits,
) -> Result<CommandResult, crate::error::BesogneError> {
    use std::process::{Command, Stdio};
    use std::time::Instant;

    let start = Instant::now();

    let mut cmd = Command::new(&args[0]);
    cmd.args(&args[1..]);
    if let Some(dir) = workdir {
        cmd.current_dir(dir);
    }

    // Set up tracking
    let env_tracker = envtrack::EnvTracker::setup();
    let preload_ctx = preload::Preload::setup();

    match env_isolation {
        crate::ir::EnvSandboxResolved::Strict => {
            cmd.env_clear();
            cmd.envs(env);
        }
        crate::ir::EnvSandboxResolved::Inherit => {
            cmd.envs(env);
        }
    }

    // Inject preload interposer
    if let Some(ref ctx) = preload_ctx {
        cmd.env(preload::Preload::preload_env_var(), &ctx.lib_path);
        cmd.env("BESOGNE_PRELOAD_FD", ctx.shm_fd.to_string());
    } else if let Some(ref tracker) = env_tracker {
        cmd.env(envtrack::EnvTracker::preload_var(), &tracker.lib_path);
        cmd.env("BESOGNE_ENVTRACK_FD", tracker.write_fd.to_string());
    }

    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    // Apply resource limits in the child process via pre_exec
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let resources = resources.clone();
        unsafe {
            cmd.pre_exec(move || {
                apply_resource_limits(&resources);
                Ok(())
            });
        }
    }

    let output = cmd
        .output()
        .map_err(|e| crate::error::BesogneError::Tracer(format!("cannot execute '{}': {e}", args[0])))?;

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
        process_tree: vec![],
        accessed_env: if preload_ctx.is_some() {
            HashSet::new()
        } else {
            env_tracker.map(|t| t.collect()).unwrap_or_default()
        },
        preload: preload_ctx.map(|p| p.collect()),
    })
}

/// Apply resource limits in a child process (after fork, before exec).
/// Works on both Linux and macOS (POSIX).
#[cfg(unix)]
fn apply_resource_limits(resources: &crate::ir::ResourceLimits) {
    use crate::ir::PriorityResolved;

    // 1. CPU scheduling priority (nice)
    match resources.priority {
        PriorityResolved::Normal => {}
        PriorityResolved::Low => {
            unsafe { libc::setpriority(libc::PRIO_PROCESS, 0, 10); }
            apply_io_priority_low();
        }
        PriorityResolved::Background => {
            #[cfg(target_os = "macos")]
            {
                // macOS: PRIO_DARWIN_BG sets full background mode (CPU + I/O + timers)
                unsafe { libc::setpriority(libc::PRIO_DARWIN_BG, 0, 1); }
            }
            #[cfg(not(target_os = "macos"))]
            {
                unsafe { libc::setpriority(libc::PRIO_PROCESS, 0, 19); }
                apply_io_priority_idle();
            }
        }
    }

    // 2. Memory limit via RLIMIT_AS (address space limit — POSIX)
    if let Some(limit) = resources.memory_limit {
        let rlim = libc::rlimit {
            rlim_cur: limit,
            rlim_max: limit,
        };
        unsafe { libc::setrlimit(libc::RLIMIT_AS, &rlim); }
    }
}

/// Linux: set I/O scheduling to best-effort priority 4
#[cfg(target_os = "linux")]
fn apply_io_priority_low() {
    const IOPRIO_WHO_PROCESS: libc::c_int = 1;
    const IOPRIO_CLASS_BE: u32 = 2;
    let ioprio = (IOPRIO_CLASS_BE << 13) | 4;
    unsafe { libc::syscall(libc::SYS_ioprio_set, IOPRIO_WHO_PROCESS, 0, ioprio); }
}

/// Linux: set I/O scheduling to idle class
#[cfg(target_os = "linux")]
fn apply_io_priority_idle() {
    const IOPRIO_WHO_PROCESS: libc::c_int = 1;
    const IOPRIO_CLASS_IDLE: u32 = 3;
    let ioprio = IOPRIO_CLASS_IDLE << 13;
    unsafe { libc::syscall(libc::SYS_ioprio_set, IOPRIO_WHO_PROCESS, 0, ioprio); }
}

/// macOS: set I/O scheduling via setiopolicy_np
#[cfg(target_os = "macos")]
fn apply_io_priority_low() {
    extern "C" {
        fn setiopolicy_np(iotype: libc::c_int, scope: libc::c_int, policy: libc::c_int) -> libc::c_int;
    }
    // IOPOL_TYPE_DISK=0, IOPOL_SCOPE_PROCESS=0, IOPOL_UTILITY=1
    unsafe { setiopolicy_np(0, 0, 1); }
}

/// Non-unix stub (no-op)
#[cfg(not(unix))]
fn apply_resource_limits(_resources: &crate::ir::ResourceLimits) {}
