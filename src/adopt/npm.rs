//! package.json parser and rewriter for `besogne adopt`.

use super::ParsedScript;
use std::path::Path;

/// Parse package.json and extract scripts
pub fn parse_package_json(
    path: &Path,
) -> Result<(serde_json::Value, Vec<ParsedScript>), String> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;

    let pkg: serde_json::Value = serde_json::from_str(&content)
        .map_err(|e| format!("invalid JSON in {}: {e}", path.display()))?;

    let scripts_obj = match pkg.get("scripts") {
        Some(serde_json::Value::Object(m)) => m,
        Some(_) => return Err("\"scripts\" field is not an object".into()),
        None => return Err("no \"scripts\" field in package.json".into()),
    };

    let scripts: Vec<ParsedScript> = scripts_obj
        .iter()
        .map(|(name, value)| {
            let body = value.as_str().unwrap_or("").to_string();
            ParsedScript {
                name: name.clone(),
                body,
                binaries: Vec::new(),
                env_vars: Vec::new(),
                parents: Vec::new(),
                side_effects: false,
            }
        })
        .collect();

    Ok((pkg, scripts))
}

/// Resolve npm lifecycle ordering: pre<X> → X → post<X>
pub fn resolve_lifecycle_ordering(scripts: &mut [ParsedScript]) {
    let names: Vec<String> = scripts.iter().map(|s| s.name.clone()).collect();

    // Collect pre/post dependencies first, then apply
    let mut deps_to_add: Vec<(String, String)> = Vec::new();
    for script in scripts.iter() {
        let name = &script.name;

        if let Some(base) = name.strip_prefix("seal") {
            if names.contains(&base.to_string()) {
                deps_to_add.push((base.to_string(), name.to_string()));
            }
        }

        if let Some(base) = name.strip_prefix("post") {
            if names.contains(&base.to_string()) {
                deps_to_add.push((name.to_string(), base.to_string()));
            }
        }
    }

    // Apply collected dependencies
    for (target_name, dep_name) in deps_to_add {
        if let Some(target) = scripts.iter_mut().find(|s| s.name == target_name) {
            if !target.parents.contains(&dep_name) {
                target.parents.push(dep_name);
            }
        }
    }
}

/// Rewrite package.json: replace each script body with `besogne run <name>`
pub fn rewrite_package_json(
    original: &serde_json::Value,
    scripts: &[ParsedScript],
) -> Result<String, String> {
    let mut pkg = original.clone();

    if let Some(scripts_obj) = pkg.get_mut("scripts").and_then(|s| s.as_object_mut()) {
        for script in scripts {
            scripts_obj.insert(
                script.name.clone(),
                serde_json::Value::String(format!("besogne run {}", script.name)),
            );
        }
    }

    serde_json::to_string_pretty(&pkg)
        .map_err(|e| format!("cannot serialize package.json: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn test_parse_package_json() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        write!(
            tmp,
            r#"{{"name": "test", "scripts": {{"build": "tsc", "test": "jest"}}}}"#
        ).unwrap();

        let (_, scripts) = parse_package_json(tmp.path()).unwrap();
        assert_eq!(scripts.len(), 2);
        let names: Vec<_> = scripts.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"build"));
        assert!(names.contains(&"test"));
    }

    #[test]
    fn test_lifecycle_ordering() {
        let mut scripts = vec![
            ParsedScript {
                name: "prebuild".into(),
                body: "lint".into(),
                binaries: vec![],
                env_vars: vec![],
                parents: vec![],
                side_effects: false,
            },
            ParsedScript {
                name: "build".into(),
                body: "tsc".into(),
                binaries: vec![],
                env_vars: vec![],
                parents: vec![],
                side_effects: false,
            },
            ParsedScript {
                name: "postbuild".into(),
                body: "cp dist/".into(),
                binaries: vec![],
                env_vars: vec![],
                parents: vec![],
                side_effects: false,
            },
        ];

        resolve_lifecycle_ordering(&mut scripts);

        let build = scripts.iter().find(|s| s.name == "build").unwrap();
        assert!(build.parents.contains(&"prebuild".to_string()));

        let postbuild = scripts.iter().find(|s| s.name == "postbuild").unwrap();
        assert!(postbuild.parents.contains(&"build".to_string()));
    }

    #[test]
    fn test_rewrite_package_json() {
        let original: serde_json::Value = serde_json::from_str(
            r#"{"name": "test", "version": "1.0.0", "scripts": {"build": "tsc", "test": "jest"}}"#,
        ).unwrap();

        let scripts = vec![
            ParsedScript {
                name: "build".into(),
                body: "tsc".into(),
                binaries: vec![],
                env_vars: vec![],
                parents: vec![],
                side_effects: false,
            },
            ParsedScript {
                name: "test".into(),
                body: "jest".into(),
                binaries: vec![],
                env_vars: vec![],
                parents: vec![],
                side_effects: false,
            },
        ];

        let result = rewrite_package_json(&original, &scripts).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["scripts"]["build"], "besogne run build");
        assert_eq!(parsed["scripts"]["test"], "besogne run test");
        // Preserve other fields
        assert_eq!(parsed["version"], "1.0.0");
    }
}
