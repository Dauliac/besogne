use super::{Probe, ProbeResult};
use std::collections::HashMap;
use std::net::ToSocketAddrs;

pub struct DnsProbe<'a> {
    pub host: &'a str,
    pub expect: Option<&'a str>,
}

impl<'a> Probe for DnsProbe<'a> {
    fn probe(&self) -> ProbeResult {
        // Resolve hostname — use port 0 to just do DNS resolution
        let lookup = format!("{}:0", self.host);

        match lookup.to_socket_addrs() {
            Ok(addrs) => {
                let ips: Vec<String> = addrs.map(|a| a.ip().to_string()).collect();

                if ips.is_empty() {
                    return ProbeResult {
                        success: false,
                        hash: String::new(),
                        variables: HashMap::new(),
                        error: Some(format!("dns: {} resolved to no addresses", self.host)),
                    };
                }

                let sanitized_name = self.host.replace('.', "_").replace('-', "_").to_uppercase();
                let mut variables = HashMap::new();
                variables.insert(format!("DNS_{sanitized_name}"), ips[0].clone());

                // Validate expected IP if specified
                if let Some(expected) = self.expect {
                    if !ips.contains(&expected.to_string()) {
                        return ProbeResult {
                            success: false,
                            hash: blake3::hash(ips.join(",").as_bytes()).to_hex().to_string(),
                            variables,
                            error: Some(format!(
                                "dns: {} resolved to [{}], expected {}",
                                self.host,
                                ips.join(", "),
                                expected
                            )),
                        };
                    }
                }

                ProbeResult {
                    success: true,
                    hash: blake3::hash(ips.join(",").as_bytes()).to_hex().to_string(),
                    variables,
                    error: None,
                }
            }
            Err(e) => ProbeResult {
                success: false,
                hash: String::new(),
                variables: HashMap::new(),
                error: Some(format!("dns: cannot resolve {}: {e}", self.host)),
            },
        }
    }
}
