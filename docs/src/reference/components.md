# Components

Components are reusable manifests that expand into native nodes. They live in `components/<category>/<name>.json`.

## Component structure

A component IS a manifest — same `nodes: {}` format:

```json
{
  "name": "go/deps",
  "nodes": {
    "go/toolchain": { "type": "component" },
    "go-mod": { "type": "file", "path": "go.mod" },
    "download": { "type": "command", "run": ["go", "mod", "download"],
                   "parents": ["go/toolchain.go", "go-mod"] }
  }
}
```

- No separate schema, no params, no templates.
- `nodes` map uses the same format as terminal manifests.
- Components can reference other components via `type: "component"` nodes (recursive expansion).

## Overrides

Users override component internals with shallow merge:

```toml
[nodes."go/deps".overrides.download]
description = "custom download step"
```

## Patches

Array operations (append/prepend/remove) on component node fields:

```toml
[nodes."go/deps".patch.download.run]
append = ["-x", "-v"]
```

## Multi-phase components

Produced nodes can have different phases:

```json
{
  "nodes": {
    "kubeconfig": { "type": "env", "phase": "seal" },
    "config-file": { "type": "file", "path": "kubeconfig.yaml", "phase": "exec" },
    "cluster-info": { "type": "command", "phase": "exec",
                      "run": ["kubectl", "cluster-info"],
                      "parents": ["config-file"] }
  }
}
```
