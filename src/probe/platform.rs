use super::{Probe, ProbeResult};
use std::collections::HashMap;

pub struct PlatformProbe<'a> {
    pub expected_os: Option<&'a str>,
    pub expected_arch: Option<&'a str>,
}

impl<'a> Probe for PlatformProbe<'a> {
    fn probe(&self) -> ProbeResult {
        let os = std::env::consts::OS;       // "linux", "macos", "windows"
        let arch = std::env::consts::ARCH;   // "x86_64", "aarch64"

        let kernel = get_kernel_version();

        let mut variables = HashMap::new();
        variables.insert("PLATFORM_OS".to_string(), os.to_string());
        variables.insert("PLATFORM_ARCH".to_string(), arch.to_string());
        if let Some(k) = &kernel {
            variables.insert("PLATFORM_KERNEL".to_string(), k.clone());
        }

        // Validate if expected
        if let Some(expected_os) = self.expected_os {
            if os != expected_os {
                return ProbeResult {
                    success: false,
                    hash: String::new(),
                    variables,
                    error: Some(format!("expected os={expected_os}, got {os}")),
                };
            }
        }

        if let Some(expected_arch) = self.expected_arch {
            if arch != expected_arch {
                return ProbeResult {
                    success: false,
                    hash: String::new(),
                    variables,
                    error: Some(format!("expected arch={expected_arch}, got {arch}")),
                };
            }
        }

        let hash_input = format!("{os}:{arch}:{}", kernel.as_deref().unwrap_or(""));
        ProbeResult {
            success: true,
            hash: blake3::hash(hash_input.as_bytes()).to_hex().to_string(),
            variables,
            error: None,
        }
    }
}

fn get_kernel_version() -> Option<String> {
    #[cfg(target_os = "linux")]
    {
        std::fs::read_to_string("/proc/version")
            .ok()
            .and_then(|v| v.split_whitespace().nth(2).map(String::from))
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}
