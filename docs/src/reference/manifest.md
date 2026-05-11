# Manifest schema

A besogne manifest is a TOML, YAML, or JSON file. Supported filenames: `besogne.toml`, `besogne.yaml`, `besogne.json`, or `<name>.besogne.toml`.

## Top-level fields

| Field | Type | Required | Description |
|---|---|---|---|
| `name` | string | yes | Besogne name |
| `description` | string | yes | Human description |
| `side_effects` | bool | no | Opt out of caching (default: false) |
| `sandbox` | string/object | no | Sandbox preset or custom config |
| `flags` | array | no | CLI flags for the produced binary |
| `inputs` | map | no | Named map of all seals and execution steps |

## Inputs (named map)

Inputs are a **named map** — the key IS the input's identity.

TOML:
```toml
[inputs.go]
type = "binary"

[inputs.go-mod]
type = "file"
path = "go.mod"

[inputs.test]
type = "command"
phase = "exec"
run = ["go", "test", "./..."]
after = ["build"]
```

YAML:
```yaml
inputs:
  go:
    type: binary
  go-mod:
    type: file
    path: go.mod
  test:
    type: command
    phase: exec
    run: ["go", "test", "./..."]
    after: ["build"]
```

The key serves as:
- **Command name** — used in `after:` references
- **Env var name** — defaults to the key for `env` type (override with `name:`)
- **Binary name** — defaults to the key for `binary` type (override with `name:`)
- **Plugin reference** — used for `after:` from exec-phase commands

## Native input types

`env`, `file`, `binary`, `service`, `command`, `user`, `platform`, `dns`, `metric`, `plugin`.

### Phase

Each input has a `phase` (when it's evaluated):

| Phase | When | Default for |
|---|---|---|
| `build` | `besogne build` | binary, platform |
| `seal` | Startup (parallel) | env, file, service, user, dns, metric |
| `exec` | DAG execution | command |

### Command inputs

```toml
[inputs.test]
type = "command"
phase = "exec"
run = ["go", "test", "./..."]
env = { CGO_ENABLED = "0" }
after = ["install"]
side_effects = false

[[inputs.test.ensure]]
type = "file"
path = "cover.out"
required = true
```

`run` forms:
- Array: `["go", "test"]`
- String (bash): `"go test | grep PASS"`
- Pipe: `{ pipe = [["echo", "hello"], ["tr", "a-z", "A-Z"]] }`
- Script: `{ file = "./run.sh", args = ["--flag"] }`

### Plugin inputs

```toml
[inputs.k8s]
type = "plugin"
plugin = "k8s/cluster"
context = "staging"

[inputs.k8s.overrides]
KUBECONFIG = { phase = "build" }
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
| `side_effects` | `false` (everything cached by default) |
| `sandbox` | none (inherit everything) |
| `phase` for binary | `build` |
| `phase` for env/file/service | `seal` |
| `phase` for command | `exec` |
| `on_missing` | `fail` |
| `side_effects` | `false` (cached by default) |
| `required` (ensure) | `true` |
| `name` (env/binary) | map key |
