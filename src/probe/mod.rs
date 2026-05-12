pub mod binary;
pub mod dns;
pub mod env;
pub mod file;
pub mod metric;
pub mod platform;
pub mod service;
pub mod source;


use crate::ir::{ResolvedNativeNode, RetryResolved};
use std::collections::HashMap;

/// Result of probing an input
#[derive(Debug, Clone)]
pub struct ProbeResult {
    pub success: bool,
    pub hash: String,
    pub variables: HashMap<String, String>,
    pub error: Option<String>,
}

/// Probe trait — each native type implements this
pub trait Probe {
    fn probe(&self) -> ProbeResult;
}

/// Dispatch to the right probe implementation
pub fn probe_input(input: &ResolvedNativeNode) -> ProbeResult {
    match input {
        ResolvedNativeNode::Env {
            name,
            value,
            secret,
            ..
        } => env::EnvProbe {
            name,
            value: value.as_deref(),
            secret: *secret,
        }
        .probe(),

        ResolvedNativeNode::File { path, .. } => file::FileProbe { path }.probe(),

        ResolvedNativeNode::Binary {
            name,
            path,
            source,
            resolved_path,
            resolved_version,
            binary_hash,
            ..
        } => binary::BinaryProbe {
            name,
            path: path.as_deref(),
            source: source.as_ref(),
            resolved_path: resolved_path.as_deref(),
            resolved_version: resolved_version.as_deref(),
            binary_hash: binary_hash.as_deref(),
        }
        .probe(),

        ResolvedNativeNode::Service { tcp, http, retry, .. } => {
            let probe = || service::ServiceProbe {
                tcp: tcp.as_deref(),
                http: http.as_deref(),
            }.probe();
            probe_with_retry(probe, retry.as_ref())
        }



        ResolvedNativeNode::Platform { os, arch, .. } => platform::PlatformProbe {
            expected_os: os.as_deref(),
            expected_arch: arch.as_deref(),
        }
        .probe(),

        ResolvedNativeNode::Dns { host, expect, retry, .. } => {
            let probe = || dns::DnsProbe {
                host,
                expect: expect.as_deref(),
            }.probe();
            probe_with_retry(probe, retry.as_ref())
        }

        ResolvedNativeNode::Metric { metric, path, .. } => metric::MetricProbe {
            metric,
            path: path.as_deref(),
        }
        .probe(),

        ResolvedNativeNode::Source {
            format,
            path,
            select,
            sealed_env,
        } => source::SourceProbe {
            format,
            path: path.as_deref(),
            select: select.as_deref(),
            sealed_env: sealed_env.as_ref(),
        }
        .probe(),

        ResolvedNativeNode::Std { .. } | ResolvedNativeNode::Command { .. } => {
            // Commands and std nodes are validated by the runtime, not probed
            ProbeResult {
                success: true,
                hash: String::new(),
                variables: HashMap::new(),
                error: None,
            }
        }
    }
}

/// Execute a probe with optional retry logic.
/// On failure, retries according to the retry config with backoff and timeout.
pub fn probe_with_retry<F>(probe_fn: F, retry: Option<&RetryResolved>) -> ProbeResult
where
    F: Fn() -> ProbeResult,
{
    let retry = match retry {
        Some(r) => r,
        None => return probe_fn(),
    };

    let deadline = retry.timeout_ms.map(|t| std::time::Instant::now() + std::time::Duration::from_millis(t));
    let mut last_result = ProbeResult {
        success: false,
        hash: String::new(),
        variables: HashMap::new(),
        error: Some("no attempts made".into()),
    };

    for attempt in 0..retry.attempts {
        if let Some(dl) = deadline {
            if std::time::Instant::now() >= dl {
                last_result.error = Some(format!(
                    "retry timeout after {} attempts: {}",
                    attempt,
                    last_result.error.as_deref().unwrap_or("unknown"),
                ));
                return last_result;
            }
        }

        last_result = probe_fn();
        if last_result.success {
            return last_result;
        }

        // Don't sleep after the last attempt
        if attempt + 1 < retry.attempts {
            let delay = retry.delay_for_attempt(attempt);

            // Respect timeout deadline
            let delay = if let Some(dl) = deadline {
                let remaining = dl.saturating_duration_since(std::time::Instant::now());
                delay.min(remaining)
            } else {
                delay
            };

            if !delay.is_zero() {
                eprintln!("  {} retry {}/{} in {}",
                    crate::output::style::styled(crate::output::style::status::PENDING, "retry"),
                    attempt + 1, retry.attempts,
                    format_duration(delay));
                std::thread::sleep(delay);
            }
        }
    }

    last_result.error = Some(format!(
        "failed after {} attempts: {}",
        retry.attempts,
        last_result.error.as_deref().unwrap_or("unknown"),
    ));
    last_result
}

fn format_duration(d: std::time::Duration) -> String {
    let ms = d.as_millis();
    if ms < 1000 {
        format!("{}ms", ms)
    } else if ms < 60_000 {
        format!("{:.1}s", ms as f64 / 1000.0)
    } else {
        format!("{:.1}m", ms as f64 / 60_000.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_env_probe_reads_existing_var() {
        std::env::set_var("BESOGNE_TEST_VAR", "hello");
        let result = env::EnvProbe {
            name: "BESOGNE_TEST_VAR",
            value: None,
            secret: false,
        }
        .probe();
        assert!(result.success);
        assert_eq!(
            result.variables.get("BESOGNE_TEST_VAR"),
            Some(&"hello".to_string())
        );
        assert!(!result.hash.is_empty());
        std::env::remove_var("BESOGNE_TEST_VAR");
    }

    #[test]
    fn test_env_probe_missing_var_fails() {
        std::env::remove_var("BESOGNE_NONEXISTENT_VAR");
        let result = env::EnvProbe {
            name: "BESOGNE_NONEXISTENT_VAR",
            value: None,
            secret: false,
        }
        .probe();
        assert!(!result.success);
        assert!(result.error.is_some());
    }

    #[test]
    fn test_env_probe_with_value_sets_var() {
        let result = env::EnvProbe {
            name: "BESOGNE_COMPUTED_VAR",
            value: Some("/custom/path"),
            secret: false,
        }
        .probe();
        assert!(result.success);
        assert_eq!(std::env::var("BESOGNE_COMPUTED_VAR").unwrap(), "/custom/path");
        assert_eq!(
            result.variables.get("BESOGNE_COMPUTED_VAR"),
            Some(&"/custom/path".to_string())
        );
        std::env::remove_var("BESOGNE_COMPUTED_VAR");
    }

    #[test]
    fn test_env_probe_secret_hides_value() {
        std::env::set_var("BESOGNE_SECRET_VAR", "s3cret");
        let result = env::EnvProbe {
            name: "BESOGNE_SECRET_VAR",
            value: None,
            secret: true,
        }
        .probe();
        assert!(result.success);
        assert!(result.variables.is_empty()); // secret not exposed
        std::env::remove_var("BESOGNE_SECRET_VAR");
    }

    #[test]
    fn test_file_probe_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("test.txt");
        std::fs::write(&file_path, "hello").unwrap();

        let result = file::FileProbe {
            path: file_path.to_str().unwrap(),
        }
        .probe();
        assert!(result.success);
        assert!(!result.hash.is_empty());
    }

    #[test]
    fn test_file_probe_missing_file() {
        let result = file::FileProbe {
            path: "/tmp/besogne_nonexistent_file_12345",
        }
        .probe();
        assert!(!result.success);
        assert!(result.error.unwrap().contains("not found"));
    }

    #[test]
    fn test_file_probe_directory() {
        let dir = tempfile::tempdir().unwrap();
        let result = file::FileProbe {
            path: dir.path().to_str().unwrap(),
        }
        .probe();
        assert!(result.success);
        assert!(!result.hash.is_empty());
    }

    #[test]
    fn test_file_probe_content_hash_changes() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("test.txt");

        std::fs::write(&file_path, "content1").unwrap();
        let r1 = file::FileProbe {
            path: file_path.to_str().unwrap(),
        }
        .probe();

        std::fs::write(&file_path, "content2").unwrap();
        let r2 = file::FileProbe {
            path: file_path.to_str().unwrap(),
        }
        .probe();

        assert!(r1.success && r2.success);
        assert_ne!(r1.hash, r2.hash); // different content = different hash
    }

    #[test]
    fn test_binary_probe_finds_echo() {
        let result = binary::BinaryProbe {
            name: "echo",
            path: None,
            source: None,
            resolved_path: None,
            resolved_version: None,
            binary_hash: None,
        }
        .probe();
        // echo might be a shell builtin, but /usr/bin/echo should exist on Linux
        // Don't hard-fail on this — it depends on the system
        if result.success {
            assert!(result.variables.contains_key("ECHO"));
        }
    }

    #[test]
    fn test_binary_probe_missing_binary() {
        let result = binary::BinaryProbe {
            name: "besogne_nonexistent_binary_xyz",
            path: None,
            source: None,
            resolved_path: None,
            resolved_version: None,
            binary_hash: None,
        }
        .probe();
        assert!(!result.success);
        assert!(result.error.unwrap().contains("not found"));
    }

    #[test]
    fn test_binary_probe_absolute_path() {
        // /bin/sh should exist on any Unix system
        let result = binary::BinaryProbe {
            name: "sh",
            path: Some("/bin/sh"),
            source: None,
            resolved_path: Some("/bin/sh"),
            resolved_version: None,
            binary_hash: None,
        }
        .probe();
        assert!(result.success);
        assert!(result.variables.contains_key("SH"));
        assert!(!result.hash.is_empty());
    }

    #[test]
    fn test_platform_probe_current() {
        let result = platform::PlatformProbe {
            expected_os: None,
            expected_arch: None,
        }
        .probe();
        assert!(result.success);
        assert!(result.variables.contains_key("PLATFORM_OS"));
        assert!(result.variables.contains_key("PLATFORM_ARCH"));
    }

    #[test]
    fn test_platform_probe_correct_os() {
        let result = platform::PlatformProbe {
            expected_os: Some(std::env::consts::OS),
            expected_arch: None,
        }
        .probe();
        assert!(result.success);
    }

    #[test]
    fn test_platform_probe_wrong_os() {
        let result = platform::PlatformProbe {
            expected_os: Some("fakeos"),
            expected_arch: None,
        }
        .probe();
        assert!(!result.success);
        assert!(result.error.unwrap().contains("expected os=fakeos"));
    }


    #[test]
    fn test_dns_probe_localhost() {
        let result = dns::DnsProbe {
            host: "localhost",
            expect: None,
        }
        .probe();
        assert!(result.success);
        assert!(result.variables.contains_key("DNS_LOCALHOST"));
    }

    #[test]
    fn test_dns_probe_nonexistent_host() {
        let result = dns::DnsProbe {
            host: "this.host.does.not.exist.besogne.invalid",
            expect: None,
        }
        .probe();
        assert!(!result.success);
    }

    #[test]
    fn test_metric_probe_cpu_count() {
        let result = metric::MetricProbe {
            metric: "cpu.count",
            path: None,
        }
        .probe();
        assert!(result.success);
        assert!(result.variables.contains_key("METRIC_CPU_COUNT"));
    }

    #[test]
    fn test_metric_probe_disk_available() {
        let result = metric::MetricProbe {
            metric: "disk.available_gb",
            path: Some("/"),
        }
        .probe();
        assert!(result.success);
    }

    #[test]
    fn test_metric_probe_unknown() {
        let result = metric::MetricProbe {
            metric: "unknown.metric",
            path: None,
        }
        .probe();
        assert!(!result.success);
    }

    #[test]
    fn test_service_probe_tcp_unreachable() {
        // Port 1 should be unreachable
        let result = service::ServiceProbe {
            tcp: Some("127.0.0.1:1"),
            http: None,
        }
        .probe();
        assert!(!result.success);
    }

    #[test]
    fn test_probe_dispatch_all_types() {
        // Verify dispatch works for every variant without panicking
        let test_nodes = vec![
            ResolvedNativeNode::Env {
                name: "HOME".into(),
                value: None,
                secret: false,
            },
            ResolvedNativeNode::File {
                path: "/tmp".into(),
                expect: None,
                permissions: None,
            },
            ResolvedNativeNode::Binary {
                name: "sh".into(),
                path: Some("/bin/sh".into()),
                version_constraint: None,
                parents: vec![],
                source: None,
                resolved_path: Some("/bin/sh".into()),
                resolved_version: None,
                binary_hash: None,
            },

            ResolvedNativeNode::Platform {
                os: None,
                arch: None,
            },
            ResolvedNativeNode::Dns {
                host: "localhost".into(),
                expect: None,
                retry: None,
            },
            ResolvedNativeNode::Metric {
                metric: "cpu.count".into(),
                path: None,
            },
            ResolvedNativeNode::Command {
                name: "test".into(),
                run: vec!["echo".into()],
                env: HashMap::new(),
                side_effects: false,
                workdir: None,
                force_args: vec![],
                debug_args: vec![],
                retry: None,
            },
            ResolvedNativeNode::Source {
                format: "dotenv".into(),
                path: None,
                select: None,
                sealed_env: None,
            },
        ];

        for node in &test_nodes {
            let result = probe_input(node);
            // Just verify no panics — some may fail depending on system
            let _ = result;
        }
    }

    #[test]
    fn test_probe_with_retry_succeeds_first_try() {
        let retry = RetryResolved {
            attempts: 3,
            interval_ms: 10,
            backoff: crate::ir::RetryBackoff::Fixed,
            max_interval_ms: None,
            timeout_ms: None,
        };
        let result = probe_with_retry(|| ProbeResult {
            success: true,
            hash: "abc".into(),
            variables: HashMap::new(),
            error: None,
        }, Some(&retry));
        assert!(result.success);
    }

    #[test]
    fn test_probe_with_retry_fails_then_succeeds() {
        use std::sync::atomic::{AtomicU32, Ordering};
        let counter = AtomicU32::new(0);
        let retry = RetryResolved {
            attempts: 5,
            interval_ms: 10,
            backoff: crate::ir::RetryBackoff::Fixed,
            max_interval_ms: None,
            timeout_ms: None,
        };
        let result = probe_with_retry(|| {
            let n = counter.fetch_add(1, Ordering::SeqCst);
            if n < 2 {
                ProbeResult {
                    success: false,
                    hash: String::new(),
                    variables: HashMap::new(),
                    error: Some("not ready".into()),
                }
            } else {
                ProbeResult {
                    success: true,
                    hash: "ok".into(),
                    variables: HashMap::new(),
                    error: None,
                }
            }
        }, Some(&retry));
        assert!(result.success);
        assert_eq!(counter.load(Ordering::SeqCst), 3); // 2 failures + 1 success
    }

    #[test]
    fn test_probe_with_retry_exhausts_attempts() {
        let retry = RetryResolved {
            attempts: 3,
            interval_ms: 10,
            backoff: crate::ir::RetryBackoff::Fixed,
            max_interval_ms: None,
            timeout_ms: None,
        };
        let result = probe_with_retry(|| ProbeResult {
            success: false,
            hash: String::new(),
            variables: HashMap::new(),
            error: Some("down".into()),
        }, Some(&retry));
        assert!(!result.success);
        assert!(result.error.unwrap().contains("failed after 3 attempts"));
    }

    #[test]
    fn test_probe_with_retry_none_skips_retry() {
        let result = probe_with_retry(|| ProbeResult {
            success: false,
            hash: String::new(),
            variables: HashMap::new(),
            error: Some("fail".into()),
        }, None);
        assert!(!result.success);
        // No retry wrapper text
        assert_eq!(result.error.unwrap(), "fail");
    }
}
