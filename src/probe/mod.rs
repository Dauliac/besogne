pub mod binary;
pub mod dns;
pub mod env;
pub mod file;
pub mod metric;
pub mod platform;
pub mod service;
pub mod source;
pub mod user;

use crate::ir::ResolvedNativeNode;
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

        ResolvedNativeNode::Service { tcp, http, .. } => service::ServiceProbe {
            tcp: tcp.as_deref(),
            http: http.as_deref(),
        }
        .probe(),

        ResolvedNativeNode::User { in_group, .. } => user::UserProbe {
            in_group: in_group.as_deref(),
        }
        .probe(),

        ResolvedNativeNode::Platform { os, arch, .. } => platform::PlatformProbe {
            expected_os: os.as_deref(),
            expected_arch: arch.as_deref(),
        }
        .probe(),

        ResolvedNativeNode::Dns { host, expect, .. } => dns::DnsProbe {
            host,
            expect: expect.as_deref(),
        }
        .probe(),

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
    fn test_user_probe_current() {
        let result = user::UserProbe { in_group: None }.probe();
        assert!(result.success);
        assert!(result.variables.contains_key("USER_NAME"));
        assert!(result.variables.contains_key("USER_UID"));
    }

    #[test]
    fn test_user_probe_nonexistent_group() {
        let result = user::UserProbe {
            in_group: Some("besogne_fake_group_xyz"),
        }
        .probe();
        assert!(!result.success);
        assert!(result.error.unwrap().contains("not in group"));
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
            ResolvedNativeNode::User {
                in_group: None,
            },
            ResolvedNativeNode::Platform {
                os: None,
                arch: None,
            },
            ResolvedNativeNode::Dns {
                host: "localhost".into(),
                expect: None,
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
}
