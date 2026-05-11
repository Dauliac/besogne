use crate::manifest::{Node, Manifest, PatchOp};
use crate::output::style::{self, DiagBuilder};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Resolve all component nodes in a manifest, returning expanded native nodes.
/// Component nodes are replaced by the component's `nodes` map (same format as manifest).
pub fn expand_components(
    manifest: &Manifest,
    manifest_path: &Path,
) -> Result<HashMap<String, Node>, String> {
    let mut expanded = HashMap::new();

    for (key, input) in &manifest.nodes {
        match input {
            Node::Component(component_input) => {
                // The map key IS the component reference (e.g., "coreutils/shell")
                let (namespace, _) = parse_component_ref(key)?;

                // Default to "builtin" if namespace not declared in components map
                let source = manifest.components
                    .get(&namespace)
                    .map(|s| s.as_str())
                    .unwrap_or("builtin");

                let mut visited = Vec::new();
                let produced = expand_component_json(
                    key,
                    &component_input.overrides,
                    &component_input.patch,
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
                        let header = style::error_diag(&format!(
                            "invalid node '{}' in component {}",
                            sub_key, style::bold(key)
                        ));
                        let body = DiagBuilder::new()
                            .location(&format!("component {key} [nodes.{sub_key}]"))
                            .blank()
                            .code(&serde_json::to_string_pretty(&value).unwrap_or_default())
                            .blank()
                            .note(&e.to_string())
                            .build();
                        format!("{header}\n{body}")
                    })?;
                    if expanded.contains_key(&full_key) {
                        let header = style::error_diag(&format!(
                            "duplicate node '{}' from component {}",
                            style::bold(&full_key), key
                        ));
                        let body = DiagBuilder::new()
                            .location(&format!("manifest [nodes.\"{key}\"]"))
                            .blank()
                            .note(&format!("node '{full_key}' already exists"))
                            .hint("two components produce the same node — rename one or use overrides")
                            .build();
                        return Err(format!("{header}\n{body}"));
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

fn parse_component_ref(component_ref: &str) -> Result<(String, String), String> {
    let parts: Vec<&str> = component_ref.splitn(2, '/').collect();
    if parts.len() != 2 {
        return Err(format!(
            "invalid component reference '{component_ref}': expected 'namespace/name' (e.g., 'docker/daemon')"
        ));
    }
    Ok((parts[0].to_string(), parts[1].to_string()))
}

/// Recursively expand a component into a flat list of (sub_key, JSON) pairs.
/// A component is a manifest: its `nodes` map is extracted and expanded.
/// Nested `type: "component"` nodes are recursively expanded.
fn expand_component_json(
    component_ref: &str,
    overrides: &Option<HashMap<String, serde_json::Value>>,
    patch: &Option<HashMap<String, HashMap<String, PatchOp>>>,
    source: &str,
    manifest_path: &Path,
    visited: &mut Vec<String>,
) -> Result<Vec<(String, serde_json::Value)>, String> {
    // Cycle detection
    if visited.contains(&component_ref.to_string()) {
        let chain = format!("{} \u{2192} {}", visited.join(" \u{2192} "), component_ref);
        let header = style::error_diag(&format!(
            "circular component reference: {}",
            style::bold(component_ref)
        ));
        let body = DiagBuilder::new()
            .location(&format!("component {component_ref}"))
            .blank()
            .note(&format!("composition chain: {chain}"))
            .hint("break the cycle by removing one of the component references")
            .build();
        return Err(format!("{header}\n{body}"));
    }
    visited.push(component_ref.to_string());

    // Load component nodes
    let component_path = resolve_component_path(source, component_ref, manifest_path)
        .map_err(|e| {
            if visited.len() > 1 {
                let chain = visited.join(" \u{2192} ");
                format!("{e}\n   = note: composition chain: {chain}")
            } else {
                e
            }
        })?;
    let mut nodes = load_component_nodes(&component_path)
        .map_err(|e| {
            if visited.len() > 1 {
                let chain = visited.join(" \u{2192} ");
                format!("{e}\n   = note: composition chain: {chain}")
            } else {
                e
            }
        })?;

    // Apply per-node overrides (shallow merge — replaces fields)
    if let Some(overrides) = overrides {
        for (node_key, partial) in overrides {
            if let Some(base) = nodes.get_mut(node_key) {
                json_shallow_merge(base, partial);
            } else {
                nodes.insert(node_key.clone(), partial.clone());
            }
        }
    }

    // Apply per-node patches (array operations — append/prepend/remove)
    if let Some(patches) = patch {
        for (node_key, field_patches) in patches {
            if let Some(node) = nodes.get_mut(node_key) {
                apply_patches(node, field_patches);
            }
        }
    }

    // Sort keys for deterministic expansion
    let mut entries: Vec<(String, serde_json::Value)> = nodes.into_iter().collect();
    entries.sort_by(|a, b| a.0.cmp(&b.0));

    // Expand: nested components → recurse, native nodes → keep
    let mut produced: Vec<(String, serde_json::Value)> = Vec::new();
    for (key, value) in entries {
        let is_component = value.get("type").and_then(|t| t.as_str()) == Some("component");
        if is_component {
            let sub_ref = key.as_str();

            let sub_overrides = value
                .get("overrides")
                .and_then(|o| serde_json::from_value(o.clone()).ok());
            let sub_patch = value
                .get("patch")
                .and_then(|p| serde_json::from_value(p.clone()).ok());

            let sub_nodes = expand_component_json(
                sub_ref,
                &sub_overrides,
                &sub_patch,
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

/// Load a component's nodes map from disk.
/// Components use manifest format (`nodes: { ... }`).
fn load_component_nodes(
    component_path: &Path,
) -> Result<HashMap<String, serde_json::Value>, String> {
    let content = std::fs::read_to_string(component_path)
        .map_err(|e| format!("cannot read component {}: {e}", component_path.display()))?;
    let raw: serde_json::Value = serde_json::from_str(&content)
        .map_err(|e| format!("invalid component JSON {}: {e}", component_path.display()))?;

    let nodes_obj = raw
        .get("nodes")
        .and_then(|n| n.as_object())
        .ok_or_else(|| {
            format!("component '{}' missing 'nodes' map", component_path.display())
        })?;

    Ok(nodes_obj
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect())
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

/// Apply array patches to a node's fields (append/prepend/remove by value).
fn apply_patches(
    node: &mut serde_json::Value,
    field_patches: &HashMap<String, PatchOp>,
) {
    if let Some(obj) = node.as_object_mut() {
        for (field, patch) in field_patches {
            let arr = obj
                .entry(field.clone())
                .or_insert_with(|| serde_json::Value::Array(vec![]));

            if let serde_json::Value::Array(arr) = arr {
                // Remove by value
                if let Some(remove_vals) = &patch.remove {
                    arr.retain(|v| !remove_vals.contains(v));
                }
                // Prepend
                if let Some(prepend_vals) = &patch.prepend {
                    let mut new_arr = prepend_vals.clone();
                    new_arr.append(arr);
                    *arr = new_arr;
                }
                // Append
                if let Some(append_vals) = &patch.append {
                    arr.extend(append_vals.iter().cloned());
                }
            }
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
/// Used when a nested component's nodes are being embedded under a key.
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

/// Resolve a component reference to a file path on disk.
fn resolve_component_path(
    source: &str,
    component_ref: &str,
    manifest_path: &Path,
) -> Result<PathBuf, String> {
    let (namespace, name) = parse_component_ref(component_ref)?;

    match source {
        "builtin" => {
            let components_dir = std::env::var("BESOGNE_COMPONENTS_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|_| {
                    let manifest_dir =
                        manifest_path.parent().unwrap_or(Path::new("."));
                    let candidate = manifest_dir.join("components");
                    if candidate.is_dir() {
                        return candidate;
                    }
                    if let Ok(exe) = std::env::current_exe() {
                        if let Some(exe_dir) = exe.parent() {
                            for ancestor in exe_dir.ancestors().skip(1) {
                                let candidate = ancestor.join("components");
                                if candidate.is_dir() {
                                    return candidate;
                                }
                            }
                        }
                    }
                    PathBuf::from("components")
                });

            let json_path =
                components_dir.join(&namespace).join(format!("{name}.json"));
            if json_path.exists() {
                return Ok(json_path);
            }

            Err(format!(
                "builtin component '{component_ref}' not found. Looked in:\n  {}",
                json_path.display()
            ))
        }

        s if s.starts_with("./") || s.starts_with("../") => {
            let base = manifest_path.parent().unwrap_or(Path::new("."));
            let component_dir = base.join(s);

            let json_path =
                component_dir.join(&namespace).join(format!("{name}.json"));
            if json_path.exists() {
                return Ok(json_path);
            }
            let flat_json = component_dir.join(format!("{component_ref}.json"));
            if flat_json.exists() {
                return Ok(flat_json);
            }

            Err(format!(
                "component '{component_ref}' not found in '{s}'. Looked in:\n  {}\n  {}",
                component_dir.join(&namespace).display(),
                component_dir.display()
            ))
        }

        _ => Err(format!(
            "unsupported component source '{source}' for '{component_ref}'. \
             Supported: \"builtin\", \"./local/path\""
        )),
    }
}
