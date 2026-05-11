use std::collections::HashMap;
use crate::ir::{BesogneIR, ResolvedNode, ResolvedNativeNode};
use crate::probe::ProbeResult;
use crate::runtime::cache::CachedCommand;
use crate::runtime::cli::LogFormat;
use crate::tracer::{CommandResult, ProcessMetrics};

/// Why a probe result is being reported
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeStatus {
    /// Sealed at build time — embedded in the binary
    Sealed,
    /// Probed fresh this run
    Fresh,
    /// Reused from cache (not re-probed)
    Cached,
}

/// Context passed alongside command start — resolved paths and env values
pub struct CommandContext<'a> {
    /// Binary name → resolved absolute path (from build-phase inputs)
    pub binary_paths: &'a HashMap<String, String>,
    /// Binary name → resolved version (from build-phase inputs)
    pub binary_versions: &'a HashMap<String, String>,
    /// Env var name → value (secrets masked)
    pub env_vars: &'a HashMap<String, String>,
    /// Set of secret env var names
    pub secret_vars: &'a std::collections::HashSet<String>,
}

/// Output renderer trait — human, CI, JSON all implement this
pub trait OutputRenderer {
    fn on_start(&mut self, ir: &BesogneIR);
    fn on_phase_start(&mut self, phase: &str, count: usize);
    fn on_probe_result(&mut self, input: &ResolvedNode, result: &ProbeResult, status: ProbeStatus);
    fn on_phase_end(&mut self, phase: &str);
    fn on_command_start(&mut self, name: &str, exec: &[String], ctx: &CommandContext);
    fn on_command_output(&mut self, name: &str, stdout: &str, stderr: &str);
    fn on_command_end(&mut self, name: &str, result: &CommandResult);
    /// Replay a cached command (skipped because inputs unchanged)
    fn on_command_cached(&mut self, name: &str, exec: &[String], cached: &CachedCommand, ctx: &CommandContext);
    /// Warn about binaries/envs used at runtime but not declared in manifest
    fn on_undeclared_deps(&mut self, binaries: &[String], env_vars: &[String]);
    fn on_summary(&mut self, exit_code: i32, wall_ms: u64);
}

pub fn renderer_for_format(format: &LogFormat, verbose: bool) -> Box<dyn OutputRenderer> {
    match format {
        LogFormat::Human => Box::new(HumanRenderer::new(verbose)),
        LogFormat::Json => Box::new(JsonRenderer::new()),
        LogFormat::Ci => Box::new(CiRenderer::new()),
    }
}

fn input_label(input: &ResolvedNode) -> String {
    match &input.node {
        ResolvedNativeNode::Env { name, .. } => format!("env:{name}"),
        ResolvedNativeNode::File { path, .. } => format!("file:{path}"),
        ResolvedNativeNode::Binary { name, .. } => format!("binary:{name}"),
        ResolvedNativeNode::Service { tcp, http, .. } => {
            format!("service:{}", tcp.as_deref().or(http.as_deref()).unwrap_or("?"))
        }
        ResolvedNativeNode::User { in_group, .. } => {
            format!("user:{}", in_group.as_deref().unwrap_or("current"))
        }
        ResolvedNativeNode::Platform { os, arch, .. } => {
            format!("platform:{}-{}", os.as_deref().unwrap_or("?"), arch.as_deref().unwrap_or("?"))
        }
        ResolvedNativeNode::Dns { host, .. } => format!("dns:{host}"),
        ResolvedNativeNode::Metric { metric, .. } => format!("metric:{metric}"),
        ResolvedNativeNode::Command { name, .. } => format!("command:{name}"),
    }
}

fn probe_detail(input: &ResolvedNode, result: &ProbeResult) -> String {
    match &input.node {
        ResolvedNativeNode::Binary {
            name, resolved_version, resolved_path, source, ..
        } => {
            let ver = resolved_version.as_deref().unwrap_or("");
            let src = match source {
                Some(crate::ir::types::BinarySourceResolved::Nix { pname, .. }) =>
                    format!("nix:{}", pname.as_deref().unwrap_or(name)),
                Some(crate::ir::types::BinarySourceResolved::Mise { tool }) =>
                    format!("mise:{tool}"),
                Some(crate::ir::types::BinarySourceResolved::System) => "system".into(),
                None => String::new(),
            };
            let path = resolved_path.as_deref().unwrap_or("");
            let mut parts = vec![name.as_str()];
            if !ver.is_empty() { parts.push(ver); }
            if !src.is_empty() { parts.push(&src); }
            if !path.is_empty() { parts.push(path); }
            parts.join(" ")
        }

        ResolvedNativeNode::Env { name, secret, .. } => {
            if *secret {
                format!("{name}=***")
            } else if let Some(val) = result.variables.get(name.as_str()) {
                let display = if val.len() > 60 { format!("{}...", &val[..57]) } else { val.clone() };
                format!("{name}={display}")
            } else {
                name.clone()
            }
        }

        ResolvedNativeNode::File { path, expect, .. } => {
            if let Some(exp) = expect { format!("{path} ({exp})") } else { path.clone() }
        }

        ResolvedNativeNode::Service { name, tcp, http, .. } => {
            let label = name.as_deref().unwrap_or("service");
            let target = tcp.as_deref().or(http.as_deref()).unwrap_or("?");
            format!("{label} {target}")
        }

        ResolvedNativeNode::User { in_group, .. } => {
            if let Some(user) = result.variables.get("USER_NAME") {
                match in_group {
                    Some(g) => format!("{user} in:{g}"),
                    None => user.clone(),
                }
            } else {
                in_group.as_deref().unwrap_or("current").to_string()
            }
        }

        ResolvedNativeNode::Platform { .. } => {
            let os = result.variables.get("PLATFORM_OS").map(|s| s.as_str()).unwrap_or("?");
            let arch = result.variables.get("PLATFORM_ARCH").map(|s| s.as_str()).unwrap_or("?");
            format!("{os}/{arch}")
        }

        ResolvedNativeNode::Dns { host, .. } => {
            let key = format!("DNS_{}", host.to_uppercase().replace(['.', '-'], "_"));
            if let Some(ip) = result.variables.get(&key) { format!("{host} → {ip}") } else { host.clone() }
        }

        ResolvedNativeNode::Metric { metric, .. } => {
            let key = format!("METRIC_{}", metric.to_uppercase().replace('.', "_"));
            if let Some(val) = result.variables.get(&key) { format!("{metric}={val}") } else { metric.clone() }
        }

        ResolvedNativeNode::Command { name, .. } => name.clone(),
    }
}

fn status_label(status: ProbeStatus) -> &'static str {
    match status {
        ProbeStatus::Sealed => "sealed",
        ProbeStatus::Fresh => "fresh",
        ProbeStatus::Cached => "cached",
    }
}

fn format_bytes(bytes: u64) -> String {
    if bytes >= 1_073_741_824 {
        format!("{:.1}GB", bytes as f64 / 1_073_741_824.0)
    } else if bytes >= 1_048_576 {
        format!("{:.1}MB", bytes as f64 / 1_048_576.0)
    } else if bytes >= 1024 {
        format!("{}KB", bytes / 1024)
    } else {
        format!("{bytes}B")
    }
}

/// Shared metrics snapshot — used by both fresh and cached command rendering
struct Metrics {
    wall_ms: u64,
    user_ms: u64,
    sys_ms: u64,
    max_rss_kb: u64,
    disk_read_bytes: u64,
    disk_write_bytes: u64,
    net_read_bytes: u64,
    net_write_bytes: u64,
    processes_spawned: u64,
}

impl From<&CommandResult> for Metrics {
    fn from(r: &CommandResult) -> Self {
        Self {
            wall_ms: r.wall_ms,
            user_ms: r.user_ms,
            sys_ms: r.sys_ms,
            max_rss_kb: r.max_rss_kb,
            disk_read_bytes: r.disk_read_bytes,
            disk_write_bytes: r.disk_write_bytes,
            net_read_bytes: r.net_read_bytes,
            net_write_bytes: r.net_write_bytes,
            processes_spawned: r.processes_spawned,
        }
    }
}

impl From<&CachedCommand> for Metrics {
    fn from(c: &CachedCommand) -> Self {
        Self {
            wall_ms: c.wall_ms,
            user_ms: c.user_ms,
            sys_ms: c.sys_ms,
            max_rss_kb: c.max_rss_kb,
            disk_read_bytes: c.disk_read_bytes,
            disk_write_bytes: c.disk_write_bytes,
            net_read_bytes: c.net_read_bytes,
            net_write_bytes: c.net_write_bytes,
            processes_spawned: c.processes_spawned,
        }
    }
}

impl From<&ProcessMetrics> for Metrics {
    fn from(p: &ProcessMetrics) -> Self {
        Self {
            wall_ms: p.wall_ms,
            user_ms: p.user_ms,
            sys_ms: p.sys_ms,
            max_rss_kb: p.max_rss_kb,
            disk_read_bytes: p.read_bytes,
            disk_write_bytes: p.write_bytes,
            net_read_bytes: 0,
            net_write_bytes: 0,
            processes_spawned: 0,
        }
    }
}

fn format_metrics_human(m: &Metrics) -> String {
    let time = format!("\x1b[36m⏱ time:{:.3}s\x1b[0m", m.wall_ms as f64 / 1000.0);
    let cpu = if m.user_ms > 0 || m.sys_ms > 0 {
        let cores_used = if m.wall_ms > 0 {
            (m.user_ms + m.sys_ms) as f64 / m.wall_ms as f64
        } else { 0.0 };
        format!(
            "  \x1b[33m⚡ cpu:{:.2}s user + {:.2}s kernel ({:.1} cores)\x1b[0m",
            m.user_ms as f64 / 1000.0,
            m.sys_ms as f64 / 1000.0,
            cores_used,
        )
    } else { String::new() };
    let mem = if m.max_rss_kb > 0 {
        format!("  \x1b[35m🧠 memory:{}\x1b[0m", format_bytes(m.max_rss_kb * 1024))
    } else { String::new() };
    let disk = if m.disk_read_bytes > 0 || m.disk_write_bytes > 0 {
        format!("  \x1b[34m💾 ⬇ read:{} ⬆ write:{}\x1b[0m", format_bytes(m.disk_read_bytes), format_bytes(m.disk_write_bytes))
    } else { String::new() };
    let net = if m.net_read_bytes > 0 || m.net_write_bytes > 0 {
        format!("  \x1b[32m🌐 ⬇ download:{} ⬆ upload:{}\x1b[0m", format_bytes(m.net_read_bytes), format_bytes(m.net_write_bytes))
    } else { String::new() };
    let procs = if m.processes_spawned > 0 {
        format!("  \x1b[31m🔀 processes:{}\x1b[0m", m.processes_spawned + 1)
    } else { String::new() };
    format!("{time}{cpu}{mem}{disk}{net}{procs}")
}

fn format_metrics_ci(m: &Metrics) -> String {
    let time = format!("{:.3}s", m.wall_ms as f64 / 1000.0);
    let cpu = if m.user_ms > 0 || m.sys_ms > 0 {
        let cores_used = if m.wall_ms > 0 {
            (m.user_ms + m.sys_ms) as f64 / m.wall_ms as f64
        } else { 0.0 };
        format!(
            "  cpu:{:.2}s user + {:.2}s kernel ({:.1} cores)",
            m.user_ms as f64 / 1000.0,
            m.sys_ms as f64 / 1000.0,
            cores_used,
        )
    } else { String::new() };
    let mem = if m.max_rss_kb > 0 {
        format!("  memory:{}", format_bytes(m.max_rss_kb * 1024))
    } else { String::new() };
    let disk = if m.disk_read_bytes > 0 || m.disk_write_bytes > 0 {
        format!("  💾 ⬇ read:{} ⬆ write:{}", format_bytes(m.disk_read_bytes), format_bytes(m.disk_write_bytes))
    } else { String::new() };
    let net = if m.net_read_bytes > 0 || m.net_write_bytes > 0 {
        format!("  🌐 ⬇ download:{} ⬆ upload:{}", format_bytes(m.net_read_bytes), format_bytes(m.net_write_bytes))
    } else { String::new() };
    let procs = if m.processes_spawned > 0 {
        format!("  processes:{}", m.processes_spawned + 1)
    } else { String::new() };
    format!("{time}{cpu}{mem}{disk}{net}{procs}")
}

fn format_metrics_json(m: &Metrics) -> serde_json::Value {
    let cores_used = if m.wall_ms > 0 {
        (m.user_ms + m.sys_ms) as f64 / m.wall_ms as f64
    } else { 0.0 };
    serde_json::json!({
        "time_ms": m.wall_ms,
        "cpu_user_ms": m.user_ms,
        "cpu_kernel_ms": m.sys_ms,
        "cpu_cores_used": cores_used,
        "memory_bytes": m.max_rss_kb * 1024,
        "disk_read_bytes": m.disk_read_bytes,
        "disk_write_bytes": m.disk_write_bytes,
        "net_download_bytes": m.net_read_bytes,
        "net_upload_bytes": m.net_write_bytes,
        "processes": m.processes_spawned + 1,
    })
}

/// Compact per-process metrics line using the same style as command metrics.
/// Reuses format_bytes and the same color codes for consistency.
/// Crop a long path for display. Nix store paths get shortened:
/// `/nix/store/abc123...-nodejs-20.11.0/bin/node` → `/nix/store/abc123.../node`
/// Other long paths get tail-cropped.
fn crop_path(path: &str, max_len: usize) -> String {
    // Nix store paths are always shortened (hash is noise)
    if path.starts_with("/nix/store/") {
        let after_store = &path["/nix/store/".len()..];
        let hash_end = after_store.find('-').unwrap_or(8).min(8);
        let hash_prefix = &after_store[..hash_end];
        let basename = path.rsplit('/').next().unwrap_or(path);
        return format!("/nix/store/{hash_prefix}.../{basename}");
    }
    // Non-nix paths: only crop if too long
    if path.len() <= max_len { return path.to_string(); }
    let basename = path.rsplit('/').next().unwrap_or(path);
    if basename.len() + 6 >= max_len {
        return format!(".../{basename}");
    }
    let prefix_len = max_len - basename.len() - 4; // ".../"
    format!("{}.../{basename}", &path[..prefix_len])
}

/// Resolve exec args to their absolute paths where known.
fn resolve_exec_display(exec: &[String], ctx: &CommandContext) -> String {
    exec.iter().map(|arg| {
        ctx.binary_paths.get(arg.as_str()).cloned().unwrap_or_else(|| arg.clone())
    }).collect::<Vec<_>>().join(" ")
}

/// Display resolved binary paths and env vars under the exec line (human output).
fn format_command_context_human(exec: &[String], ctx: &CommandContext) {
    // Show resolved binary paths for args that match declared binaries
    for arg in exec {
        if let Some(path) = ctx.binary_paths.get(arg.as_str()) {
            if path != arg {
                let display_path = crop_path(path, 60);
                let version = ctx.binary_versions.get(arg.as_str())
                    .map(|v| format!(" \x1b[36mv{v}\x1b[0m"))
                    .unwrap_or_default();
                eprintln!("    \x1b[2;34m{arg} \x1b[0m\x1b[2m→ {display_path}\x1b[0m{version}");
            }
        }
    }
    // Show env vars (sorted, secrets masked)
    if !ctx.env_vars.is_empty() {
        let mut vars: Vec<_> = ctx.env_vars.iter().collect();
        vars.sort_by_key(|(k, _)| k.clone());
        let parts: Vec<String> = vars.iter().map(|(k, v)| {
            if ctx.secret_vars.contains(k.as_str()) {
                format!("\x1b[33m{k}\x1b[2m=*****\x1b[0m")
            } else {
                let display = if v.len() > 50 { format!("{}...", &v[..47]) } else { v.to_string() };
                format!("\x1b[2m{k}={display}\x1b[0m")
            }
        }).collect();
        // Print in rows of ~120 chars
        let mut line = String::from("    ");
        for (i, part) in parts.iter().enumerate() {
            if i > 0 { line.push_str("  "); }
            // Strip ANSI for length check (rough: each var ~20 visible chars)
            if line.len() > 140 && i > 0 {
                eprintln!("{line}");
                line = String::from("    ");
            }
            line.push_str(part);
        }
        if line.trim().len() > 0 {
            eprintln!("{line}");
        }
    }
}



/// Format process tree for human output — colored, indented per nesting level.
fn format_process_tree_human(tree: &[ProcessMetrics]) {
    if tree.len() <= 1 { return; }
    eprintln!(
        "    \x1b[2;36m┌ process tree\x1b[0m \x1b[36m({} processes)\x1b[0m",
        tree.len(),
    );
    // Build parent→children map (by pid)
    let mut children: HashMap<u32, Vec<usize>> = HashMap::new();
    for (i, p) in tree.iter().enumerate() {
        if i > 0 { // skip root from children map — we print it as the tree root
            children.entry(p.ppid).or_default().push(i);
        }
    }
    print_tree_node(tree, &children, 0, "    ", true);
}

/// Label for a process tree node: prefer cmdline (full path), fall back to comm, then pid.
/// Long paths (especially nix store paths) are shortened via `crop_path`.
fn process_label(p: &ProcessMetrics) -> String {
    if !p.cmdline.is_empty() {
        crop_cmdline(&p.cmdline, 80)
    } else if !p.comm.is_empty() {
        p.comm.clone()
    } else {
        format!("pid:{}", p.pid)
    }
}

/// Shorten all absolute paths in a command line string using `crop_path`.
fn crop_cmdline(cmdline: &str, max_path_len: usize) -> String {
    cmdline
        .split(' ')
        .map(|token| {
            if token.starts_with('/') {
                crop_path(token, max_path_len)
            } else {
                token.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn print_tree_node(
    tree: &[ProcessMetrics],
    children: &HashMap<u32, Vec<usize>>,
    idx: usize,
    prefix: &str,
    is_root: bool,
) {
    let p = &tree[idx];
    let label = process_label(p);
    let metrics = format_metrics_human(&Metrics::from(p));
    let exit_tag = if p.exit_code == 0 {
        "\x1b[32m0\x1b[0m".to_string()
    } else {
        format!("\x1b[31m{}\x1b[0m", p.exit_code)
    };

    if is_root {
        // Root process: special connector
        eprintln!("{prefix}\x1b[1;37m{label}\x1b[0m [{exit_tag}]  {metrics}");
    } else {
        eprintln!("{prefix}\x1b[37m{label}\x1b[0m [{exit_tag}]  {metrics}");
    }

    let kids = children.get(&p.pid);
    let kid_list = kids.map(|k| k.as_slice()).unwrap_or(&[]);
    for (i, &kid_idx) in kid_list.iter().enumerate() {
        let is_last = i == kid_list.len() - 1;
        let connector = if is_last { "└─ " } else { "├─ " };
        let child_prefix = if is_last {
            format!("{prefix}   ")
        } else {
            format!("{prefix}│  ")
        };
        let child_label = process_label(&tree[kid_idx]);
        let child_metrics = format_metrics_human(&Metrics::from(&tree[kid_idx]));
        let child_exit = if tree[kid_idx].exit_code == 0 {
            "\x1b[32m0\x1b[0m".to_string()
        } else {
            format!("\x1b[31m{}\x1b[0m", tree[kid_idx].exit_code)
        };
        eprintln!("{prefix}{connector}\x1b[37m{child_label}\x1b[0m [{child_exit}]  {child_metrics}");

        // Recurse for grandchildren
        if let Some(grandkids) = children.get(&tree[kid_idx].pid) {
            for (j, &gk_idx) in grandkids.iter().enumerate() {
                let gk_is_last = j == grandkids.len() - 1;
                let gk_connector = if gk_is_last { "└─ " } else { "├─ " };
                let gk_prefix = if gk_is_last {
                    format!("{child_prefix}   ")
                } else {
                    format!("{child_prefix}│  ")
                };
                print_tree_node_recursive(tree, children, gk_idx, &child_prefix, gk_connector, &gk_prefix);
            }
        }
    }
}

fn print_tree_node_recursive(
    tree: &[ProcessMetrics],
    children: &HashMap<u32, Vec<usize>>,
    idx: usize,
    parent_prefix: &str,
    connector: &str,
    my_prefix: &str,
) {
    let p = &tree[idx];
    let label = process_label(p);
    let metrics = format_metrics_human(&Metrics::from(p));
    let exit_tag = if p.exit_code == 0 {
        "\x1b[32m0\x1b[0m".to_string()
    } else {
        format!("\x1b[31m{}\x1b[0m", p.exit_code)
    };
    eprintln!("{parent_prefix}{connector}\x1b[37m{label}\x1b[0m [{exit_tag}]  {metrics}");

    if let Some(kids) = children.get(&p.pid) {
        for (i, &kid_idx) in kids.iter().enumerate() {
            let is_last = i == kids.len() - 1;
            let kid_connector = if is_last { "└─ " } else { "├─ " };
            let kid_prefix = if is_last {
                format!("{my_prefix}   ")
            } else {
                format!("{my_prefix}│  ")
            };
            print_tree_node_recursive(tree, children, kid_idx, my_prefix, kid_connector, &kid_prefix);
        }
    }
}

/// Format process tree for JSON output
fn format_process_tree_json(tree: &[ProcessMetrics]) -> serde_json::Value {
    tree.iter().map(|p| {
        let mut obj = serde_json::json!({
            "pid": p.pid,
            "ppid": p.ppid,
            "comm": p.comm,
            "cmdline": p.cmdline,
            "exit_code": p.exit_code,
            "wall_ms": p.wall_ms,
            "user_ms": p.user_ms,
            "sys_ms": p.sys_ms,
            "max_rss_kb": p.max_rss_kb,
            "read_bytes": p.read_bytes,
            "write_bytes": p.write_bytes,
            "voluntary_cs": p.voluntary_cs,
            "involuntary_cs": p.involuntary_cs,
        });
        obj
    }).collect()
}

// ── Human renderer ──────────────────────────────────────────────────

pub struct HumanRenderer {
    verbose: bool,
    cached_probes: usize,
    cached_commands: usize,
    /// Timestamp of the most recent cached command (for "nothing to do" message)
    last_cached_at: Option<String>,
}

impl HumanRenderer {
    pub fn new(verbose: bool) -> Self {
        Self { verbose, cached_probes: 0, cached_commands: 0, last_cached_at: None }
    }
}

impl OutputRenderer for HumanRenderer {
    fn on_start(&mut self, ir: &BesogneIR) {
        eprintln!(
            "\x1b[1m{}\x1b[0m v{} — {}",
            ir.metadata.name, ir.metadata.version, ir.metadata.description
        );
    }

    fn on_phase_start(&mut self, _phase: &str, _count: usize) {}

    fn on_probe_result(&mut self, input: &ResolvedNode, result: &ProbeResult, status: ProbeStatus) {
        // In non-verbose mode, silently count cached probes
        if !self.verbose && status == ProbeStatus::Cached {
            self.cached_probes += 1;
            return;
        }
        let detail = probe_detail(input, result);
        if result.success {
            let tag = match status {
                ProbeStatus::Sealed => "\x1b[34msealed\x1b[0m",
                ProbeStatus::Cached => "\x1b[33mcached\x1b[0m",
                ProbeStatus::Fresh  => "\x1b[32mok\x1b[0m",
            };
            eprintln!("  {tag} {detail}");
        } else {
            eprintln!(
                "  \x1b[31mfail\x1b[0m {detail} — {}",
                result.error.as_deref().unwrap_or("failed")
            );
        }
    }

    fn on_phase_end(&mut self, _phase: &str) {}

    fn on_command_start(&mut self, name: &str, exec: &[String], ctx: &CommandContext) {
        eprintln!("\n\x1b[1mexec\x1b[0m {name}: {}", exec.join(" "));
        // Always show context on fresh execution (not cached replay)
        format_command_context_human(exec, ctx);
    }

    fn on_command_output(&mut self, _name: &str, stdout: &str, stderr: &str) {
        for line in stdout.lines() { eprintln!("    {line}"); }
        for line in stderr.lines() { eprintln!("    {line}"); }
    }

    fn on_command_end(&mut self, name: &str, result: &CommandResult) {
        let metrics = format_metrics_human(&Metrics::from(result));
        if result.exit_code == 0 {
            eprintln!("  \x1b[32mok\x1b[0m {name}  {metrics}");
        } else {
            eprintln!("  \x1b[31mfail\x1b[0m {name}  exit {}  {metrics}", result.exit_code);
        }
        // Always show process tree on fresh execution
        format_process_tree_human(&result.process_tree);
    }

    fn on_command_cached(&mut self, name: &str, exec: &[String], cached: &CachedCommand, ctx: &CommandContext) {
        // Track the most recent cached timestamp for "nothing to do" message
        if self.last_cached_at.is_none() || self.last_cached_at.as_deref() < Some(&cached.ran_at) {
            self.last_cached_at = Some(cached.ran_at.clone());
        }
        if !self.verbose {
            self.cached_commands += 1;
            return;
        }
        eprintln!(
            "\n\x1b[33mcached\x1b[0m {name}: {}  \x1b[2m(ran {})\x1b[0m",
            resolve_exec_display(exec, ctx),
            format_relative_time(&cached.ran_at),
        );
        format_command_context_human(exec, ctx);
        let stdout = &cached.stdout;
        let stderr = &cached.stderr;
        if !stdout.is_empty() || !stderr.is_empty() {
            for line in stdout.lines() {
                eprintln!("    \x1b[2m{line}\x1b[0m");
            }
            for line in stderr.lines() {
                eprintln!("    \x1b[2m{line}\x1b[0m");
            }
        }
        let metrics = format_metrics_human(&Metrics::from(cached));
        eprintln!("  \x1b[33mcached\x1b[0m {name}  {metrics}");
        format_process_tree_human(&cached.process_tree);
    }

    fn on_undeclared_deps(&mut self, binaries: &[String], env_vars: &[String]) {
        if binaries.is_empty() && env_vars.is_empty() { return; }
        eprintln!("\n\x1b[33;1mwarning: undeclared dependencies detected at runtime\x1b[0m");
        eprintln!("  \x1b[2mCache disabled until manifest is updated.\x1b[0m");
        if !binaries.is_empty() {
            eprintln!("  \x1b[33mbinaries:\x1b[0m {}", binaries.join(", "));
        }
        if !env_vars.is_empty() {
            eprintln!("  \x1b[33menv vars:\x1b[0m {}", env_vars.join(", "));
        }
    }

    fn on_summary(&mut self, exit_code: i32, wall_ms: u64) {
        let total_cached = self.cached_probes + self.cached_commands;
        if !self.verbose && self.cached_commands > 0 && exit_code == 0 {
            let ago = self.last_cached_at.as_deref()
                .map(|t| format!(", ran {}", format_relative_time(t)))
                .unwrap_or_default();
            eprintln!(
                "\x1b[32mnothing to do\x1b[0m \x1b[2m({total_cached} nodes cached{ago}, use --status to show last run)\x1b[0m",
            );
        } else if exit_code == 0 {
            eprintln!("\n\x1b[32mdone\x1b[0m {:.3}s", wall_ms as f64 / 1000.0);
        } else {
            eprintln!("\n\x1b[31mfailed\x1b[0m exit {exit_code}  {:.3}s", wall_ms as f64 / 1000.0);
        }
    }
}

/// Format an ISO timestamp as a relative time string (e.g., "2m ago", "1h ago")
fn format_relative_time(iso_time: &str) -> String {
    let Ok(then) = chrono::DateTime::parse_from_rfc3339(iso_time) else {
        return iso_time.to_string();
    };
    let now = chrono::Utc::now();
    let delta = now.signed_duration_since(then);

    if delta.num_seconds() < 5 {
        "just now".to_string()
    } else if delta.num_seconds() < 60 {
        format!("{}s ago", delta.num_seconds())
    } else if delta.num_minutes() < 60 {
        format!("{}m ago", delta.num_minutes())
    } else if delta.num_hours() < 24 {
        format!("{}h ago", delta.num_hours())
    } else {
        format!("{}d ago", delta.num_days())
    }
}

// ── JSON renderer ───────────────────────────────────────────────────

pub struct JsonRenderer;

impl JsonRenderer {
    pub fn new() -> Self { Self }
    fn emit(&self, event: &serde_json::Value) {
        println!("{}", serde_json::to_string(event).unwrap_or_default());
    }
}

impl OutputRenderer for JsonRenderer {
    fn on_start(&mut self, ir: &BesogneIR) {
        self.emit(&serde_json::json!({
            "event": "start",
            "name": ir.metadata.name,
            "version": ir.metadata.version,
            "description": ir.metadata.description,
        }));
    }

    fn on_phase_start(&mut self, phase: &str, count: usize) {
        self.emit(&serde_json::json!({
            "event": "phase_start",
            "phase": phase,
            "input_count": count,
        }));
    }

    fn on_probe_result(&mut self, input: &ResolvedNode, result: &ProbeResult, status: ProbeStatus) {
        let phase = format!("{:?}", input.phase).to_lowercase();
        self.emit(&serde_json::json!({
            "event": "probe",
            "input": input_label(input),
            "phase": phase,
            "status": status_label(status),
            "success": result.success,
            "hash": result.hash,
            "error": result.error,
        }));
    }

    fn on_phase_end(&mut self, phase: &str) {
        self.emit(&serde_json::json!({"event": "phase_end", "phase": phase}));
    }



    fn on_command_start(&mut self, name: &str, exec: &[String], ctx: &CommandContext) {
        // Resolve exec args to full paths where known
        let resolved: Vec<String> = exec.iter().map(|arg| {
            ctx.binary_paths.get(arg.as_str()).cloned().unwrap_or_else(|| arg.clone())
        }).collect();
        let env_display: HashMap<&str, &str> = ctx.env_vars.iter()
            .map(|(k, v)| {
                if ctx.secret_vars.contains(k.as_str()) { (k.as_str(), "*****") } else { (k.as_str(), v.as_str()) }
            }).collect();
        self.emit(&serde_json::json!({
            "event": "command_start",
            "phase": "exec",
            "name": name,
            "exec": exec,
            "resolved_exec": resolved,
            "env": env_display,
        }));
    }

    fn on_command_output(&mut self, name: &str, stdout: &str, stderr: &str) {
        if !stdout.is_empty() || !stderr.is_empty() {
            self.emit(&serde_json::json!({
                "event": "command_output",
                "name": name,
                "stdout": stdout,
                "stderr": stderr,
            }));
        }
    }

    fn on_command_end(&mut self, name: &str, result: &CommandResult) {
        let mut obj = serde_json::json!({
            "event": "command_end",
            "name": name,
            "exit_code": result.exit_code,
        });
        let metrics = format_metrics_json(&Metrics::from(result));
        obj.as_object_mut().unwrap().extend(metrics.as_object().unwrap().clone());
        self.emit(&obj);
    }

    fn on_command_cached(&mut self, name: &str, exec: &[String], cached: &CachedCommand, _ctx: &CommandContext) {
        let mut obj = serde_json::json!({
            "event": "command_cached",
            "name": name,
            "exec": exec,
            "exit_code": cached.exit_code,
            "ran_at": cached.ran_at,
            "stdout": cached.stdout,
            "stderr": cached.stderr,
        });
        let metrics = format_metrics_json(&Metrics::from(cached));
        obj.as_object_mut().unwrap().extend(metrics.as_object().unwrap().clone());
        if !cached.process_tree.is_empty() {
            obj.as_object_mut().unwrap().insert(
                "process_tree".to_string(),
                format_process_tree_json(&cached.process_tree),
            );
        }
        self.emit(&obj);
    }

    fn on_undeclared_deps(&mut self, binaries: &[String], env_vars: &[String]) {
        if binaries.is_empty() && env_vars.is_empty() { return; }
        self.emit(&serde_json::json!({
            "event": "undeclared_deps",
            "binaries": binaries,
            "env_vars": env_vars,
        }));
    }

    fn on_summary(&mut self, exit_code: i32, wall_ms: u64) {
        self.emit(&serde_json::json!({
            "event": "summary",
            "exit_code": exit_code,
            "time_ms": wall_ms,
        }));
    }
}

// ── CI renderer ─────────────────────────────────────────────────────

pub struct CiRenderer;

impl CiRenderer {
    pub fn new() -> Self { Self }
}

impl OutputRenderer for CiRenderer {
    fn on_start(&mut self, ir: &BesogneIR) {
        eprintln!("::group::{} v{} — {}", ir.metadata.name, ir.metadata.version, ir.metadata.description);
    }

    fn on_phase_start(&mut self, _phase: &str, _count: usize) {}

    fn on_probe_result(&mut self, input: &ResolvedNode, result: &ProbeResult, status: ProbeStatus) {
        let detail = probe_detail(input, result);
        let tag = match status {
            ProbeStatus::Sealed => "[SEAL]",
            ProbeStatus::Cached => "[CACHE]",
            ProbeStatus::Fresh if result.success => "[PASS]",
            ProbeStatus::Fresh => "[FAIL]",
        };
        if result.success {
            eprintln!("  {tag} {detail}");
        } else {
            eprintln!("  {tag} {detail} — {}", result.error.as_deref().unwrap_or("failed"));
        }
    }

    fn on_phase_end(&mut self, _phase: &str) {
        eprintln!("::endgroup::");
    }

    fn on_command_start(&mut self, name: &str, exec: &[String], _ctx: &CommandContext) {
        eprintln!("::group::{name}: {}", exec.join(" "));
    }

    fn on_command_output(&mut self, _name: &str, stdout: &str, stderr: &str) {
        for line in stdout.lines() { eprintln!("    {line}"); }
        for line in stderr.lines() { eprintln!("    {line}"); }
    }

    fn on_command_end(&mut self, name: &str, result: &CommandResult) {
        let metrics = format_metrics_ci(&Metrics::from(result));
        if result.exit_code == 0 {
            eprintln!("  [PASS] {name}  {metrics}");
        } else {
            eprintln!("  [FAIL] {name}  exit {}  {metrics}", result.exit_code);
        }
        if result.process_tree.len() > 1 {
            eprintln!("  process tree ({} processes):", result.process_tree.len());
            for p in &result.process_tree {
                let label = process_label(p);
                eprintln!("    {label}  {}", format_metrics_ci(&Metrics::from(p)));
            }
        }
        eprintln!("::endgroup::");
    }

    fn on_command_cached(&mut self, name: &str, exec: &[String], cached: &CachedCommand, _ctx: &CommandContext) {
        eprintln!("::group::[CACHE] {name}: {} (ran {})", exec.join(" "), cached.ran_at);
        if !cached.stdout.is_empty() {
            for line in cached.stdout.lines() { eprintln!("    {line}"); }
        }
        if !cached.stderr.is_empty() {
            for line in cached.stderr.lines() { eprintln!("    {line}"); }
        }
        let metrics = format_metrics_ci(&Metrics::from(cached));
        eprintln!("  [CACHE] {name}  {metrics}");
        if cached.process_tree.len() > 1 {
            eprintln!("  process tree ({} processes):", cached.process_tree.len());
            for p in &cached.process_tree {
                let label = process_label(p);
                eprintln!("    {label}  {}", format_metrics_ci(&Metrics::from(p)));
            }
        }
        eprintln!("::endgroup::");
    }

    fn on_undeclared_deps(&mut self, binaries: &[String], env_vars: &[String]) {
        if binaries.is_empty() && env_vars.is_empty() { return; }
        eprintln!("::warning::Undeclared runtime dependencies — cache disabled");
        if !binaries.is_empty() { eprintln!("  binaries: {}", binaries.join(", ")); }
        if !env_vars.is_empty() { eprintln!("  env vars: {}", env_vars.join(", ")); }
    }

    fn on_summary(&mut self, exit_code: i32, wall_ms: u64) {
        if exit_code == 0 {
            eprintln!("::notice::PASS {:.3}s", wall_ms as f64 / 1000.0);
        } else {
            eprintln!("::error::FAILED exit {exit_code}  {:.3}s", wall_ms as f64 / 1000.0);
        }
    }
}
