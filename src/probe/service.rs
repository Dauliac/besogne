use super::{Probe, ProbeResult};
use std::collections::HashMap;
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

pub struct ServiceProbe<'a> {
    pub tcp: Option<&'a str>,
    pub http: Option<&'a str>,
}

impl<'a> Probe for ServiceProbe<'a> {
    fn probe(&self) -> ProbeResult {
        if let Some(tcp_addr) = self.tcp {
            return probe_tcp(tcp_addr);
        }
        if let Some(http_url) = self.http {
            return probe_http(http_url);
        }
        ProbeResult {
            success: false,
            hash: String::new(),
            variables: HashMap::new(),
            error: Some("service input has neither tcp nor http".into()),
        }
    }
}

fn probe_tcp(addr: &str) -> ProbeResult {
    let timeout = Duration::from_secs(5);

    // Resolve and connect
    match addr.to_socket_addrs() {
        Ok(mut addrs) => {
            if let Some(socket_addr) = addrs.next() {
                match TcpStream::connect_timeout(&socket_addr, timeout) {
                    Ok(_) => ProbeResult {
                        success: true,
                        hash: blake3::hash(addr.as_bytes()).to_hex().to_string(),
                        variables: HashMap::new(),
                        error: None,
                    },
                    Err(e) => ProbeResult {
                        success: false,
                        hash: String::new(),
                        variables: HashMap::new(),
                        error: Some(format!("tcp connect to {addr} failed: {e}")),
                    },
                }
            } else {
                ProbeResult {
                    success: false,
                    hash: String::new(),
                    variables: HashMap::new(),
                    error: Some(format!("cannot resolve {addr}: no addresses")),
                }
            }
        }
        Err(e) => ProbeResult {
            success: false,
            hash: String::new(),
            variables: HashMap::new(),
            error: Some(format!("cannot resolve {addr}: {e}")),
        },
    }
}

fn probe_http(url: &str) -> ProbeResult {
    // Minimal HTTP/1.1 GET — no external crate
    // Parse url: expect http://host:port/path or https://...
    let url = url.trim();

    let (host, port, path) = match parse_http_url(url) {
        Some(v) => v,
        None => {
            return ProbeResult {
                success: false,
                hash: String::new(),
                variables: HashMap::new(),
                error: Some(format!("cannot parse HTTP URL: {url}")),
            };
        }
    };

    let addr = format!("{host}:{port}");
    let timeout = Duration::from_secs(5);

    match addr.to_socket_addrs() {
        Ok(mut addrs) => {
            if let Some(socket_addr) = addrs.next() {
                match TcpStream::connect_timeout(&socket_addr, timeout) {
                    Ok(mut stream) => {
                        use std::io::{Read, Write};
                        let request = format!(
                            "GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n"
                        );
                        if stream.write_all(request.as_bytes()).is_err() {
                            return ProbeResult {
                                success: false,
                                hash: String::new(),
                                variables: HashMap::new(),
                                error: Some(format!("http write failed to {url}")),
                            };
                        }

                        let mut response = Vec::new();
                        let _ = stream.read_to_end(&mut response);
                        let response_str = String::from_utf8_lossy(&response);

                        // Parse status code from first line: HTTP/1.1 200 OK
                        let status = response_str
                            .lines()
                            .next()
                            .and_then(|line| line.split_whitespace().nth(1))
                            .and_then(|s| s.parse::<u16>().ok())
                            .unwrap_or(0);

                        let mut variables = HashMap::new();
                        variables.insert("HTTP_STATUS".to_string(), status.to_string());

                        ProbeResult {
                            success: status >= 200 && status < 400,
                            hash: blake3::hash(&response).to_hex().to_string(),
                            variables,
                            error: if status >= 400 {
                                Some(format!("http {url} returned status {status}"))
                            } else {
                                None
                            },
                        }
                    }
                    Err(e) => ProbeResult {
                        success: false,
                        hash: String::new(),
                        variables: HashMap::new(),
                        error: Some(format!("http connect to {url} failed: {e}")),
                    },
                }
            } else {
                ProbeResult {
                    success: false,
                    hash: String::new(),
                    variables: HashMap::new(),
                    error: Some(format!("cannot resolve {host}")),
                }
            }
        }
        Err(e) => ProbeResult {
            success: false,
            hash: String::new(),
            variables: HashMap::new(),
            error: Some(format!("cannot resolve {host}: {e}")),
        },
    }
}

fn parse_http_url(url: &str) -> Option<(String, u16, String)> {
    let url = url.strip_prefix("http://").or_else(|| url.strip_prefix("https://"))?;
    let (host_port, path) = if let Some(idx) = url.find('/') {
        (&url[..idx], &url[idx..])
    } else {
        (url, "/")
    };

    let (host, port) = if let Some(idx) = host_port.rfind(':') {
        let port = host_port[idx + 1..].parse::<u16>().ok()?;
        (&host_port[..idx], port)
    } else {
        (host_port, 80)
    };

    Some((host.to_string(), port, path.to_string()))
}
