use super::{Probe, ProbeResult};
use std::collections::HashMap;

pub struct UserProbe<'a> {
    pub in_group: Option<&'a str>,
}

impl<'a> Probe for UserProbe<'a> {
    fn probe(&self) -> ProbeResult {
        let uid = unsafe { libc::getuid() };
        let gid = unsafe { libc::getgid() };

        let username = get_username(uid).unwrap_or_else(|| uid.to_string());
        let groups = get_groups();

        let mut variables = HashMap::new();
        variables.insert("USER_NAME".to_string(), username.clone());
        variables.insert("USER_UID".to_string(), uid.to_string());
        variables.insert("USER_GID".to_string(), gid.to_string());

        // Check group membership if requested
        if let Some(required_group) = self.in_group {
            if !groups.contains(&required_group.to_string()) {
                return ProbeResult {
                    success: false,
                    hash: blake3::hash(username.as_bytes()).to_hex().to_string(),
                    variables,
                    error: Some(format!(
                        "user '{username}' is not in group '{required_group}' (groups: {})",
                        groups.join(", ")
                    )),
                };
            }
        }

        let hash_input = format!("{uid}:{gid}:{}", groups.join(","));
        ProbeResult {
            success: true,
            hash: blake3::hash(hash_input.as_bytes()).to_hex().to_string(),
            variables,
            error: None,
        }
    }
}

fn get_username(uid: u32) -> Option<String> {
    // Read /etc/passwd for uid → username mapping
    let passwd = std::fs::read_to_string("/etc/passwd").ok()?;
    for line in passwd.lines() {
        let fields: Vec<&str> = line.split(':').collect();
        if fields.len() >= 3 {
            if let Ok(file_uid) = fields[2].parse::<u32>() {
                if file_uid == uid {
                    return Some(fields[0].to_string());
                }
            }
        }
    }
    None
}

fn get_groups() -> Vec<String> {
    let mut gids = vec![0i32; 64];
    let mut ngroups: libc::c_int = gids.len() as libc::c_int;

    let ret = unsafe { libc::getgroups(ngroups, gids.as_mut_ptr() as *mut libc::gid_t) };

    if ret < 0 {
        return vec![];
    }
    ngroups = ret;
    gids.truncate(ngroups as usize);

    // Map gids to names via /etc/group
    let group_file = std::fs::read_to_string("/etc/group").unwrap_or_default();
    let gid_to_name: HashMap<u32, String> = group_file
        .lines()
        .filter_map(|line| {
            let fields: Vec<&str> = line.split(':').collect();
            if fields.len() >= 3 {
                let gid = fields[2].parse::<u32>().ok()?;
                Some((gid, fields[0].to_string()))
            } else {
                None
            }
        })
        .collect();

    gids.iter()
        .map(|&g| {
            gid_to_name
                .get(&(g as u32))
                .cloned()
                .unwrap_or_else(|| g.to_string())
        })
        .collect()
}

