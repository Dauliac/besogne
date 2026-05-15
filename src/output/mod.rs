pub mod style;
pub mod views;

use std::collections::{HashMap, HashSet};
use crate::ir::{BesogneIR, ResolvedNode, ResolvedNativeNode, ContentId};
use crate::manifest::Phase;
use crate::probe::ProbeResult;
use crate::runtime::cache::{CachedCommand, ContextCache};
use crate::runtime::cli::LogFormat;
use crate::tracer::{CommandResult, ProcessMetrics};
use termtree::Tree;

use style::{styled, dim, bold, exit_code as fmt_exit, palette::RESET};
use style::{status, node, telemetry, phase, weight, diagnostic};
use style::{label, badge, phase_label, metric_label, message};

/// Why a probe result is being reported
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeStatus {
    /// Build-phase: frozen at build time, embedded in binary
    Pinned,
    /// Cached: replaying sealed result from previous run
    Sealed,
    /// Freshly probed/verified just now
    Probed,
    /// Probe failed
    Failed,
    /// Node skipped (on_missing: skip) — children will be skipped too
    Skipped,
}

/// Context passed alongside command start — resolved paths and env values
#[derive(Debug)]
pub struct CommandContext<'a> {
    pub binary_paths: &'a HashMap<String, String>,
    pub binary_versions: &'a HashMap<String, String>,
    pub env_vars: &'a HashMap<String, String>,
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
    fn on_command_cached(&mut self, name: &str, exec: &[String], cached: &CachedCommand, ctx: &CommandContext);
    fn on_build_pinned_summary(&mut self, nodes: &[&ResolvedNode]);
    fn on_changed_probes(&mut self, changed: &[String]);
    fn on_skip(&mut self, total_nodes: usize, ran_at: &str, duration_ms: u64);
    fn on_undeclared_deps(&mut self, binaries: &[String], env_vars: &[String]);
    fn on_summary(&mut self, exit_code: i32, wall_ms: u64);
}

/// Enum dispatch for renderers — compile-time monomorphized, no vtable.
pub enum Renderer {
    Human(HumanRenderer),
    Json(JsonRenderer),
    Ci(CiRenderer),
}

/// Delegate every OutputRenderer method to the inner variant.
macro_rules! delegate_renderer {
    ($self:ident, $method:ident ( $($arg:ident),* )) => {
        match $self {
            Renderer::Human(r) => r.$method($($arg),*),
            Renderer::Json(r) => r.$method($($arg),*),
            Renderer::Ci(r) => r.$method($($arg),*),
        }
    };
}

impl OutputRenderer for Renderer {
    fn on_start(&mut self, ir: &BesogneIR) { delegate_renderer!(self, on_start(ir)); }
    fn on_phase_start(&mut self, phase: &str, count: usize) { delegate_renderer!(self, on_phase_start(phase, count)); }
    fn on_probe_result(&mut self, input: &ResolvedNode, result: &ProbeResult, status: ProbeStatus) { delegate_renderer!(self, on_probe_result(input, result, status)); }
    fn on_phase_end(&mut self, phase: &str) { delegate_renderer!(self, on_phase_end(phase)); }
    fn on_command_start(&mut self, name: &str, exec: &[String], ctx: &CommandContext) { delegate_renderer!(self, on_command_start(name, exec, ctx)); }
    fn on_command_output(&mut self, name: &str, stdout: &str, stderr: &str) { delegate_renderer!(self, on_command_output(name, stdout, stderr)); }
    fn on_command_end(&mut self, name: &str, result: &CommandResult) { delegate_renderer!(self, on_command_end(name, result)); }
    fn on_command_cached(&mut self, name: &str, exec: &[String], cached: &CachedCommand, ctx: &CommandContext) { delegate_renderer!(self, on_command_cached(name, exec, cached, ctx)); }
    fn on_build_pinned_summary(&mut self, nodes: &[&ResolvedNode]) { delegate_renderer!(self, on_build_pinned_summary(nodes)); }
    fn on_changed_probes(&mut self, changed: &[String]) { delegate_renderer!(self, on_changed_probes(changed)); }
    fn on_skip(&mut self, total_nodes: usize, ran_at: &str, duration_ms: u64) { delegate_renderer!(self, on_skip(total_nodes, ran_at, duration_ms)); }
    fn on_undeclared_deps(&mut self, binaries: &[String], env_vars: &[String]) { delegate_renderer!(self, on_undeclared_deps(binaries, env_vars)); }
    fn on_summary(&mut self, exit_code: i32, wall_ms: u64) { delegate_renderer!(self, on_summary(exit_code, wall_ms)); }
}

pub fn renderer_for_format(format: &LogFormat, verbose: bool) -> Renderer {
    match format {
        LogFormat::Human => Renderer::Human(HumanRenderer::new(verbose)),
        LogFormat::Json => Renderer::Json(JsonRenderer::new()),
        LogFormat::Ci => Renderer::Ci(CiRenderer::new()),
    }
}

// Bridge: EventHandler → OutputRenderer dispatch.
// This allows the existing Renderer to be used as an EventHandler.
impl crate::event::EventHandler for Renderer {
    fn on_event(&mut self, event: &crate::event::BesogneEvent<'_>) {
        use crate::event::BesogneEvent;
        match event {
            BesogneEvent::Start { ir } => self.on_start(ir),
            BesogneEvent::PhaseStart { phase, node_count } => self.on_phase_start(phase, *node_count),
            BesogneEvent::ProbeResult { node, result, status } => self.on_probe_result(node, result, *status),
            BesogneEvent::PhaseEnd { phase } => self.on_phase_end(phase),
            BesogneEvent::CommandStart { name, exec, ctx } => self.on_command_start(name, exec, ctx),
            BesogneEvent::CommandOutput { name, stdout, stderr } => self.on_command_output(name, stdout, stderr),
            BesogneEvent::CommandEnd { name, result, .. } => self.on_command_end(name, result),
            BesogneEvent::CommandCached { name, exec, cached, ctx } => self.on_command_cached(name, exec, cached, ctx),
            BesogneEvent::BuildPinnedSummary { nodes } => self.on_build_pinned_summary(nodes),
            BesogneEvent::ChangedProbes { names } => self.on_changed_probes(names),
            BesogneEvent::Skip { total_nodes, ran_at, duration_ms } => self.on_skip(*total_nodes, ran_at, *duration_ms),
            BesogneEvent::UndeclaredDeps { binaries, env_vars } => self.on_undeclared_deps(binaries, env_vars),
            BesogneEvent::Summary { exit_code, wall_ms } => self.on_summary(*exit_code, *wall_ms),
        }
    }
}

/// Composite emitter: renders output AND emits structured events to an external handler.
///
/// When no handler is set, behaves identically to `Renderer`.
/// Use [`EventEmitter::with_handler`] to attach a custom [`crate::event::EventHandler`].
pub struct EventEmitter {
    renderer: Renderer,
    handler: Option<Box<dyn crate::event::EventHandler>>,
}

impl EventEmitter {
    /// Create an emitter with only the built-in renderer (no external handler).
    pub fn new(renderer: Renderer) -> Self {
        Self { renderer, handler: None }
    }

    /// Create an emitter with a built-in renderer and an external event handler.
    pub fn with_handler(renderer: Renderer, handler: Box<dyn crate::event::EventHandler>) -> Self {
        Self { renderer, handler: Some(handler) }
    }

    fn emit(&mut self, event: &crate::event::BesogneEvent<'_>) {
        if let Some(h) = &mut self.handler {
            h.on_event(event);
        }
    }
}

impl OutputRenderer for EventEmitter {
    fn on_start(&mut self, ir: &BesogneIR) {
        self.renderer.on_start(ir);
        self.emit(&crate::event::BesogneEvent::Start { ir });
    }
    fn on_phase_start(&mut self, phase: &str, count: usize) {
        self.renderer.on_phase_start(phase, count);
        self.emit(&crate::event::BesogneEvent::PhaseStart { phase, node_count: count });
    }
    fn on_probe_result(&mut self, input: &ResolvedNode, result: &ProbeResult, status: ProbeStatus) {
        self.renderer.on_probe_result(input, result, status);
        self.emit(&crate::event::BesogneEvent::ProbeResult { node: input, result, status });
    }
    fn on_phase_end(&mut self, phase: &str) {
        self.renderer.on_phase_end(phase);
        self.emit(&crate::event::BesogneEvent::PhaseEnd { phase });
    }
    fn on_command_start(&mut self, name: &str, exec: &[String], ctx: &CommandContext) {
        self.renderer.on_command_start(name, exec, ctx);
        self.emit(&crate::event::BesogneEvent::CommandStart { name, exec, ctx });
    }
    fn on_command_output(&mut self, name: &str, stdout: &str, stderr: &str) {
        self.renderer.on_command_output(name, stdout, stderr);
        self.emit(&crate::event::BesogneEvent::CommandOutput { name, stdout, stderr });
    }
    fn on_command_end(&mut self, name: &str, result: &CommandResult) {
        self.renderer.on_command_end(name, result);
        self.emit(&crate::event::BesogneEvent::CommandEnd {
            name,
            exit_code: result.exit_code,
            wall_ms: result.wall_ms,
            result,
        });
    }
    fn on_command_cached(&mut self, name: &str, exec: &[String], cached: &CachedCommand, ctx: &CommandContext) {
        self.renderer.on_command_cached(name, exec, cached, ctx);
        self.emit(&crate::event::BesogneEvent::CommandCached { name, exec, cached, ctx });
    }
    fn on_build_pinned_summary(&mut self, nodes: &[&ResolvedNode]) {
        self.renderer.on_build_pinned_summary(nodes);
        self.emit(&crate::event::BesogneEvent::BuildPinnedSummary { nodes });
    }
    fn on_changed_probes(&mut self, changed: &[String]) {
        self.renderer.on_changed_probes(changed);
        self.emit(&crate::event::BesogneEvent::ChangedProbes { names: changed });
    }
    fn on_skip(&mut self, total_nodes: usize, ran_at: &str, duration_ms: u64) {
        self.renderer.on_skip(total_nodes, ran_at, duration_ms);
        self.emit(&crate::event::BesogneEvent::Skip { total_nodes, ran_at, duration_ms });
    }
    fn on_undeclared_deps(&mut self, binaries: &[String], env_vars: &[String]) {
        self.renderer.on_undeclared_deps(binaries, env_vars);
        self.emit(&crate::event::BesogneEvent::UndeclaredDeps { binaries, env_vars });
    }
    fn on_summary(&mut self, exit_code: i32, wall_ms: u64) {
        self.renderer.on_summary(exit_code, wall_ms);
        self.emit(&crate::event::BesogneEvent::Summary { exit_code, wall_ms });
    }
}

fn format_duration_ms(ms: u64) -> String {
    style::format_duration(ms as u128)
}

fn node_type_name(node: &ResolvedNativeNode) -> &'static str {
    match node {
        ResolvedNativeNode::Binary { .. } => "binary",
        ResolvedNativeNode::File { .. } => "file",
        ResolvedNativeNode::Env { .. } => "env",
        ResolvedNativeNode::Service { .. } => "service",
        ResolvedNativeNode::Command { .. } => "command",

        ResolvedNativeNode::Platform { .. } => "platform",
        ResolvedNativeNode::Dns { .. } => "dns",
        ResolvedNativeNode::Metric { .. } => "metric",
        ResolvedNativeNode::Source { .. } => "source",
        ResolvedNativeNode::Std { .. } => "std",
        ResolvedNativeNode::Flag { .. } => "flag",
    }
}

/// Build a "N type, M type, ..." summary string from a list of nodes
fn build_pinned_summary(nodes: &[&ResolvedNode]) -> String {
    let mut counts: Vec<(&'static str, usize)> = Vec::new();
    for node in nodes {
        let name = node_type_name(&node.node);
        if let Some(entry) = counts.iter_mut().find(|(n, _)| *n == name) {
            entry.1 += 1;
        } else {
            counts.push((name, 1));
        }
    }
    counts.iter()
        .map(|(name, count)| {
            if *count == 1 {
                format!("{count} {name}")
            } else {
                let plural = match *name {
                    "binary" => "binaries",
                    _ => return format!("{count} {name}s"),
                };
                format!("{count} {plural}")
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

// ── Shared helpers ──────────────────────────────────────────────────

pub fn input_label(input: &ResolvedNode) -> String {
    match &input.node {
        ResolvedNativeNode::Env { name, .. } => format!("env:{name}"),
        ResolvedNativeNode::File { path, .. } => format!("file:{path}"),
        ResolvedNativeNode::Binary { name, .. } => format!("binary:{name}"),
        ResolvedNativeNode::Service { tcp, http, .. } =>
            format!("service:{}", tcp.as_deref().or(http.as_deref()).unwrap_or("?")),

        ResolvedNativeNode::Platform { os, arch, .. } =>
            format!("platform:{}-{}", os.as_deref().unwrap_or("?"), arch.as_deref().unwrap_or("?")),
        ResolvedNativeNode::Dns { host, .. } => format!("dns:{host}"),
        ResolvedNativeNode::Metric { metric, .. } => format!("metric:{metric}"),
        ResolvedNativeNode::Command { name, .. } => format!("command:{name}"),
        ResolvedNativeNode::Source { format, path, .. } =>
            format!("source:{}", path.as_deref().unwrap_or(format)),
        ResolvedNativeNode::Std { stream, .. } => format!("std:{stream}"),
        ResolvedNativeNode::Flag { name, value, .. } => {
            match value {
                Some(v) => format!("flag:{name}={v}"),
                None => format!("flag:{name}"),
            }
        }
    }
}

fn probe_detail(input: &ResolvedNode, result: &ProbeResult) -> String {
    match &input.node {
        ResolvedNativeNode::Binary { name, resolved_version, resolved_path, source, .. } => {
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
        ResolvedNativeNode::File { path, expect, .. } =>
            if let Some(exp) = expect { format!("{path} ({exp})") } else { path.clone() },
        ResolvedNativeNode::Service { name, tcp, http, .. } => {
            let label = name.as_deref().unwrap_or("service");
            let target = tcp.as_deref().or(http.as_deref()).unwrap_or("?");
            format!("{label} {target}")
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
        ResolvedNativeNode::Source { format, path, .. } => {
            let label = path.as_deref().unwrap_or(format);
            let count = result.variables.len();
            if count > 0 { format!("{label} ({count} vars)") } else { label.to_string() }
        }
        ResolvedNativeNode::Std { stream, expect, contains, .. } => {
            let mut parts = vec![stream.clone()];
            if let Some(exp) = expect { parts.push(format!("= {exp}")); }
            if !contains.is_empty() { parts.push(format!("contains {:?}", contains)); }
            parts.join(" ")
        }
        ResolvedNativeNode::Flag { name, value, .. } => {
            match value {
                Some(v) => format!("--{name}={v}"),
                None => format!("--{name}"),
            }
        }
    }
}

fn status_label(s: ProbeStatus) -> &'static str {
    match s {
        ProbeStatus::Pinned => label::PINNED,
        ProbeStatus::Sealed => label::SEALED,
        ProbeStatus::Probed => label::PROBED,
        ProbeStatus::Failed => label::FAILED,
        ProbeStatus::Skipped => label::SKIPPED,
    }
}

fn probe_status_token(s: ProbeStatus) -> &'static str {
    match s {
        ProbeStatus::Pinned => status::PINNED,
        ProbeStatus::Sealed => status::SEALED,
        ProbeStatus::Probed => status::PROBED,
        ProbeStatus::Failed => status::FAILED,
        ProbeStatus::Skipped => status::SKIP,
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

// ── Metrics ─────────────────────────────────────────────────────────

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
            wall_ms: r.wall_ms, user_ms: r.user_ms, sys_ms: r.sys_ms,
            max_rss_kb: r.max_rss_kb, disk_read_bytes: r.disk_read_bytes,
            disk_write_bytes: r.disk_write_bytes, net_read_bytes: r.net_read_bytes,
            net_write_bytes: r.net_write_bytes, processes_spawned: r.processes_spawned,
        }
    }
}

impl From<&CachedCommand> for Metrics {
    fn from(c: &CachedCommand) -> Self {
        Self {
            wall_ms: c.wall_ms, user_ms: c.user_ms, sys_ms: c.sys_ms,
            max_rss_kb: c.max_rss_kb, disk_read_bytes: c.disk_read_bytes,
            disk_write_bytes: c.disk_write_bytes, net_read_bytes: c.net_read_bytes,
            net_write_bytes: c.net_write_bytes, processes_spawned: c.processes_spawned,
        }
    }
}

impl From<&ProcessMetrics> for Metrics {
    fn from(p: &ProcessMetrics) -> Self {
        Self {
            wall_ms: p.wall_ms, user_ms: p.user_ms, sys_ms: p.sys_ms,
            max_rss_kb: p.max_rss_kb, disk_read_bytes: p.read_bytes,
            disk_write_bytes: p.write_bytes, net_read_bytes: 0,
            net_write_bytes: 0, processes_spawned: 0,
        }
    }
}

fn format_metrics_human(m: &Metrics) -> String {
    use telemetry::*;
    use metric_label as ml;
    let mut parts = Vec::new();
    parts.push(format!("{TIME}{TIME_ICON} {}:{:.3}s{RESET}", ml::TIME, m.wall_ms as f64 / 1000.0));
    // CPU: only show if >= 1s total (filters out bash/shell startup overhead)
    if m.user_ms >= 1000 || m.sys_ms >= 1000 {
        let cores = if m.wall_ms > 0 { (m.user_ms + m.sys_ms) as f64 / m.wall_ms as f64 } else { 0.0 };
        parts.push(format!(
            "{CPU}{CPU_ICON} {}:{:.2}s {} + {:.2}s {} ({:.1} {}){RESET}",
            ml::CPU, m.user_ms as f64 / 1000.0, ml::USER, m.sys_ms as f64 / 1000.0, ml::KERNEL, cores, ml::CORES,
        ));
    }
    // Memory: only show if >= 50MB (filters out shell/runtime baseline ~4MB)
    if m.max_rss_kb >= 50 * 1024 {
        parts.push(format!("{MEMORY}{MEMORY_ICON} {}:{}{RESET}", ml::MEMORY, format_bytes(m.max_rss_kb * 1024)));
    }
    // >= 50MB read or >= 1MB write — filters out library loading noise (~13MB per process)
    if m.disk_read_bytes >= 50 * 1_048_576 || m.disk_write_bytes >= 1_048_576 {
        parts.push(format!(
            "{DISK}{DISK_ICON} \u{2b07} {}:{} \u{2b06} {}:{}{RESET}",
            ml::READ, format_bytes(m.disk_read_bytes), ml::WRITE, format_bytes(m.disk_write_bytes),
        ));
    }
    // Network metrics: per-process when LD_PRELOAD is active (send/recv hooks),
    // falls back to /proc/net/dev (system-wide) when preload unavailable.
    if m.net_read_bytes > 0 || m.net_write_bytes > 0 {
        parts.push(format!(
            "{NETWORK}{NETWORK_ICON} \u{2b07} {}:{} \u{2b06} {}:{}{RESET}",
            ml::DOWNLOAD, format_bytes(m.net_read_bytes), ml::UPLOAD, format_bytes(m.net_write_bytes),
        ));
    }
    if m.processes_spawned > 0 {
        parts.push(format!("{PROCESS}{PROCESS_ICON} {}:{}{RESET}", ml::PROCESSES, m.processes_spawned + 1));
    }
    parts.join("  ")
}

fn format_metrics_ci(m: &Metrics) -> String {
    use telemetry::*;
    use metric_label as ml;
    let mut parts = vec![format!("{:.3}s", m.wall_ms as f64 / 1000.0)];
    if m.user_ms > 0 || m.sys_ms > 0 {
        let cores = if m.wall_ms > 0 { (m.user_ms + m.sys_ms) as f64 / m.wall_ms as f64 } else { 0.0 };
        parts.push(format!(
            "{}:{:.2}s {} + {:.2}s {} ({:.1} {})",
            ml::CPU, m.user_ms as f64 / 1000.0, ml::USER, m.sys_ms as f64 / 1000.0, ml::KERNEL, cores, ml::CORES,
        ));
    }
    if m.max_rss_kb > 0 {
        parts.push(format!("{}:{}", ml::MEMORY, format_bytes(m.max_rss_kb * 1024)));
    }
    if m.disk_read_bytes > 0 || m.disk_write_bytes > 0 {
        parts.push(format!("{DISK_ICON} \u{2b07} {}:{} \u{2b06} {}:{}",
            ml::READ, format_bytes(m.disk_read_bytes), ml::WRITE, format_bytes(m.disk_write_bytes)));
    }
    if m.net_read_bytes > 0 || m.net_write_bytes > 0 {
        parts.push(format!("{NETWORK_ICON} \u{2b07} {}:{} \u{2b06} {}:{}",
            ml::DOWNLOAD, format_bytes(m.net_read_bytes), ml::UPLOAD, format_bytes(m.net_write_bytes)));
    }
    if m.processes_spawned > 0 {
        parts.push(format!("{}:{}", ml::PROCESSES, m.processes_spawned + 1));
    }
    parts.join("  ")
}

fn format_metrics_json(m: &Metrics) -> serde_json::Value {
    let cores = if m.wall_ms > 0 { (m.user_ms + m.sys_ms) as f64 / m.wall_ms as f64 } else { 0.0 };
    serde_json::json!({
        "time_ms": m.wall_ms, "cpu_user_ms": m.user_ms, "cpu_kernel_ms": m.sys_ms,
        "cpu_cores_used": cores, "memory_bytes": m.max_rss_kb * 1024,
        "disk_read_bytes": m.disk_read_bytes, "disk_write_bytes": m.disk_write_bytes,
        "net_download_bytes": m.net_read_bytes, "net_upload_bytes": m.net_write_bytes,
        "processes": m.processes_spawned + 1,
    })
}

// ── Path helpers ────────────────────────────────────────────────────

pub(crate) fn crop_path(path: &str, max_len: usize) -> String {
    if path.starts_with("/nix/store/") {
        let after_store = &path["/nix/store/".len()..];
        let hash_end = after_store.find('-').unwrap_or(8).min(8);
        let hash_prefix = &after_store[..hash_end];
        let basename = path.rsplit('/').next().unwrap_or(path);
        return format!("/nix/store/{hash_prefix}.../{basename}");
    }
    if path.len() <= max_len { return path.to_string(); }
    let basename = path.rsplit('/').next().unwrap_or(path);
    if basename.len() + 6 >= max_len { return format!(".../{basename}"); }
    let prefix_len = max_len - basename.len() - 4;
    format!("{}.../{basename}", &path[..prefix_len])
}

fn crop_cmdline(cmdline: &str, max_path_len: usize) -> String {
    cmdline.split(' ').map(|t| {
        if t.starts_with('/') { crop_path(t, max_path_len) } else { t.to_string() }
    }).collect::<Vec<_>>().join(" ")
}

fn resolve_exec_display(exec: &[String], ctx: &CommandContext) -> String {
    exec.iter().map(|arg| {
        ctx.binary_paths.get(arg.as_str()).cloned().unwrap_or_else(|| arg.clone())
    }).collect::<Vec<_>>().join(" ")
}

pub(crate) fn process_label(p: &ProcessMetrics) -> String {
    if !p.cmdline.is_empty() { crop_cmdline(&p.cmdline, 80) }
    else if !p.comm.is_empty() { p.comm.clone() }
    else { format!("pid:{}", p.pid) }
}

/// Detect if a child process is a subshell (forked copy of its parent).
/// Returns " (subshell)" tag if the child has the same cmdline as its parent.
fn subshell_tag(child: &ProcessMetrics, parent_cmdline: &str) -> String {
    if !child.cmdline.is_empty() && !parent_cmdline.is_empty() && child.cmdline == parent_cmdline {
        format!(" {}(subshell){RESET}", weight::L3)
    } else {
        String::new()
    }
}

// ── Command context display ─────────────────────────────────────────

fn format_command_context_human(exec: &[String], ctx: &CommandContext) {
    for arg in exec {
        if let Some(path) = ctx.binary_paths.get(arg.as_str()) {
            if path != arg {
                let display_path = crop_path(path, 60);
                let version = ctx.binary_versions.get(arg.as_str())
                    .map(|v| format!(" {}v{v}{RESET}", style::palette::CYAN))
                    .unwrap_or_default();
                eprintln!("    {}{arg} {RESET}{}→ {display_path}{RESET}{version}",
                    style::palette::DIM_BLUE, weight::L3);
            }
        }
    }
    if !ctx.env_vars.is_empty() {
        let mut vars: Vec<_> = ctx.env_vars.iter().collect();
        vars.sort_by_key(|(k, _)| *k);
        let parts: Vec<String> = vars.iter().map(|(k, v)| {
            if ctx.secret_vars.contains(k.as_str()) {
                format!("{}{k}{}=*****{RESET}", status::CACHED, weight::L3)
            } else {
                let display = if v.len() > 50 { format!("{}...", &v[..47]) } else { v.to_string() };
                format!("{}{k}={display}{RESET}", weight::L3)
            }
        }).collect();
        let mut line = String::from("    ");
        for (i, part) in parts.iter().enumerate() {
            if i > 0 { line.push_str("  "); }
            if line.len() > 140 && i > 0 { eprintln!("{line}"); line = String::from("    "); }
            line.push_str(part);
        }
        if line.trim().len() > 0 { eprintln!("{line}"); }
    }
}

// ── Process tree ────────────────────────────────────────────────────

fn format_process_tree_human(tree: &[ProcessMetrics]) {
    use metric_label as ml;
    if tree.len() <= 1 { return; }
    eprintln!("    {}\u{250c} {}{RESET} {}({} {}){RESET}",
        phase::EXEC_DIM, message::PROCESS_TREE, style::palette::CYAN, tree.len(), ml::PROCESSES);
    let mut children: HashMap<u32, Vec<usize>> = HashMap::new();
    for (i, p) in tree.iter().enumerate() {
        if i > 0 { children.entry(p.ppid).or_default().push(i); }
    }
    print_process_node(tree, &children, 0, "    ", true);
}

fn print_process_node(
    tree: &[ProcessMetrics], children: &HashMap<u32, Vec<usize>>,
    idx: usize, prefix: &str, is_root: bool,
) {
    let p = &tree[idx];
    let label = process_label(p);
    let metrics = format_metrics_human(&Metrics::from(p));
    let exit_tag = fmt_exit(p.exit_code);
    let label_style = if is_root { style::ptree::ROOT } else { style::ptree::CHILD };
    eprintln!("{prefix}{label_style}{label}{RESET} [{exit_tag}]  {metrics}");

    let parent_cmdline = &p.cmdline;
    let kids = children.get(&p.pid).map(|k| k.as_slice()).unwrap_or(&[]);
    for (i, &kid_idx) in kids.iter().enumerate() {
        let is_last = i == kids.len() - 1;
        let connector = if is_last { "\u{2514}\u{2500} " } else { "\u{251c}\u{2500} " };
        let child_prefix = format!("{prefix}{}", if is_last { "   " } else { "\u{2502}  " });
        let kp = &tree[kid_idx];
        let kid_label = process_label(kp);
        let kid_metrics = format_metrics_human(&Metrics::from(kp));
        let kid_exit = fmt_exit(kp.exit_code);
        let ss_tag = subshell_tag(kp, parent_cmdline);
        eprintln!("{prefix}{connector}{}{kid_label}{RESET}{ss_tag} [{kid_exit}]  {kid_metrics}", style::ptree::CHILD);

        if let Some(grandkids) = children.get(&kp.pid) {
            for (j, &gk_idx) in grandkids.iter().enumerate() {
                let gk_last = j == grandkids.len() - 1;
                let gk_conn = if gk_last { "\u{2514}\u{2500} " } else { "\u{251c}\u{2500} " };
                let gk_prefix = format!("{child_prefix}{}", if gk_last { "   " } else { "\u{2502}  " });
                print_process_node_recursive(tree, children, gk_idx, &child_prefix, gk_conn, &gk_prefix, &kp.cmdline);
            }
        }
    }
}

fn print_process_node_recursive(
    tree: &[ProcessMetrics], children: &HashMap<u32, Vec<usize>>,
    idx: usize, parent_prefix: &str, connector: &str, my_prefix: &str,
    parent_cmdline: &str,
) {
    let p = &tree[idx];
    let label = process_label(p);
    let metrics = format_metrics_human(&Metrics::from(p));
    let exit_tag = fmt_exit(p.exit_code);
    let ss_tag = subshell_tag(p, parent_cmdline);
    eprintln!("{parent_prefix}{connector}{}{label}{RESET}{ss_tag} [{exit_tag}]  {metrics}", style::ptree::CHILD);

    if let Some(kids) = children.get(&p.pid) {
        for (i, &kid_idx) in kids.iter().enumerate() {
            let is_last = i == kids.len() - 1;
            let kid_conn = if is_last { "\u{2514}\u{2500} " } else { "\u{251c}\u{2500} " };
            let kid_prefix = format!("{my_prefix}{}", if is_last { "   " } else { "\u{2502}  " });
            print_process_node_recursive(tree, children, kid_idx, my_prefix, kid_conn, &kid_prefix, &p.cmdline);
        }
    }
}

/// Build pid→cmdline map for CI flat-list subshell detection
fn ci_cmdline_map(tree: &[ProcessMetrics]) -> HashMap<u32, String> {
    tree.iter().map(|p| (p.pid, p.cmdline.clone())).collect()
}

/// CI subshell tag: " [subshell]" if child has same cmdline as parent
fn ci_subshell_tag(p: &ProcessMetrics, cmdlines: &HashMap<u32, String>) -> &'static str {
    if !p.cmdline.is_empty() {
        if let Some(parent_cmd) = cmdlines.get(&p.ppid) {
            if *parent_cmd == p.cmdline {
                return " [subshell]";
            }
        }
    }
    ""
}

fn format_process_tree_json(tree: &[ProcessMetrics]) -> serde_json::Value {
    // Build pid→cmdline map for subshell detection
    let cmdline_by_pid: HashMap<u32, &str> = tree.iter()
        .map(|p| (p.pid, p.cmdline.as_str()))
        .collect();
    tree.iter().map(|p| {
        let is_subshell = !p.cmdline.is_empty()
            && cmdline_by_pid.get(&p.ppid).map_or(false, |parent| *parent == p.cmdline);
        serde_json::json!({
            "pid": p.pid, "ppid": p.ppid, "comm": p.comm, "cmdline": p.cmdline,
            "subshell": is_subshell,
            "exit_code": p.exit_code, "wall_ms": p.wall_ms, "user_ms": p.user_ms,
            "sys_ms": p.sys_ms, "max_rss_kb": p.max_rss_kb,
            "read_bytes": p.read_bytes, "write_bytes": p.write_bytes,
            "voluntary_cs": p.voluntary_cs, "involuntary_cs": p.involuntary_cs,
        })
    }).collect()
}

// ── Human renderer ──────────────────────────────────────────────────

pub struct HumanRenderer {
    verbose: bool,
    cached_probes: usize,
    cached_commands: usize,
    last_cached_at: Option<String>,
    phase_start: Option<std::time::Instant>,
    /// Collect executed command metrics for end-of-run summary
    command_metrics: Vec<(String, Metrics, i32)>, // (name, metrics, exit_code)
}

impl HumanRenderer {
    pub fn new(verbose: bool) -> Self {
        Self {
            verbose, cached_probes: 0, cached_commands: 0,
            last_cached_at: None, phase_start: None,
            command_metrics: Vec::new(),
        }
    }
}

impl OutputRenderer for HumanRenderer {
    fn on_start(&mut self, ir: &BesogneIR) {
        eprintln!("{} v{} — {}",
            bold(&ir.metadata.name), ir.metadata.version, ir.metadata.description);
    }

    fn on_phase_start(&mut self, p: &str, count: usize) {
        if count == 0 { return; }
        let token = match p {
            "build" => phase::BUILD,
            "seal" => phase::SEAL,
            "exec" => phase::EXEC,
            _ => phase::EXEC,
        };
        self.phase_start = Some(std::time::Instant::now());
        eprintln!("\n{}", styled(token, &format!("▸ {p} ({count} nodes)")));
    }

    fn on_probe_result(&mut self, input: &ResolvedNode, result: &ProbeResult, s: ProbeStatus) {
        if !self.verbose && s == ProbeStatus::Sealed {
            self.cached_probes += 1;
            return;
        }
        let detail = probe_detail(input, result);
        if result.success {
            eprintln!("  {} {detail}", styled(probe_status_token(s), status_label(s)));
        } else {
            eprintln!("  {} {detail} — {}",
                styled(status::FAILED, label::FAILED), result.error.as_deref().unwrap_or(label::FAILED));
        }
    }

    fn on_phase_end(&mut self, p: &str) {
        if let Some(start) = self.phase_start.take() {
            let ms = start.elapsed().as_millis();
            let token = match p {
                "build" => phase::BUILD,
                "seal" => phase::SEAL,
                "exec" => phase::EXEC,
                _ => phase::EXEC,
            };
            eprintln!("  {} {}", styled(token, &format!("▸ {p}")), dim(&format!("({})", format_duration_ms(ms as u64))));
        }
    }

    fn on_command_start(&mut self, name: &str, exec: &[String], ctx: &CommandContext) {
        eprintln!("\n  {} {name}: {}", bold("▹"), exec.join(" "));
        format_command_context_human(exec, ctx);
    }

    fn on_command_output(&mut self, _name: &str, _stdout: &str, _stderr: &str) {
        // Output already streamed in real-time by the tracer
    }

    fn on_command_end(&mut self, name: &str, result: &CommandResult) {
        // Just show pass/fail inline — metrics summary at the end
        if result.exit_code == 0 {
            eprintln!("  {} {name}", styled(status::FRESH, label::FRESH));
        } else {
            eprintln!("  {} {name}  exit {}",
                styled(status::FAILED, label::FAILED), result.exit_code);
        }
        // Store metrics for end-of-run summary
        self.command_metrics.push((name.to_string(), Metrics::from(result), result.exit_code));
    }

    fn on_command_cached(&mut self, name: &str, exec: &[String], cached: &CachedCommand, ctx: &CommandContext) {
        if self.last_cached_at.is_none() || self.last_cached_at.as_deref() < Some(&cached.ran_at) {
            self.last_cached_at = Some(cached.ran_at.clone());
        }
        if !self.verbose {
            self.cached_commands += 1;
            return;
        }
        eprintln!("\n{} {name}: {}  {}",
            styled(status::CACHED, label::CACHED),
            resolve_exec_display(exec, ctx),
            dim(&format!("({} {})", message::RAN, format_relative_time(&cached.ran_at))));
        format_command_context_human(exec, ctx);
        if !cached.stdout.is_empty() || !cached.stderr.is_empty() {
            for line in cached.stdout.lines() { eprintln!("    {}", dim(line)); }
            for line in cached.stderr.lines() { eprintln!("    {}", dim(line)); }
        }
        let metrics = format_metrics_human(&Metrics::from(cached));
        eprintln!("  {} {name}  {metrics}", styled(status::CACHED, label::CACHED));
        // Process tree only shown in --status view, not during cached replay
    }

    fn on_build_pinned_summary(&mut self, nodes: &[&ResolvedNode]) {
        let summary = build_pinned_summary(nodes);
        eprintln!("  {} {summary}", styled(status::PINNED, label::PINNED));
    }

    fn on_changed_probes(&mut self, changed: &[String]) {
        eprintln!("\n  {} {} changed → re-executing",
            styled(status::INVALIDATED, "△"),
            changed.join(", "));
    }

    fn on_skip(&mut self, total_nodes: usize, ran_at: &str, duration_ms: u64) {
        eprintln!("{} ({} nodes cached, ran {}, {}, use {} to show last run)",
            styled(status::FRESH, label::NOTHING_TO_DO),
            total_nodes,
            format_relative_time(ran_at),
            format_duration_ms(duration_ms),
            bold("--status"));
    }

    fn on_undeclared_deps(&mut self, binaries: &[String], env_vars: &[String]) {
        if binaries.is_empty() && env_vars.is_empty() { return; }
        eprintln!("\n{}", styled(diagnostic::WARN, message::UNDECLARED_DEPS));
        eprintln!("  {}", dim(message::CACHE_DISABLED));
        if !binaries.is_empty() {
            eprintln!("  {} {}", styled(status::CACHED, "binaries:"), binaries.join(", "));
        }
        if !env_vars.is_empty() {
            eprintln!("  {} {}", styled(status::CACHED, "env vars:"), env_vars.join(", "));
        }
    }

    fn on_summary(&mut self, exit_code: i32, wall_ms: u64) {
        let total = self.cached_probes + self.cached_commands;
        if !self.verbose && self.cached_commands > 0 && exit_code == 0 && self.command_metrics.is_empty() {
            let ago = self.last_cached_at.as_deref()
                .map(|t| format!(", ran {}", format_relative_time(t)))
                .unwrap_or_default();
            eprintln!("{} {}",
                styled(status::FRESH, label::NOTHING_TO_DO),
                dim(&format!("({total} {}{ago}, {})", message::NODES_CACHED, message::STATUS_HINT)));
            return;
        }

        // Per-command metrics summary (only significant values)
        if !self.command_metrics.is_empty() {
            eprintln!();
            for (name, m, code) in &self.command_metrics {
                let mut parts = Vec::new();

                // Only include metrics with significant values
                if m.wall_ms >= 10 {
                    parts.push(format!("{}{} {:.3}s{}",
                        telemetry::TIME, telemetry::TIME_ICON, m.wall_ms as f64 / 1000.0, RESET));
                }
                if m.user_ms >= 1000 || m.sys_ms >= 1000 {
                    let cores = if m.wall_ms > 0 { (m.user_ms + m.sys_ms) as f64 / m.wall_ms as f64 } else { 0.0 };
                    parts.push(format!("{}{} {:.2}s user + {:.2}s kernel ({:.1} cores){}",
                        telemetry::CPU, telemetry::CPU_ICON,
                        m.user_ms as f64 / 1000.0, m.sys_ms as f64 / 1000.0, cores, RESET));
                }
                if m.max_rss_kb >= 50 * 1024 { // >= 50MB
                    parts.push(format!("{}{} {}{}",
                        telemetry::MEMORY, telemetry::MEMORY_ICON, format_bytes(m.max_rss_kb * 1024), RESET));
                }
                // >= 50MB read or >= 1MB write — filters out library loading noise (~13MB)
                if m.disk_read_bytes >= 50 * 1_048_576 || m.disk_write_bytes >= 1_048_576 {
                    parts.push(format!("{}{} \u{2b07}{}  \u{2b06}{}{}",
                        telemetry::DISK, telemetry::DISK_ICON,
                        format_bytes(m.disk_read_bytes), format_bytes(m.disk_write_bytes), RESET));
                }
                if m.net_read_bytes >= 1024 || m.net_write_bytes >= 1024 {
                    parts.push(format!("{}{} \u{2b07}{}  \u{2b06}{}{}",
                        telemetry::NETWORK, telemetry::NETWORK_ICON,
                        format_bytes(m.net_read_bytes), format_bytes(m.net_write_bytes), RESET));
                }

                if parts.is_empty() { continue; } // skip commands with no significant metrics

                let status_tag = if *code == 0 {
                    styled(status::FRESH, label::FRESH)
                } else {
                    styled(status::FAILED, &format!("{} exit {code}", label::FAILED))
                };
                eprintln!("  {status_tag} {name}  {}", parts.join("  "));
            }
        }

        if exit_code == 0 {
            eprintln!("\n{} {}", styled(status::FRESH, label::DONE), format_duration_ms(wall_ms));
        } else {
            eprintln!("\n{} exit {exit_code}  {}",
                styled(status::FAILED, label::FAILED), format_duration_ms(wall_ms));
        }
    }
}

pub(crate) fn format_relative_time(iso_time: &str) -> String {
    let Ok(then) = chrono::DateTime::parse_from_rfc3339(iso_time) else {
        return iso_time.to_string();
    };
    let delta = chrono::Utc::now().signed_duration_since(then);
    if delta.num_seconds() < 5 { "just now".to_string() }
    else if delta.num_seconds() < 60 { format!("{}s ago", delta.num_seconds()) }
    else if delta.num_minutes() < 60 { format!("{}m ago", delta.num_minutes()) }
    else if delta.num_hours() < 24 { format!("{}h ago", delta.num_hours()) }
    else { format!("{}d ago", delta.num_days()) }
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
            "event": "start", "name": ir.metadata.name,
            "version": ir.metadata.version, "description": ir.metadata.description,
        }));
    }

    fn on_phase_start(&mut self, phase: &str, count: usize) {
        self.emit(&serde_json::json!({"event": "phase_start", "phase": phase, "input_count": count}));
    }

    fn on_probe_result(&mut self, input: &ResolvedNode, result: &ProbeResult, s: ProbeStatus) {
        self.emit(&serde_json::json!({
            "event": "probe", "input": input_label(input),
            "phase": format!("{:?}", input.phase).to_lowercase(),
            "status": status_label(s), "success": result.success,
            "hash": result.hash, "error": result.error,
        }));
    }

    fn on_phase_end(&mut self, phase: &str) {
        self.emit(&serde_json::json!({"event": "phase_end", "phase": phase}));
    }

    fn on_command_start(&mut self, name: &str, exec: &[String], ctx: &CommandContext) {
        let resolved: Vec<String> = exec.iter().map(|arg|
            ctx.binary_paths.get(arg.as_str()).cloned().unwrap_or_else(|| arg.clone())
        ).collect();
        let env_display: HashMap<&str, &str> = ctx.env_vars.iter()
            .map(|(k, v)| if ctx.secret_vars.contains(k.as_str()) { (k.as_str(), "*****") } else { (k.as_str(), v.as_str()) })
            .collect();
        self.emit(&serde_json::json!({
            "event": "command_start", "phase": "exec", "name": name,
            "exec": exec, "resolved_exec": resolved, "env": env_display,
        }));
    }

    fn on_command_output(&mut self, name: &str, stdout: &str, stderr: &str) {
        if !stdout.is_empty() || !stderr.is_empty() {
            self.emit(&serde_json::json!({"event": "command_output", "name": name, "stdout": stdout, "stderr": stderr}));
        }
    }

    fn on_command_end(&mut self, name: &str, result: &CommandResult) {
        let mut obj = serde_json::json!({"event": "command_end", "name": name, "exit_code": result.exit_code});
        let m = format_metrics_json(&Metrics::from(result));
        obj.as_object_mut().unwrap().extend(m.as_object().unwrap().clone());
        self.emit(&obj);
    }

    fn on_command_cached(&mut self, name: &str, exec: &[String], cached: &CachedCommand, _ctx: &CommandContext) {
        let mut obj = serde_json::json!({
            "event": "command_cached", "name": name, "exec": exec,
            "exit_code": cached.exit_code, "ran_at": cached.ran_at,
            "stdout": cached.stdout, "stderr": cached.stderr,
        });
        let m = format_metrics_json(&Metrics::from(cached));
        obj.as_object_mut().unwrap().extend(m.as_object().unwrap().clone());
        if !cached.process_tree.is_empty() {
            obj.as_object_mut().unwrap().insert("process_tree".to_string(), format_process_tree_json(&cached.process_tree));
        }
        self.emit(&obj);
    }

    fn on_build_pinned_summary(&mut self, nodes: &[&ResolvedNode]) {
        let counts: HashMap<&str, usize> = nodes.iter().fold(HashMap::new(), |mut m, n| {
            *m.entry(node_type_name(&n.node)).or_insert(0) += 1;
            m
        });
        self.emit(&serde_json::json!({"event": "build_pinned", "counts": counts, "total": nodes.len()}));
    }

    fn on_changed_probes(&mut self, changed: &[String]) {
        self.emit(&serde_json::json!({"event": "changed_probes", "changed": changed}));
    }

    fn on_skip(&mut self, total_nodes: usize, ran_at: &str, duration_ms: u64) {
        self.emit(&serde_json::json!({"event": "skip", "total_nodes": total_nodes, "ran_at": ran_at, "duration_ms": duration_ms}));
    }

    fn on_undeclared_deps(&mut self, binaries: &[String], env_vars: &[String]) {
        if binaries.is_empty() && env_vars.is_empty() { return; }
        self.emit(&serde_json::json!({"event": "undeclared_deps", "binaries": binaries, "env_vars": env_vars}));
    }

    fn on_summary(&mut self, exit_code: i32, wall_ms: u64) {
        self.emit(&serde_json::json!({"event": "summary", "exit_code": exit_code, "time_ms": wall_ms}));
    }
}

// ── CI renderer ─────────────────────────────────────────────────────

pub struct CiRenderer;
impl CiRenderer { pub fn new() -> Self { Self } }

impl OutputRenderer for CiRenderer {
    fn on_start(&mut self, ir: &BesogneIR) {
        eprintln!("::group::{} v{} — {}", ir.metadata.name, ir.metadata.version, ir.metadata.description);
    }

    fn on_phase_start(&mut self, _phase: &str, _count: usize) {}

    fn on_probe_result(&mut self, input: &ResolvedNode, result: &ProbeResult, s: ProbeStatus) {
        let detail = probe_detail(input, result);
        let tag = match s {
            ProbeStatus::Pinned => "[PIN]",
            ProbeStatus::Sealed => "[SEAL]",
            ProbeStatus::Probed => "[PROBE]",
            ProbeStatus::Failed => "[FAIL]",
            ProbeStatus::Skipped => "[SKIP]",
        };
        if result.success { eprintln!("  {tag} {detail}"); }
        else { eprintln!("  {tag} {detail} — {}", result.error.as_deref().unwrap_or(label::FAILED)); }
    }

    fn on_phase_end(&mut self, _phase: &str) { eprintln!("::endgroup::"); }

    fn on_command_start(&mut self, name: &str, exec: &[String], _ctx: &CommandContext) {
        eprintln!("::group::{name}: {}", exec.join(" "));
    }

    fn on_command_output(&mut self, _name: &str, stdout: &str, stderr: &str) {
        for line in stdout.lines() { eprintln!("    {line}"); }
        for line in stderr.lines() { eprintln!("    {line}"); }
    }

    fn on_command_end(&mut self, name: &str, result: &CommandResult) {
        let metrics = format_metrics_ci(&Metrics::from(result));
        if result.exit_code == 0 { eprintln!("  [PASS] {name}  {metrics}"); }
        else { eprintln!("  [FAIL] {name}  exit {}  {metrics}", result.exit_code); }
        if result.process_tree.len() > 1 {
            eprintln!("  process tree ({} processes):", result.process_tree.len());
            let ci_cmdlines = ci_cmdline_map(&result.process_tree);
            for p in &result.process_tree {
                let ss = ci_subshell_tag(p, &ci_cmdlines);
                eprintln!("    {}{ss}  {}", process_label(p), format_metrics_ci(&Metrics::from(p)));
            }
        }
        eprintln!("::endgroup::");
    }

    fn on_command_cached(&mut self, name: &str, exec: &[String], cached: &CachedCommand, _ctx: &CommandContext) {
        eprintln!("::group::[CACHE] {name}: {} (ran {})", exec.join(" "), cached.ran_at);
        for line in cached.stdout.lines() { eprintln!("    {line}"); }
        for line in cached.stderr.lines() { eprintln!("    {line}"); }
        let metrics = format_metrics_ci(&Metrics::from(cached));
        eprintln!("  [CACHE] {name}  {metrics}");
        if cached.process_tree.len() > 1 {
            eprintln!("  process tree ({} processes):", cached.process_tree.len());
            let ci_cmdlines = ci_cmdline_map(&cached.process_tree);
            for p in &cached.process_tree {
                let ss = ci_subshell_tag(p, &ci_cmdlines);
                eprintln!("    {}{ss}  {}", process_label(p), format_metrics_ci(&Metrics::from(p)));
            }
        }
        eprintln!("::endgroup::");
    }

    fn on_build_pinned_summary(&mut self, nodes: &[&ResolvedNode]) {
        let summary = build_pinned_summary(nodes);
        eprintln!("  [PIN] {summary}");
    }

    fn on_changed_probes(&mut self, changed: &[String]) {
        eprintln!("::notice::Changed: {}", changed.join(", "));
    }

    fn on_skip(&mut self, total_nodes: usize, ran_at: &str, duration_ms: u64) {
        eprintln!("::notice::SKIP ({total_nodes} nodes cached, ran {ran_at}, {})", format_duration_ms(duration_ms));
    }

    fn on_undeclared_deps(&mut self, binaries: &[String], env_vars: &[String]) {
        if binaries.is_empty() && env_vars.is_empty() { return; }
        eprintln!("::warning::Undeclared runtime dependencies — cache disabled");
        if !binaries.is_empty() { eprintln!("  binaries: {}", binaries.join(", ")); }
        if !env_vars.is_empty() { eprintln!("  env vars: {}", env_vars.join(", ")); }
    }

    fn on_summary(&mut self, exit_code: i32, wall_ms: u64) {
        if exit_code == 0 { eprintln!("::notice::PASS {:.3}s", wall_ms as f64 / 1000.0); }
        else { eprintln!("::error::FAILED exit {exit_code}  {:.3}s", wall_ms as f64 / 1000.0); }
    }
}

// ── Status tree (--status) ──────────────────────────────────────────

pub(crate) enum NodeStatus { Pinned, Sealed, Cached, Unknown }

pub(crate) fn node_status_badge(s: &NodeStatus) -> String {
    match s {
        NodeStatus::Pinned => style::status_badge(label::PINNED, status::PINNED),
        NodeStatus::Sealed => style::status_badge(label::SEALED, status::SEALED),
        NodeStatus::Cached => style::status_badge(label::CACHED, status::CACHED),
        NodeStatus::Unknown => style::status_badge(label::PENDING, status::PENDING),
    }
}

pub(crate) fn node_type_badge(n: &ResolvedNode) -> String {
    let (token, text) = match &n.node {
        ResolvedNativeNode::Binary { .. }   => (node::BINARY, badge::BINARY),
        ResolvedNativeNode::File { .. }     => (node::FILE, badge::FILE),
        ResolvedNativeNode::Env { .. }      => (node::ENV, badge::ENV),
        ResolvedNativeNode::Service { .. }  => (node::SERVICE, badge::SERVICE),
        ResolvedNativeNode::Command { .. }  => (node::COMMAND, badge::COMMAND),

        ResolvedNativeNode::Platform { .. } => (node::PLATFORM, badge::PLATFORM),
        ResolvedNativeNode::Dns { .. }      => (node::DNS, badge::DNS),
        ResolvedNativeNode::Metric { .. }   => (node::METRIC, badge::METRIC),
        ResolvedNativeNode::Source { .. }   => (node::SOURCE, badge::SOURCE),
        ResolvedNativeNode::Std { .. }     => (node::STD, badge::STD),
        ResolvedNativeNode::Flag { .. }    => (node::ENV, badge::ENV), // reuse env style for flags
    };
    styled(token, text)
}

pub(crate) fn node_short_label(n: &ResolvedNode) -> String {
    match &n.node {
        ResolvedNativeNode::Binary { name, resolved_version, source, .. } => {
            let ver = resolved_version.as_deref()
                .map(|v| format!(" {}v{v}{RESET}", weight::L3)).unwrap_or_default();
            let src = match source {
                Some(crate::ir::types::BinarySourceResolved::Nix { pname, .. }) =>
                    format!(" {}[nix:{}]{RESET}", weight::L3, pname.as_deref().unwrap_or("?")),
                Some(crate::ir::types::BinarySourceResolved::Mise { tool }) =>
                    format!(" {}[mise:{tool}]{RESET}", weight::L3),
                Some(crate::ir::types::BinarySourceResolved::System) =>
                    format!(" {}[system]{RESET}", weight::L3),
                None => String::new(),
            };
            format!("{name}{ver}{src}")
        }
        ResolvedNativeNode::File { path, .. } => path.clone(),
        ResolvedNativeNode::Env { name, secret, .. } =>
            if *secret { format!("{name} {}", dim(message::SECRET)) } else { name.clone() },
        ResolvedNativeNode::Service { tcp, http, .. } =>
            tcp.as_deref().or(http.as_deref()).unwrap_or("?").to_string(),
        ResolvedNativeNode::Command { name, run, side_effects, .. } => {
            let cmd = run.join(" ");
            let se = if *side_effects { format!(" {}", styled(status::FAILED, message::SIDE_EFFECTS)) } else { String::new() };
            let display_cmd = if cmd.len() > 40 { format!("{}...", &cmd[..37]) } else { cmd };
            format!("{name} {}{se}", dim(&display_cmd))
        }

        ResolvedNativeNode::Platform { os, arch, .. } =>
            format!("{}/{}", os.as_deref().unwrap_or("?"), arch.as_deref().unwrap_or("?")),
        ResolvedNativeNode::Dns { host, .. } => host.clone(),
        ResolvedNativeNode::Metric { metric, .. } => metric.clone(),
        ResolvedNativeNode::Source { format, path, .. } =>
            path.as_deref().unwrap_or(format).to_string(),
        ResolvedNativeNode::Std { stream, expect, contains, .. } => {
            let mut desc = stream.clone();
            if let Some(exp) = expect { desc.push_str(&format!(" {}", dim(&format!("= {exp}")))); }
            if !contains.is_empty() { desc.push_str(&format!(" {}", dim(&format!("contains {:?}", contains)))); }
            desc
        }
        ResolvedNativeNode::Flag { name, value, .. } => {
            match value {
                Some(v) => format!("--{name}={v}"),
                None => format!("--{name}"),
            }
        }
    }
}

pub(crate) fn get_node_status(n: &ResolvedNode, cache: &ContextCache) -> NodeStatus {
    if n.sealed.is_some() { return NodeStatus::Pinned; }
    if cache.get_probe(&n.id.0).is_some() {
        // Seal-phase probes → "sealed", exec-phase probes → "cached"
        return if n.phase == Phase::Exec { NodeStatus::Cached } else { NodeStatus::Sealed };
    }
    if let ResolvedNativeNode::Command { name, .. } = &n.node {
        if cache.get_command(name).is_some() { return NodeStatus::Cached; }
    }
    NodeStatus::Unknown
}

pub fn render_status_tree(ir: &BesogneIR, cache: &ContextCache) {
    let build_nodes: Vec<&ResolvedNode> = ir.nodes.iter().filter(|n| n.phase == Phase::Build).collect();
    let seal_nodes: Vec<&ResolvedNode> = ir.nodes.iter().filter(|n| n.phase == Phase::Seal).collect();
    let exec_nodes: Vec<&ResolvedNode> = ir.nodes.iter().filter(|n| n.phase == Phase::Exec).collect();

    let root_label = format!("{} {} — {}",
        bold(&ir.metadata.name), dim(&format!("v{}", ir.metadata.version)), ir.metadata.description);
    let mut root = Tree::new(root_label);

    // Build phase
    if !build_nodes.is_empty() {
        let mut t = Tree::new(format!("{} {}",
            styled(phase::BUILD, phase_label::BUILD), dim(&format!("({} nodes)", build_nodes.len()))));
        for n in &build_nodes {
            t.push(Tree::new(format!("{} {} {}",
                node_status_badge(&get_node_status(n, cache)), node_type_badge(n), node_short_label(n))));
        }
        root.push(t);
    }

    // Seal phase
    if !seal_nodes.is_empty() {
        let mut t = Tree::new(format!("{} {}",
            styled(phase::SEAL, phase_label::SEAL), dim(&format!("({} nodes)", seal_nodes.len()))));
        for n in &seal_nodes {
            t.push(Tree::new(format!("{} {} {}",
                node_status_badge(&get_node_status(n, cache)), node_type_badge(n), node_short_label(n))));
        }
        root.push(t);
    }

    // Exec phase — tier-based DAG layout (not a tree — DAGs have diamond deps)
    if !exec_nodes.is_empty() {
        let mut t = Tree::new(format!("{} {}",
            styled(phase::EXEC, phase_label::EXEC), dim(&format!("({} nodes)", exec_nodes.len()))));

        // Build the DAG and compute tiers using the same logic as the runtime
        let node_by_id: HashMap<&ContentId, &ResolvedNode> = exec_nodes.iter()
            .map(|n| (&n.id, *n)).collect();

        if let Ok((graph, _node_map)) = crate::ir::dag::build_exec_dag(ir) {
            if let Ok(tiers) = crate::ir::dag::compute_tiers(&graph) {
                for (tier_idx, tier) in tiers.iter().enumerate() {
                    // Tier header
                    let parallel = if tier.len() > 1 {
                        format!(" {}", dim(&format!("({} parallel)", tier.len())))
                    } else { String::new() };
                    let mut tier_tree = Tree::new(format!("{}tier {tier_idx}{RESET}{parallel}",
                        phase::EXEC_DIM));

                    for &node_idx in tier {
                        let content_id = &graph[node_idx];
                        if let Some(n) = node_by_id.get(content_id) {
                            let detail = if let ResolvedNativeNode::Command { name, .. } = &n.node {
                                cache.get_command(name).map(|c| {
                                    format!("  {} {}", fmt_exit(c.exit_code),
                                        dim(&format!("{:.3}s", c.wall_ms as f64 / 1000.0)))
                                }).unwrap_or_default()
                            } else { String::new() };

                            // Show parent edges for nodes with multiple parents (diamond deps)
                            let exec_ids: HashSet<&ContentId> = exec_nodes.iter().map(|n| &n.id).collect();
                            let parent_names: Vec<String> = n.parents.iter()
                                .filter(|p| exec_ids.contains(p))
                                .filter_map(|p| node_by_id.get(p))
                                .filter_map(|p| match &p.node {
                                    ResolvedNativeNode::Command { name, .. } => Some(name.clone()),
                                    _ => Some(input_label(p)),
                                })
                                .collect();
                            let parents_tag = if parent_names.len() > 1 {
                                format!("  {}", dim(&format!("\u{2190} {}", parent_names.join(", "))))
                            } else if !parent_names.is_empty() && tier_idx > 0 {
                                format!("  {}", dim(&format!("\u{2190} {}", parent_names[0])))
                            } else { String::new() };

                            tier_tree.push(Tree::new(format!("{} {} {}{}{}",
                                node_status_badge(&get_node_status(n, cache)),
                                node_type_badge(n), node_short_label(n),
                                detail, parents_tag)));
                        }
                    }
                    t.push(tier_tree);
                }
            }
        }
        root.push(t);
    }

    // Last run
    if let Some(lr) = cache.get_last_run() {
        let s = if lr.skipped { styled(status::SKIP, label::SKIPPED) }
            else if lr.exit_code == 0 { styled(status::FRESH, label::PASSED) }
            else { styled(status::FAILED, label::FAILED) };
        root.push(Tree::new(format!("{} {s} {}",
            dim(message::LAST_RUN),
            dim(&format!("{} ({:.3}s)", format_relative_time(&lr.ran_at), lr.duration_ms as f64 / 1000.0)))));
    }

    eprintln!("{root}");
}
