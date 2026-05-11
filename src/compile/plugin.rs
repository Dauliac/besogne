use crate::compile::nickel;
use crate::manifest::{Node, Manifest};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Resolve all plugin nodes in a manifest, returning expanded native nodes.
/// Plugin nodes are replaced by the plugin's `nodes` map (same format as manifest).
pub fn expand_plugins(
    manifest: &Manifest,
    manifest_path: &Path,
) -> Result<HashMap<String, Node>, String> {
    let mut expanded = HashMap::new();

    for (key, input) in &manifest.nodes {
        match input {
            Node::Plugin(plugin_input) => {
                // The map key IS the plugin reference (e.g., "coreutils/shell")
                let (namespace, _) = parse_plugin_ref(key)?;

                // Default to "builtin" if namespace not declared in plugins map
                let source = manifest.plugins
                    .get(&namespace)
                    .map(|s| s.as_str())
                    .unwrap_or("builtin");

                let mut visited = Vec::new();
                let produced = expand_plugin_json(
                    key,
                    &plugin_input.overrides,
                    source,
                    manifest_path,
                    &mut visited,
                )?;

                // Collect all sub-keys for final parents rewriting
                let sub_keys: Vec<String> =
                    produced.iter().map(|(k, _)| k.clone()).collect();

                for (sub_key, mut value) in produced {
                    // Final parents rewriting: prefix sibling refs with manifest-level key
                    rewrite_parents_json(&mut value, &sub_keys, key);

                    let full_key = format!("{key}.{sub_key}");
                    let node: Node = serde_json::from_value(value.clone()).map_err(|e| {
                        format!(
                            "plugin '{key}' node '{sub_key}': {e}\n  value: {}",
                            serde_json::to_string_pretty(&value).unwrap_or_default()
                        )
                    })?;
                    if expanded.contains_key(&full_key) {
                        return Err(format!(
                            "plugin '{key}' produces duplicate node '{full_key}'"
                        ));
                    }
                    expanded.insert(full_key, node);
                }
            }
            _ => {
                expanded.insert(key.clone(), input.clone());
            }
        }
    }

    Ok(expanded)
}

fn parse_plugin_ref(plugin_ref: &str) -> Result<(String, String), String> {
    let parts: Vec<&str> = plugin_ref.splitn(2, '/').collect();
    if parts.len() != 2 {
        return Err(format!(
            "invalid plugin reference '{plugin_ref}': expected 'namespace/name' (e.g., 'docker/daemon')"
        ));
    }
    Ok((parts[0].to_string(), parts[1].to_string()))
}

/// Recursively expand a plugin into a flat list of (sub_key, JSON) pairs.
/// A plugin is a manifest: its `nodes` map is extracted and expanded.
/// Nested `type: "plugin"` nodes are recursively expanded.
fn expand_plugin_json(
    plugin_ref: &str,
    overrides: &Option<HashMap<String, serde_json::Value>>,
    source: &str,
    manifest_path: &Path,
    visited: &mut Vec<String>,
) -> Result<Vec<(String, serde_json::Value)>, String> {
    // Cycle detection
    if visited.contains(&plugin_ref.to_string()) {
        return Err(format!(
            "circular plugin: {} \u{2192} {}",
            visited.join(" \u{2192} "),
            plugin_ref
        ));
    }
    visited.push(plugin_ref.to_string());

    // Load plugin nodes
    let plugin_path = resolve_plugin_path(source, plugin_ref, manifest_path)?;
    let mut nodes = load_plugin_nodes(&plugin_path)?;

    // Apply per-node overrides (shallow merge into matching nodes)
    if let Some(overrides) = overrides {
        for (node_key, partial) in overrides {
            if let Some(base) = nodes.get_mut(node_key) {
                json_shallow_merge(base, partial);
            } else {
                // Override adds a new node to the plugin
                nodes.insert(node_key.clone(), partial.clone());
            }
        }
    }

    // Sort keys for deterministic expansion
    let mut entries: Vec<(String, serde_json::Value)> = nodes.into_iter().collect();
    entries.sort_by(|a, b| a.0.cmp(&b.0));

    // Expand: nested plugins → recurse, native nodes → keep
    let mut produced: Vec<(String, serde_json::Value)> = Vec::new();
    for (key, value) in entries {
        let is_plugin = value.get("type").and_then(|t| t.as_str()) == Some("plugin");
        if is_plugin {
            let sub_ref = value
                .get("plugin")
                .and_then(|p| p.as_str())
                .ok_or_else(|| {
                    format!("plugin node '{key}' in '{plugin_ref}' missing 'plugin' field")
                })?;
            let sub_overrides = value
                .get("overrides")
                .and_then(|o| serde_json::from_value(o.clone()).ok());

            let sub_nodes = expand_plugin_json(
                sub_ref,
                &sub_overrides,
                source,
                manifest_path,
                visited,
            )?;

            // Collect nested sub-keys so we can prefix their parent references
            let nested_keys: Vec<String> =
                sub_nodes.iter().map(|(k, _)| k.clone()).collect();

            for (sub_key, mut sub_value) in sub_nodes {
                prefix_parents(&mut sub_value, &key, &nested_keys);
                produced.push((format!("{key}.{sub_key}"), sub_value));
            }
        } else {
            let mut v = value;
            inject_name_json(&mut v, &key);
            produced.push((key, v));
        }
    }

    visited.pop();
    Ok(produced)
}

/// Load a plugin's nodes map from disk.
/// JSON plugins use manifest format (`nodes: { ... }`).
/// Nickel plugins return arrays (legacy — converted via derive_sub_key).
fn load_plugin_nodes(
    plugin_path: &Path,
) -> Result<HashMap<String, serde_json::Value>, String> {
    let ext = plugin_path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");

    if ext == "ncl" {
        // Nickel plugins return arrays — convert to named map
        let arr = nickel::eval_plugin(plugin_path, &HashMap::new())?;
        Ok(arr
            .into_iter()
            .enumerate()
            .map(|(idx, v)| (derive_sub_key(&v, idx), v))
            .collect())
    } else {
        let content = std::fs::read_to_string(plugin_path)
            .map_err(|e| format!("cannot read plugin {}: {e}", plugin_path.display()))?;
        let raw: serde_json::Value = serde_json::from_str(&content)
            .map_err(|e| format!("invalid plugin JSON {}: {e}", plugin_path.display()))?;

        let nodes_obj = raw
            .get("nodes")
            .and_then(|n| n.as_object())
            .ok_or_else(|| {
                format!("plugin '{}' missing 'nodes' map", plugin_path.display())
            })?;

        Ok(nodes_obj
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect())
    }
}

// ── Helpers ──

/// For binary/env nodes, inject `name` from the map key if not already set.
/// This mirrors manifest behavior where the key IS the name.
fn inject_name_json(value: &mut serde_json::Value, key: &str) {
    if let Some(obj) = value.as_object_mut() {
        let node_type = obj
            .get("type")
            .and_then(|t| t.as_str())
            .unwrap_or("");
        if matches!(node_type, "binary" | "env") && !obj.contains_key("name") {
            obj.insert(
                "name".to_string(),
                serde_json::Value::String(key.to_string()),
            );
        }
    }
}

/// Shallow merge: insert all fields from patch into base (overwriting on conflict).
fn json_shallow_merge(
    base: &mut serde_json::Value,
    patch: &serde_json::Value,
) {
    if let (Some(base_obj), Some(patch_obj)) = (base.as_object_mut(), patch.as_object()) {
        for (k, v) in patch_obj {
            base_obj.insert(k.clone(), v.clone());
        }
    }
}

/// Prefix parent references that match nested sub-keys.
/// Used when a nested plugin's nodes are being embedded under a key.
fn prefix_parents(
    value: &mut serde_json::Value,
    prefix: &str,
    nested_keys: &[String],
) {
    if let Some(obj) = value.as_object_mut() {
        if let Some(serde_json::Value::Array(parents)) = obj.get_mut("parents") {
            for dep in parents.iter_mut() {
                if let serde_json::Value::String(dep_name) = dep {
                    if nested_keys.contains(dep_name) {
                        *dep_name = format!("{prefix}.{dep_name}");
                    }
                }
            }
        }
    }
}

/// Rewrite parent references: if a ref matches a sibling sub-key, prefix with input_key.
fn rewrite_parents_json(
    value: &mut serde_json::Value,
    sub_keys: &[String],
    input_key: &str,
) {
    if let Some(obj) = value.as_object_mut() {
        if let Some(serde_json::Value::Array(parents)) = obj.get_mut("parents") {
            for dep in parents.iter_mut() {
                if let serde_json::Value::String(dep_name) = dep {
                    if sub_keys.contains(dep_name) {
                        *dep_name = format!("{input_key}.{dep_name}");
                    }
                }
            }
        }
    }
}

/// Derive a sub-key from a JSON node value (fallback for Nickel array plugins).
fn derive_sub_key(value: &serde_json::Value, idx: usize) -> String {
    if let Some(name) = value.get("name").and_then(|n| n.as_str()) {
        name.to_string()
    } else if let Some(typ) = value.get("type").and_then(|t| t.as_str()) {
        if let Some(path) = value.get("path").and_then(|p| p.as_str()) {
            format!("{typ}-{}", path.replace('/', "-").replace('.', "-"))
        } else {
            format!("{typ}-{idx}")
        }
    } else {
        format!("{idx}")
    }
}

/// Resolve a plugin reference to a file path on disk.
fn resolve_plugin_path(
    source: &str,
    plugin_ref: &str,
    manifest_path: &Path,
) -> Result<PathBuf, String> {
    let (namespace, name) = parse_plugin_ref(plugin_ref)?;

    match source {
        "builtin" => {
            let plugins_dir = std::env::var("BESOGNE_PLUGINS_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|_| {
                    let manifest_dir =
                        manifest_path.parent().unwrap_or(Path::new("."));
                    let candidate = manifest_dir.join("plugins");
                    if candidate.is_dir() {
                        return candidate;
                    }
                    if let Ok(exe) = std::env::current_exe() {
                        if let Some(exe_dir) = exe.parent() {
                            for ancestor in exe_dir.ancestors().skip(1) {
                                let candidate = ancestor.join("plugins");
                                if candidate.is_dir() {
                                    return candidate;
                                }
                            }
                        }
                    }
                    PathBuf::from("plugins")
                });

            let ncl_path =
                plugins_dir.join(&namespace).join(format!("{name}.ncl"));
            if ncl_path.exists() {
                return Ok(ncl_path);
            }
            let json_path =
                plugins_dir.join(&namespace).join(format!("{name}.json"));
            if json_path.exists() {
                return Ok(json_path);
            }

            Err(format!(
                "builtin plugin '{plugin_ref}' not found. Looked in:\n  {}\n  {}",
                ncl_path.display(),
                json_path.display()
            ))
        }

        s if s.starts_with("./") || s.starts_with("../") => {
            let base = manifest_path.parent().unwrap_or(Path::new("."));
            let plugin_dir = base.join(s);

            let ncl_path =
                plugin_dir.join(&namespace).join(format!("{name}.ncl"));
            if ncl_path.exists() {
                return Ok(ncl_path);
            }
            let json_path =
                plugin_dir.join(&namespace).join(format!("{name}.json"));
            if json_path.exists() {
                return Ok(json_path);
            }
            let flat_ncl = plugin_dir.join(format!("{plugin_ref}.ncl"));
            if flat_ncl.exists() {
                return Ok(flat_ncl);
            }
            let flat_json = plugin_dir.join(format!("{plugin_ref}.json"));
            if flat_json.exists() {
                return Ok(flat_json);
            }

            Err(format!(
                "plugin '{plugin_ref}' not found in '{s}'. Looked for .ncl and .json in:\n  {}\n  {}",
                plugin_dir.join(&namespace).display(),
                plugin_dir.display()
            ))
        }

        _ => Err(format!(
            "unsupported plugin source '{source}' for '{plugin_ref}'. \
             Supported: \"builtin\", \"./local/path\""
        )),
    }
}
