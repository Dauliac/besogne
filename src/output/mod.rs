use crate::ir::{BesogneIR, ResolvedInput, ResolvedNativeInput};
use crate::probe::ProbeResult;
use crate::runtime::cache::CachedCommand;
use crate::runtime::cli::LogFormat;
use crate::tracer::CommandResult;

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

/// Output renderer trait — human, CI, JSON all implement this
pub trait OutputRenderer {
    fn on_start(&mut self, ir: &BesogneIR);
    fn on_phase_start(&mut self, phase: &str, count: usize);
    fn on_probe_result(&mut self, input: &ResolvedInput, result: &ProbeResult, status: ProbeStatus);
    fn on_phase_end(&mut self, phase: &str);
    fn on_skip(&mut self, reason: &str);
    fn on_command_start(&mut self, name: &str, exec: &[String]);
    fn on_command_output(&mut self, name: &str, stdout: &str, stderr: &str);
    fn on_command_end(&mut self, name: &str, result: &CommandResult);
    /// Replay a cached command (skipped because inputs unchanged)
    fn on_command_cached(&mut self, name: &str, exec: &[String], cached: &CachedCommand);
    fn on_summary(&mut self, exit_code: i32, wall_ms: u64);
}

pub fn renderer_for_format(format: &LogFormat) -> Box<dyn OutputRenderer> {
    match format {
        LogFormat::Human => Box::new(HumanRenderer::new()),
        LogFormat::Json => Box::new(JsonRenderer::new()),
        LogFormat::Ci => Box::new(CiRenderer::new()),
    }
}

fn input_label(input: &ResolvedInput) -> String {
    match &input.input {
        ResolvedNativeInput::Env { name, .. } => format!("env:{name}"),
        ResolvedNativeInput::File { path, .. } => format!("file:{path}"),
        ResolvedNativeInput::Binary { name, .. } => format!("binary:{name}"),
        ResolvedNativeInput::Service { tcp, http, .. } => {
            format!("service:{}", tcp.as_deref().or(http.as_deref()).unwrap_or("?"))
        }
        ResolvedNativeInput::User { in_group, .. } => {
            format!("user:{}", in_group.as_deref().unwrap_or("current"))
        }
        ResolvedNativeInput::Platform { os, arch, .. } => {
            format!("platform:{}-{}", os.as_deref().unwrap_or("?"), arch.as_deref().unwrap_or("?"))
        }
        ResolvedNativeInput::Dns { host, .. } => format!("dns:{host}"),
        ResolvedNativeInput::Metric { metric, .. } => format!("metric:{metric}"),
        ResolvedNativeInput::Command { name, .. } => format!("command:{name}"),
    }
}

fn probe_detail(input: &ResolvedInput, result: &ProbeResult) -> String {
    match &input.input {
        ResolvedNativeInput::Binary {
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

        ResolvedNativeInput::Env { name, secret, .. } => {
            if *secret {
                format!("{name}=***")
            } else if let Some(val) = result.variables.get(name.as_str()) {
                let display = if val.len() > 60 { format!("{}...", &val[..57]) } else { val.clone() };
                format!("{name}={display}")
            } else {
                name.clone()
            }
        }

        ResolvedNativeInput::File { path, expect, .. } => {
            if let Some(exp) = expect { format!("{path} ({exp})") } else { path.clone() }
        }

        ResolvedNativeInput::Service { name, tcp, http, .. } => {
            let label = name.as_deref().unwrap_or("service");
            let target = tcp.as_deref().or(http.as_deref()).unwrap_or("?");
            format!("{label} {target}")
        }

        ResolvedNativeInput::User { in_group, .. } => {
            if let Some(user) = result.variables.get("USER_NAME") {
                match in_group {
                    Some(g) => format!("{user} in:{g}"),
                    None => user.clone(),
                }
            } else {
                in_group.as_deref().unwrap_or("current").to_string()
            }
        }

        ResolvedNativeInput::Platform { .. } => {
            let os = result.variables.get("PLATFORM_OS").map(|s| s.as_str()).unwrap_or("?");
            let arch = result.variables.get("PLATFORM_ARCH").map(|s| s.as_str()).unwrap_or("?");
            format!("{os}/{arch}")
        }

        ResolvedNativeInput::Dns { host, .. } => {
            let key = format!("DNS_{}", host.to_uppercase().replace(['.', '-'], "_"));
            if let Some(ip) = result.variables.get(&key) { format!("{host} → {ip}") } else { host.clone() }
        }

        ResolvedNativeInput::Metric { metric, .. } => {
            let key = format!("METRIC_{}", metric.to_uppercase().replace('.', "_"));
            if let Some(val) = result.variables.get(&key) { format!("{metric}={val}") } else { metric.clone() }
        }

        ResolvedNativeInput::Command { name, .. } => name.clone(),
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

// ── Human renderer ──────────────────────────────────────────────────

pub struct HumanRenderer;

impl HumanRenderer {
    pub fn new() -> Self { Self }
}

impl OutputRenderer for HumanRenderer {
    fn on_start(&mut self, ir: &BesogneIR) {
        eprintln!(
            "\x1b[1m{}\x1b[0m v{} — {}",
            ir.metadata.name, ir.metadata.version, ir.metadata.description
        );
    }

    fn on_phase_start(&mut self, _phase: &str, _count: usize) {}

    fn on_probe_result(&mut self, input: &ResolvedInput, result: &ProbeResult, status: ProbeStatus) {
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

    fn on_skip(&mut self, reason: &str) {
        eprintln!("\x1b[33mskip\x1b[0m {reason}");
    }

    fn on_command_start(&mut self, name: &str, exec: &[String]) {
        eprintln!("\n\x1b[1mexec\x1b[0m {name}: {}", exec.join(" "));
    }

    fn on_command_output(&mut self, _name: &str, stdout: &str, stderr: &str) {
        for line in stdout.lines() { eprintln!("    {line}"); }
        for line in stderr.lines() { eprintln!("    {line}"); }
    }

    fn on_command_end(&mut self, name: &str, result: &CommandResult) {
        let time = format!("\x1b[36m⏱ time:{:.3}s\x1b[0m", result.wall_ms as f64 / 1000.0);
        let cpu = if result.user_ms > 0 || result.sys_ms > 0 {
            let cores_used = if result.wall_ms > 0 {
                (result.user_ms + result.sys_ms) as f64 / result.wall_ms as f64
            } else { 0.0 };
            format!(
                "  \x1b[33m⚡ cpu:{:.2}s user + {:.2}s kernel ({:.1} cores)\x1b[0m",
                result.user_ms as f64 / 1000.0,
                result.sys_ms as f64 / 1000.0,
                cores_used,
            )
        } else { String::new() };
        let mem = if result.max_rss_kb > 0 {
            format!("  \x1b[35m🧠 memory:{}\x1b[0m", format_bytes(result.max_rss_kb * 1024))
        } else { String::new() };
        let disk = {
            let r = result.disk_read_bytes;
            let w = result.disk_write_bytes;
            if r > 0 || w > 0 {
                format!("  \x1b[34m💾 disk:read {} write {}\x1b[0m", format_bytes(r), format_bytes(w))
            } else { String::new() }
        };
        let net = {
            let r = result.net_read_bytes;
            let w = result.net_write_bytes;
            if r > 0 || w > 0 {
                format!("  \x1b[32m🌐 net:download {} upload {}\x1b[0m", format_bytes(r), format_bytes(w))
            } else { String::new() }
        };
        let procs = format!("  \x1b[31m🔀 processes:{}\x1b[0m", result.processes_spawned + 1);

        if result.exit_code == 0 {
            eprintln!("  \x1b[32mok\x1b[0m {name}  {time}{cpu}{mem}{disk}{net}{procs}");
        } else {
            eprintln!("  \x1b[31mfail\x1b[0m {name}  exit {}  {time}{cpu}{mem}{disk}{net}{procs}", result.exit_code);
        }
    }

    fn on_command_cached(&mut self, name: &str, exec: &[String], cached: &CachedCommand) {
        eprintln!(
            "\n\x1b[33mcached\x1b[0m {name}: {}  (ran {})",
            exec.join(" "),
            format_relative_time(&cached.ran_at),
        );
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
        let time = format!("time:{:.3}s", cached.wall_ms as f64 / 1000.0);
        let cpu = if cached.user_ms > 0 || cached.sys_ms > 0 {
            format!(
                "  cpu:{:.2}s user + {:.2}s kernel",
                cached.user_ms as f64 / 1000.0,
                cached.sys_ms as f64 / 1000.0,
            )
        } else {
            String::new()
        };
        let mem = if cached.max_rss_kb > 0 {
            format!("  memory:{:.1}MB", cached.max_rss_kb as f64 / 1024.0)
        } else {
            String::new()
        };
        eprintln!("  \x1b[33mcached\x1b[0m {name}  {time}{cpu}{mem}");
    }

    fn on_summary(&mut self, exit_code: i32, wall_ms: u64) {
        if exit_code == 0 {
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

    fn on_probe_result(&mut self, input: &ResolvedInput, result: &ProbeResult, status: ProbeStatus) {
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

    fn on_skip(&mut self, reason: &str) {
        self.emit(&serde_json::json!({"event": "skip", "reason": reason}));
    }

    fn on_command_start(&mut self, name: &str, exec: &[String]) {
        self.emit(&serde_json::json!({
            "event": "command_start",
            "phase": "exec",
            "name": name,
            "exec": exec,
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
        let cores_used = if result.wall_ms > 0 {
            (result.user_ms + result.sys_ms) as f64 / result.wall_ms as f64
        } else { 0.0 };
        self.emit(&serde_json::json!({
            "event": "command_end",
            "name": name,
            "exit_code": result.exit_code,
            "time_ms": result.wall_ms,
            "cpu_user_ms": result.user_ms,
            "cpu_kernel_ms": result.sys_ms,
            "cpu_cores_used": cores_used,
            "memory_bytes": result.max_rss_kb * 1024,
            "disk_read_bytes": result.disk_read_bytes,
            "disk_write_bytes": result.disk_write_bytes,
            "net_download_bytes": result.net_read_bytes,
            "net_upload_bytes": result.net_write_bytes,
            "processes": result.processes_spawned + 1,
            "context_switches_voluntary": result.voluntary_cs,
            "context_switches_involuntary": result.involuntary_cs,
        }));
    }

    fn on_command_cached(&mut self, name: &str, exec: &[String], cached: &CachedCommand) {
        self.emit(&serde_json::json!({
            "event": "command_cached",
            "name": name,
            "exec": exec,
            "exit_code": cached.exit_code,
            "time_ms": cached.wall_ms,
            "ran_at": cached.ran_at,
            "stdout": cached.stdout,
            "stderr": cached.stderr,
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

    fn on_probe_result(&mut self, input: &ResolvedInput, result: &ProbeResult, status: ProbeStatus) {
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

    fn on_skip(&mut self, reason: &str) {
        eprintln!("::notice::SKIP {reason}");
    }

    fn on_command_start(&mut self, name: &str, exec: &[String]) {
        eprintln!("::group::{name}: {}", exec.join(" "));
    }

    fn on_command_output(&mut self, _name: &str, stdout: &str, stderr: &str) {
        for line in stdout.lines() { eprintln!("    {line}"); }
        for line in stderr.lines() { eprintln!("    {line}"); }
    }

    fn on_command_end(&mut self, name: &str, result: &CommandResult) {
        let time = format!("{:.3}s", result.wall_ms as f64 / 1000.0);
        let mem = if result.max_rss_kb > 0 {
            format!("  memory:{}", format_bytes(result.max_rss_kb * 1024))
        } else { String::new() };
        let disk = {
            let r = result.disk_read_bytes;
            let w = result.disk_write_bytes;
            if r > 0 || w > 0 {
                format!("  disk:read {} write {}", format_bytes(r), format_bytes(w))
            } else { String::new() }
        };
        let cpu = if result.user_ms > 0 || result.sys_ms > 0 {
            let cores_used = if result.wall_ms > 0 {
                (result.user_ms + result.sys_ms) as f64 / result.wall_ms as f64
            } else { 0.0 };
            format!(
                "  cpu:{:.2}s user + {:.2}s kernel ({:.1} cores)",
                result.user_ms as f64 / 1000.0,
                result.sys_ms as f64 / 1000.0,
                cores_used,
            )
        } else { String::new() };
        let net = {
            let r = result.net_read_bytes;
            let w = result.net_write_bytes;
            if r > 0 || w > 0 {
                format!("  net:download {} upload {}", format_bytes(r), format_bytes(w))
            } else { String::new() }
        };
        let procs = format!("  processes:{}", result.processes_spawned + 1);
        if result.exit_code == 0 {
            eprintln!("  [PASS] {name}  {time}{cpu}{mem}{disk}{net}{procs}");
        } else {
            eprintln!("  [FAIL] {name}  exit {}  {time}{cpu}{mem}{disk}{net}{procs}", result.exit_code);
        }
        eprintln!("::endgroup::");
    }

    fn on_command_cached(&mut self, name: &str, exec: &[String], cached: &CachedCommand) {
        eprintln!("::group::[CACHE] {name}: {} (ran {})", exec.join(" "), cached.ran_at);
        if !cached.stdout.is_empty() {
            for line in cached.stdout.lines() { eprintln!("    {line}"); }
        }
        if !cached.stderr.is_empty() {
            for line in cached.stderr.lines() { eprintln!("    {line}"); }
        }
        let time = format!("{:.3}s", cached.wall_ms as f64 / 1000.0);
        eprintln!("  [CACHE] {name}  {time}");
        eprintln!("::endgroup::");
    }

    fn on_summary(&mut self, exit_code: i32, wall_ms: u64) {
        if exit_code == 0 {
            eprintln!("::notice::PASS {:.3}s", wall_ms as f64 / 1000.0);
        } else {
            eprintln!("::error::FAILED exit {exit_code}  {:.3}s", wall_ms as f64 / 1000.0);
        }
    }
}
