# Manifest schema

A besogne manifest is a TOML, YAML, or JSON file. Supported filenames: `besogne.toml`, `besogne.yaml`, `besogne.json`, or `<name>.besogne.toml`.

## Top-level fields

| Field | Type | Required | Description |
|---|---|---|---|
| `name` | string | yes | Besogne name |
| `description` | string | yes | Human description |
| `sandbox` | string/object | no | Sandbox preset or custom config |
| `flags` | array | no | CLI flags for the produced binary |
| `components` | map | no | Component sources: namespace → source |
| `nodes` | map | no | Named map of all seals and execution steps |

## Nodes (named map)

Nodes are a **named map** — the key IS the node's identity.

TOML:
```toml
[nodes.go]
type = "binary"

[nodes.go-mod]
type = "file"
path = "go.mod"

[nodes.test]
type = "command"
phase = "exec"
run = ["go", "test", "./..."]
parents = ["build"]
```

YAML:
```yaml
nodes:
  go:
    type: binary
  go-mod:
    type: file
    path: go.mod
  test:
    type: command
    phase: exec
    run: ["go", "test", "./..."]
    parents: ["build"]
```

The key serves as:
- **Command name** — used in `parents:` references
- **Env var name** — defaults to the key for `env` type (override with `name:`)
- **Binary name** — defaults to the key for `binary` type (override with `name:`)
- **Component reference** — resolves to `components/<key>.json`

## Native node types

`env`, `file`, `binary`, `service`, `command`, `source`, `platform`, `dns`, `metric`, `component`, `std`.

### Phase

Each node has a `phase` (when it's evaluated):

| Phase | When | Default for |
|---|---|---|
| `build` | `besogne build` | binary, platform |
| `seal` | Startup (parallel) | env, file, source |
| `exec` | DAG execution | command, service, dns, metric |

### Command nodes

```toml
[nodes.test]
type = "command"
phase = "exec"
run = ["go", "test", "./..."]
env = { CGO_ENABLED = "0" }
parents = ["install"]
side_effects = false
force_args = ["-count=1"]
debug_args = ["-v"]
```

`run` forms:
- Array: `["go", "test"]`
- String (bash): `"go test | grep PASS"`
- Script: `{ file = "./run.sh", args = ["--flag"] }`

### Component nodes

```toml
[nodes."go/deps"]
type = "component"

[nodes."go/deps".overrides.download]
description = "custom download step"

[nodes."go/deps".patch.download.run]
append = ["-x", "-v"]
```

## Sandbox presets

```toml
sandbox = "strict"
```

| Preset | Env | Filesystem | Network |
|---|---|---|---|
| `"strict"` | declared only | tmpdir | none |
| `"container"` | declared only | docker/podman | restricted |

Custom:
```toml
[sandbox]
preset = "strict"
network = "host"
```

## Flags

```toml
[[flags]]
name = "verbose"
kind = "bool"
description = "Enable verbose output"

[[flags]]
name = "env"
kind = "string"
values = ["staging", "prod"]
required = true
```

## Defaults

| Field | Default |
|---|---|
| `sandbox` | none (inherit everything) |
| `phase` for binary | `build` |
| `phase` for platform | `build` |
| `phase` for env/file/source | `seal` |
| `phase` for command/service/dns/metric | `exec` |
| `on_missing` | `fail` |
| `side_effects` | `false` (cached by default) |
| `name` (env/binary) | map key |
