use crate::compile::nickel;
use crate::manifest::{Input, Manifest};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// A loaded plugin definition (from JSON — Nickel plugins get evaluated to this)
#[derive(Debug, Clone, serde::Deserialize)]
pub struct PluginDef {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub params: HashMap<String, ParamSchema>,
    pub produces: Vec<serde_json::Value>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct ParamSchema {
    #[serde(rename = "type", default = "default_string")]
    pub param_type: String,
    #[serde(default)]
    pub optional: bool,
    #[serde(default)]
    pub default: Option<serde_json::Value>,
}

fn default_string() -> String {
    "string".into()
}

/// Resolve all plugin inputs in a manifest, returning expanded native inputs.
pub fn expand_plugins(
    manifest: &Manifest,
    manifest_path: &Path,
) -> Result<HashMap<String, Input>, String> {
    let mut expanded = HashMap::new();

    for (key, input) in &manifest.inputs {
        match input {
            Input::Plugin(plugin_input) => {
                let (namespace, _plugin_name) = parse_plugin_ref(&plugin_input.plugin)?;

                let source = manifest.plugins.get(&namespace).ok_or_else(|| {
                    format!(
                        "input '{key}': plugin namespace '{namespace}' not declared in `plugins` map. \
                         Add: plugins.{namespace} = \"builtin\" (or a path)"
                    )
                })?;

                let produced = expand_plugin_input(
                    source,
                    &plugin_input.plugin,
                    &plugin_input.params,
                    &plugin_input.overrides,
                    manifest_path,
                    key,
                )?;

                for (sub_key, native_input) in produced {
                    let full_key = format!("{key}.{sub_key}");
                    if expanded.contains_key(&full_key) {
                        return Err(format!("plugin '{key}' produces duplicate input '{full_key}'"));
                    }
                    expanded.insert(full_key, native_input);
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

/// Expand a single plugin input into native inputs.
/// Tries Nickel first (.ncl), then falls back to JSON (.json).
fn expand_plugin_input(
    source: &str,
    plugin_ref: &str,
    params: &HashMap<String, serde_json::Value>,
    overrides: &Option<HashMap<String, serde_json::Value>>,
    manifest_path: &Path,
    input_key: &str,
) -> Result<Vec<(String, Input)>, String> {
    let plugin_path = resolve_plugin_path(source, plugin_ref, manifest_path)?;

    let ext = plugin_path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");

    let produced_json: Vec<serde_json::Value> = if ext == "ncl" {
        // Nickel plugin: evaluate produces(params) via nickel CLI
        nickel::eval_plugin(&plugin_path, params)?
    } else {
        // JSON plugin: load definition, validate params, expand templates
        let content = std::fs::read_to_string(&plugin_path)
            .map_err(|e| format!("cannot read plugin {}: {e}", plugin_path.display()))?;
        let plugin_def: PluginDef = serde_json::from_str(&content)
            .map_err(|e| format!("invalid plugin JSON {}: {e}", plugin_path.display()))?;

        validate_params(&plugin_def, params, input_key)?;
        let resolved = resolve_params(&plugin_def, params);
        expand_produces_json(&plugin_def.produces, &resolved)?
    };

    // First pass: collect sub-keys so we can rewrite `after` references
    let sub_keys: Vec<String> = produced_json
        .iter()
        .enumerate()
        .map(|(idx, v)| derive_sub_key(v, idx))
        .collect();

    // Second pass: rewrite `after` arrays to use full keys, parse into native Inputs
    let mut result = Vec::new();
    for (idx, mut value) in produced_json.into_iter().enumerate() {
        // Apply overrides
        if let (Some(overrides), Some(obj)) = (overrides, value.as_object_mut()) {
            for (k, v) in overrides {
                obj.insert(k.clone(), v.clone());
            }
        }

        // Rewrite `after` references: if a ref matches a sibling sub-key, prefix with input_key
        if let Some(obj) = value.as_object_mut() {
            if let Some(serde_json::Value::Array(after)) = obj.get_mut("after") {
                for dep in after.iter_mut() {
                    if let serde_json::Value::String(dep_name) = dep {
                        if sub_keys.contains(dep_name) {
                            *dep_name = format!("{input_key}.{dep_name}");
                        }
                    }
                }
            }
        }

        let sub_key = sub_keys[idx].clone();

        let input: Input = serde_json::from_value(value.clone()).map_err(|e| {
            format!(
                "plugin '{input_key}' produces[{idx}]: cannot parse as native input: {e}\n  value: {}",
                serde_json::to_string_pretty(&value).unwrap_or_default()
            )
        })?;

        result.push((sub_key, input));
    }

    Ok(result)
}

/// Resolve a plugin reference to a file path on disk.
fn resolve_plugin_path(source: &str, plugin_ref: &str, manifest_path: &Path) -> Result<PathBuf, String> {
    let (namespace, name) = parse_plugin_ref(plugin_ref)?;

    match source {
        "builtin" => {
            // Builtin plugins: look in the repo's plugins/ dir relative to the binary
            // At build time, BESOGNE_PLUGINS_DIR can override
            let plugins_dir = std::env::var("BESOGNE_PLUGINS_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|_| {
                    // Try relative to manifest, then relative to cwd
                    let manifest_dir = manifest_path.parent().unwrap_or(Path::new("."));
                    let candidate = manifest_dir.join("plugins");
                    if candidate.is_dir() {
                        candidate
                    } else {
                        PathBuf::from("plugins")
                    }
                });

            // Try .ncl first, then .json
            let ncl_path = plugins_dir.join(&namespace).join(format!("{name}.ncl"));
            if ncl_path.exists() {
                return Ok(ncl_path);
            }
            let json_path = plugins_dir.join(&namespace).join(format!("{name}.json"));
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

            // Try namespace/name.ncl, then namespace/name.json
            let ncl_path = plugin_dir.join(&namespace).join(format!("{name}.ncl"));
            if ncl_path.exists() {
                return Ok(ncl_path);
            }
            let json_path = plugin_dir.join(&namespace).join(format!("{name}.json"));
            if json_path.exists() {
                return Ok(json_path);
            }
            // Flat: plugin_ref.ncl, plugin_ref.json
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

// ── JSON plugin helpers (template-based, for plugins without Nickel) ──

fn validate_params(
    plugin: &PluginDef,
    user_params: &HashMap<String, serde_json::Value>,
    input_key: &str,
) -> Result<(), String> {
    for (name, schema) in &plugin.params {
        if !schema.optional && schema.default.is_none() && !user_params.contains_key(name) {
            return Err(format!(
                "input '{input_key}': plugin '{}' requires param '{name}'",
                plugin.name
            ));
        }
    }
    Ok(())
}

fn resolve_params(
    plugin: &PluginDef,
    user_params: &HashMap<String, serde_json::Value>,
) -> HashMap<String, String> {
    let mut resolved = HashMap::new();
    for (name, schema) in &plugin.params {
        if let Some(val) = user_params.get(name) {
            resolved.insert(name.clone(), json_value_to_string(val));
        } else if let Some(default) = &schema.default {
            resolved.insert(name.clone(), json_value_to_string(default));
        }
    }
    for (name, val) in user_params {
        if !resolved.contains_key(name) {
            resolved.insert(name.clone(), json_value_to_string(val));
        }
    }
    resolved
}

fn json_value_to_string(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Null => String::new(),
        other => other.to_string(),
    }
}

fn expand_produces_json(
    produces: &[serde_json::Value],
    params: &HashMap<String, String>,
) -> Result<Vec<serde_json::Value>, String> {
    let mut result = Vec::new();
    for template in produces {
        if let Some(when) = template.get("when") {
            if !eval_when(when, params) {
                continue;
            }
        }
        let mut obj = template.clone();
        if let Some(map) = obj.as_object_mut() {
            map.remove("when");
        }
        result.push(substitute_json(&obj, params));
    }
    Ok(result)
}

fn eval_when(when: &serde_json::Value, params: &HashMap<String, String>) -> bool {
    match when {
        serde_json::Value::String(param) => {
            params.get(param).map(|v| !v.is_empty()).unwrap_or(false)
        }
        serde_json::Value::Object(map) => {
            if let Some(serde_json::Value::String(param)) = map.get("not") {
                !params.get(param).map(|v| !v.is_empty()).unwrap_or(false)
            } else {
                true
            }
        }
        serde_json::Value::Bool(b) => *b,
        _ => true,
    }
}

fn substitute_json(
    value: &serde_json::Value,
    params: &HashMap<String, String>,
) -> serde_json::Value {
    match value {
        serde_json::Value::String(s) => serde_json::Value::String(substitute_string(s, params)),
        serde_json::Value::Array(arr) => {
            serde_json::Value::Array(arr.iter().map(|v| substitute_json(v, params)).collect())
        }
        serde_json::Value::Object(map) => {
            serde_json::Value::Object(
                map.iter()
                    .map(|(k, v)| (k.clone(), substitute_json(v, params)))
                    .collect(),
            )
        }
        other => other.clone(),
    }
}

fn substitute_string(s: &str, params: &HashMap<String, String>) -> String {
    let mut result = s.to_string();
    for (key, value) in params {
        result = result.replace(&format!("{{{{ {key} }}}}"), value);
        result = result.replace(&format!("{{{{{key}}}}}"), value);
    }
    result
}
