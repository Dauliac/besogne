/// Process tracing — collects metrics from command execution.
///
/// Uses fork + exec + wait4(rusage) for per-process CPU, memory, I/O metrics.
/// On Linux, uses PR_SET_CHILD_SUBREAPER to adopt all orphaned descendants,
/// then reaps each one individually with wait4(-1) to get per-process rusage.
/// A background /proc scanner captures command names and ppid for tree structure.

use std::collections::HashMap;
use std::io::Read;

/// Metadata collected from Docker for container processes
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ContainerMetadata {
    pub container_id: String,
    pub image: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub container_name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub status: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ports: Vec<String>,
}

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
    /// Container metadata if this process is a `docker run` invocation
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub container: Option<ContainerMetadata>,
}

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
    /// Per-process metrics for the entire process tree (root + all descendants)
    pub process_tree: Vec<ProcessMetrics>,
    /// Containers created during command execution (detected via Docker API diff)
    pub containers: Vec<ContainerMetadata>,
}

/// Execute a command with wait4 for rusage metrics collection
pub fn execute_traced(
    args: &[String],
    env: &HashMap<String, String>,
    env_isolation: &crate::ir::EnvSandboxResolved,
    workdir: Option<&str>,
) -> Result<CommandResult, String> {
    if args.is_empty() {
        return Err("empty command".into());
    }

    #[cfg(target_os = "linux")]
    {
        execute_with_wait4(args, env, env_isolation, workdir)
    }

    #[cfg(not(target_os = "linux"))]
    {
        execute_simple(args, env, env_isolation, workdir)
    }
}

/// Per-PID metadata captured by the /proc scanner
#[cfg(target_os = "linux")]
#[derive(Debug, Clone)]
struct ProcSnapshot {
    pid: u32,
    ppid: u32,
    comm: String,
    cmdline: String,
    read_bytes: u64,
    write_bytes: u64,
    first_seen: std::time::Instant,
    last_seen: std::time::Instant,
    container: Option<ContainerMetadata>,
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

/// Collect all descendant PIDs of a given root PID by walking /proc.
#[cfg(target_os = "linux")]
fn collect_descendants(root_pid: i32) -> std::collections::HashSet<u32> {
    let mut descendants = std::collections::HashSet::new();
    let Ok(entries) = std::fs::read_dir("/proc") else { return descendants };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name_str) = name.to_str() else { continue };
        let Ok(pid) = name_str.parse::<u32>() else { continue };
        if pid == root_pid as u32 { continue; }
        let mut cur = pid;
        for _ in 0..64 {
            let ppid = read_proc_ppid(cur);
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

/// Parse a `docker run` cmdline and extract the image name.
/// Handles: `docker run [OPTIONS] IMAGE [COMMAND] [ARG...]`
#[cfg(target_os = "linux")]
fn parse_docker_run_image(cmdline: &str) -> Option<String> {
    let parts: Vec<&str> = cmdline.split_whitespace().collect();
    // Find "run" after "docker"
    let run_idx = parts.iter().position(|&p| p == "run")?;
    // Skip flags after "run" to find the image
    let mut i = run_idx + 1;
    while i < parts.len() {
        let arg = parts[i];
        if arg == "--" {
            // Everything after -- is the command, image was before
            return None;
        }
        if arg.starts_with('-') {
            // Flags that take a value (common docker run flags)
            let takes_value = matches!(arg,
                "-e" | "--env" | "-v" | "--volume" | "-p" | "--publish" |
                "-w" | "--workdir" | "--name" | "--network" | "--entrypoint" |
                "-u" | "--user" | "--memory" | "--cpus" | "-l" | "--label" |
                "--mount" | "--platform" | "--restart" | "--hostname" |
                "--add-host" | "--cap-add" | "--cap-drop" | "--device" |
                "--tmpfs" | "--env-file" | "--log-driver" | "--log-opt" |
                "--pid" | "--ipc" | "--uts" | "--cgroupns" | "--shm-size"
            );
            if takes_value && !arg.contains('=') {
                i += 2; // skip flag + value
            } else {
                i += 1; // skip flag (boolean or --flag=value)
            }
        } else {
            // First non-flag arg after `run` is the image
            return Some(arg.to_string());
        }
    }
    None
}

/// Query Docker daemon via Unix socket for running containers.
/// Returns a list of (container_id_short, image, name, status, ports).
#[cfg(target_os = "linux")]
fn query_docker_containers() -> Vec<ContainerMetadata> {
    use std::io::Write;
    use std::os::unix::net::UnixStream;

    let sock_path = std::env::var("DOCKER_HOST")
        .unwrap_or_else(|_| "/var/run/docker.sock".to_string())
        .strip_prefix("unix://")
        .map(|s| s.to_string())
        .unwrap_or_else(|| "/var/run/docker.sock".to_string());

    let Ok(mut stream) = UnixStream::connect(&sock_path) else {
        return Vec::new();
    };
    let _ = stream.set_read_timeout(Some(std::time::Duration::from_millis(500)));
    let _ = stream.set_write_timeout(Some(std::time::Duration::from_millis(200)));

    let request = "GET /containers/json HTTP/1.0\r\nHost: localhost\r\n\r\n";
    if stream.write_all(request.as_bytes()).is_err() {
        return Vec::new();
    }

    let mut response = Vec::new();
    let _ = stream.read_to_end(&mut response);
    let response_str = String::from_utf8_lossy(&response);

    // Find JSON body after \r\n\r\n
    let Some(body_start) = response_str.find("\r\n\r\n") else {
        return Vec::new();
    };
    let body = &response_str[body_start + 4..];

    let Ok(containers) = serde_json::from_str::<Vec<serde_json::Value>>(body) else {
        return Vec::new();
    };

    containers.iter().filter_map(|c| {
        let id = c.get("Id")?.as_str()?;
        let image = c.get("Image")?.as_str()?;
        let names: Vec<String> = c.get("Names")
            .and_then(|n| n.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str().map(|s| s.trim_start_matches('/').to_string())).collect())
            .unwrap_or_default();
        let status = c.get("Status").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let ports: Vec<String> = c.get("Ports")
            .and_then(|p| p.as_array())
            .map(|arr| arr.iter().filter_map(|p| {
                let private = p.get("PrivatePort")?.as_u64()?;
                let public = p.get("PublicPort").and_then(|v| v.as_u64());
                let proto = p.get("Type").and_then(|v| v.as_str()).unwrap_or("tcp");
                match public {
                    Some(pub_port) => Some(format!("{pub_port}->{private}/{proto}")),
                    None => Some(format!("{private}/{proto}")),
                }
            }).collect())
            .unwrap_or_default();

        Some(ContainerMetadata {
            container_id: id[..12.min(id.len())].to_string(),
            image: image.to_string(),
            container_name: names.first().cloned().unwrap_or_default(),
            status,
            ports,
        })
    }).collect()
}

/// Try to match a docker run process to a running container by image name.
#[cfg(target_os = "linux")]
fn detect_container_for_process(cmdline: &str, containers: &[ContainerMetadata]) -> Option<ContainerMetadata> {
    let image = parse_docker_run_image(cmdline)?;
    // Match by image name (exact or tag-stripped)
    containers.iter().find(|c| {
        c.image == image
            || c.image.starts_with(&format!("{image}:"))
            || c.image.split(':').next() == Some(&image)
            // Docker may resolve to full registry path
            || c.image.ends_with(&format!("/{image}"))
            || c.image.contains(&image)
    }).cloned()
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
        container: snapshot.and_then(|s| s.container.clone()),
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
) -> Result<CommandResult, String> {
    use std::os::unix::io::FromRawFd;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Instant;

    let start = Instant::now();
    let (net_rx_before, net_tx_before) = read_net_dev();

    // Snapshot Docker containers before execution to detect newly created ones
    let containers_before: std::collections::HashSet<String> =
        query_docker_containers().into_iter().map(|c| c.container_id).collect();

    // Become subreaper: all orphaned descendants will be reparented to us
    // so we can wait4 them and collect their rusage.
    unsafe { libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 1, 0, 0, 0); }

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
            let mut docker_queried = false;
            let mut docker_containers: Vec<ContainerMetadata> = Vec::new();
            let mut scan_count = 0u32;
            while !stop.load(Ordering::Relaxed) {
                let now = Instant::now();
                let found = collect_descendants(pid);

                if let Ok(mut m) = metrics.lock() {
                    snapshot_pid(&mut m, root_pid, now);
                    for &dpid in &found {
                        snapshot_pid(&mut m, dpid, now);
                    }

                    // Detect docker run processes and query Docker API for metadata.
                    // Query Docker every ~50 scans (~150ms) to avoid hammering the socket.
                    let has_docker_proc = m.snapshots.values().any(|s| {
                        s.container.is_none() && s.cmdline.contains("docker") && s.cmdline.contains(" run ")
                    });
                    if has_docker_proc && (!docker_queried || scan_count % 50 == 0) {
                        docker_containers = query_docker_containers();
                        docker_queried = true;
                    }

                    // Match docker run processes to containers
                    if !docker_containers.is_empty() {
                        let pids_to_update: Vec<u32> = m.snapshots.iter()
                            .filter(|(_, s)| s.container.is_none() && s.cmdline.contains("docker") && s.cmdline.contains(" run "))
                            .map(|(&pid, _)| pid)
                            .collect();
                        for dpid in pids_to_update {
                            if let Some(snap) = m.snapshots.get_mut(&dpid) {
                                snap.container = detect_container_for_process(&snap.cmdline, &docker_containers);
                            }
                        }
                    }
                }
                scan_count = scan_count.wrapping_add(1);
                std::thread::sleep(std::time::Duration::from_millis(3));
            }
        })
    };

    // Read stdout and stderr
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    {
        let mut stdout_file = unsafe { std::fs::File::from_raw_fd(stdout_read) };
        let mut stderr_file = unsafe { std::fs::File::from_raw_fd(stderr_read) };
        let _ = stdout_file.read_to_end(&mut stdout);
        let _ = stderr_file.read_to_end(&mut stderr);
    }

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
        return Err(format!("wait4 failed: {}", std::io::Error::last_os_error()));
    }

    let main_exit = extract_exit_code(status);

    // Build ProcessMetrics for the root child
    let root_snapshot = metrics.lock().ok().and_then(|m| m.snapshots.get(&(pid as u32)).cloned());
    let root_container = root_snapshot.as_ref().and_then(|s| s.container.clone());
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
        container: root_container,
        ..rusage_to_metrics(pid as u32, &rusage, main_exit, root_snapshot.as_ref(), main_wall_ms)
    }];

    // Reap all orphaned descendants (adopted via PR_SET_CHILD_SUBREAPER)
    loop {
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

    // Disable subreaper
    unsafe { libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 0, 0, 0, 0); }

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
                container: snap.container.clone(),
            });
        }
    }

    // Detect containers created during execution by diffing Docker state.
    // This catches containers spawned via Docker API (e.g. testcontainers-go)
    // that don't appear as `docker run` subprocesses.
    let mut containers: Vec<ContainerMetadata> = process_tree.iter()
        .filter_map(|p| p.container.clone())
        .collect();
    let process_container_ids: std::collections::HashSet<String> =
        containers.iter().map(|c| c.container_id.clone()).collect();
    let containers_after = query_docker_containers();
    for c in containers_after {
        if !containers_before.contains(&c.container_id) && !process_container_ids.contains(&c.container_id) {
            containers.push(c);
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
        containers,
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
        container: None,
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
) -> Result<CommandResult, String> {
    use std::process::{Command, Stdio};
    use std::time::Instant;

    let start = Instant::now();

    let mut cmd = Command::new(&args[0]);
    cmd.args(&args[1..]);
    if let Some(dir) = workdir {
        cmd.current_dir(dir);
    }

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
        process_tree: vec![],
        containers: vec![],
    })
}
