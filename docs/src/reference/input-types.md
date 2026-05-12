# Native node types

All 11 native types are implemented in pure Rust with syscalls — no external dependencies.

Nodes are either **probes** (hashable, no execution) or **actions** (execute commands). The `parents:` field connects them into a DAG. Edge meaning is derived from node types.

## env

Read or set an environment variable.

```toml
[nodes.home]
type = "env"
name = "HOME"

[nodes.token]
type = "env"
name = "TOKEN"
secret = true

[nodes.cache-dir]
type = "env"
name = "CACHE"
value = "/tmp/cache"
```

| Field | Description |
|---|---|
| `name` | Env var name (defaults to map key) |
| `value` | Set this value (don't read from shell) |
| `from_env` | If true + value set: use value as default, shell overrides |
| `secret` | Mask value in output |
| `on_missing` | What to do if var is missing: `fail` (default) or `skip` |

## file

Check file/directory/socket existence. Content-hashed for caching.

```toml
[nodes.go-mod]
type = "file"
path = "go.mod"

[nodes.docker-sock]
type = "file"
path = "/var/run/docker.sock"
expect = "socket"
```

As a **postcondition** (child of a command via `parents:`):

```toml
[nodes.node-modules]
type = "file"
path = "node_modules"
expect = "directory"
parents = ["install"]       # postcondition of install command
```

| Field | Description |
|---|---|
| `path` | File path (relative to manifest dir) |
| `expect` | Expected type: `file`, `directory`, `socket` |
| `permissions` | Expected permissions string |
| `parents` | DAG parents |

## binary

Resolve a binary via PATH, probe version, hash content.

```toml
[nodes.go]
type = "binary"

[nodes.compile]
type = "binary"
parents = ["go"]            # toolchain internal, hash derived from parent
```

| Field | Description |
|---|---|
| `name` | Binary name for PATH resolution (defaults to map key) |
| `path` | Explicit path (skip PATH resolution) |
| `version` | Expected version or semver constraint (e.g. `"22"`, `">=1.22"`) |
| `parents` | Parent binary nodes (embedded binaries, e.g. Go's `compile` inside `go`) |
| `sealed` | If true, resolve and embed at build time (Nix store paths) |

## service

Check TCP/HTTP reachability.

```toml
[nodes.postgres]
type = "service"
tcp = "localhost:5432"

[nodes.api]
type = "service"
http = "http://localhost:8080/health"
```

| Field | Description |
|---|---|
| `tcp` | TCP address to probe (e.g. `"localhost:5432"`) |
| `http` | HTTP URL to probe (e.g. `"http://localhost:8080/health"`) |
| `on_fail` | What to do on failure: `fail` (default), `skip`, or `warn` |
| `retry` | Retry configuration (see below) |
| `parents` | DAG parents |

## command

Execute a command. The only **action** node type.

```toml
[nodes.test]
type = "command"
run = ["go", "test", "./..."]
parents = ["mod-download", "test-go"]
force_args = ["-count=1"]
debug_args = ["-v"]
```

| Field | Description |
|---|---|
| `run` | Command to execute (array, string, or script) |
| `parents` | DAG parents (must complete before this runs) |
| `env` | Extra env vars for this command |
| `workdir` | Working directory (relative to manifest dir) |
| `side_effects` | If true, always run, never cache |
| `force_args` | Extra args appended when `--force` is passed |
| `debug_args` | Extra args appended when `--debug` is passed |
| `on_fail` | What to do on failure: `fail` (default), `skip`, or `warn` |
| `retry` | Retry configuration (see below) |
| `description` | Human-readable description |

## std

Probe on command I/O — stdout, stderr, or exit code.

```toml
# Exit code check
[nodes.test-exit]
type = "std"
stream = "exit_code"
parents = ["test"]
expect = "0"

# Stdout validation
[nodes.test-stdout]
type = "std"
stream = "stdout"
parents = ["test"]
contains = ["PASS"]
```

| Field | Description |
|---|---|
| `stream` | Which stream: `stdout`, `stderr`, or `exit_code` |
| `parents` | The command whose output to probe |
| `contains` | Assert content contains these strings (for stdout/stderr) |
| `expect` | Assert exact match (for exit_code: `"0"`) |

## platform

Check OS and architecture.

```toml
[nodes.linux-only]
type = "platform"
os = "linux"
arch = "x86_64"
```

| Field | Description |
|---|---|
| `os` | Expected OS (e.g. `"linux"`, `"macos"`) |
| `arch` | Expected architecture (e.g. `"x86_64"`, `"aarch64"`) |
| `kernel_min` | Minimum kernel version |

## dns

Resolve a hostname.

```toml
[nodes.registry]
type = "dns"
host = "registry.internal.io"
```

| Field | Description |
|---|---|
| `host` | Hostname to resolve |
| `expect` | Expected IP address |
| `retry` | Retry configuration |
| `parents` | DAG parents |

## metric

Read system metrics from `/proc` or `statvfs()`.

```toml
[nodes.memory]
type = "metric"
metric = "memory.available_mb"

[nodes.disk]
type = "metric"
metric = "disk.available_gb"
path = "/"
```

| Field | Description |
|---|---|
| `metric` | Metric name (e.g. `"memory.available_mb"`, `"disk.available_gb"`, `"cpu.count"`) |
| `path` | Filesystem path for disk metrics |

## source

Load a map of environment variables from a file or command output. Env vars are injected into commands that depend on this node.

```toml
# From a .env file
[nodes.secrets]
type = "source"
format = "dotenv"
path = ".env"

# From a JSON file (e.g. direnv export, mise env)
[nodes.dev-env]
type = "source"
format = "json"
path = "env.json"

# With select filter (only keep specific vars)
[nodes.nix-env]
type = "source"
format = "json"
path = "nix-env.json"
phase = "build"
select = ["GOPATH", "PATH", "CC"]
```

| Field | Description |
|---|---|
| `format` | Parse format: `json` (flat object), `dotenv` (KEY=VALUE), `shell` (export KEY=VALUE) |
| `path` | File to parse (omit if reading from a `std` parent) |
| `select` | Only keep these env var names (filter) |
| `parents` | DAG parents (file nodes, std nodes, etc.) |

## component

Reference a reusable manifest that expands into native nodes at build time. Not a runtime node.

```toml
[nodes."coreutils/shell"]
type = "component"

[nodes."go/deps"]
type = "component"

[nodes."go/deps".overrides.download]
description = "custom download step"

[nodes."go/deps".patch.download.run]
append = ["-x", "-v"]
```

| Field | Description |
|---|---|
| `overrides` | Per-node field overrides (shallow merge) |
| `patch` | Per-node array patches: `append`, `prepend`, `remove` |

See [Components](./components.md) for the full component reference.

## Retry configuration

Services, DNS, and commands support retry:

```toml
[nodes.db]
type = "service"
tcp = "localhost:5432"

[nodes.db.retry]
attempts = 5
interval = "2s"
backoff = "exponential"
max_interval = "30s"
timeout = "5m"
```

| Field | Description |
|---|---|
| `attempts` | Maximum number of attempts (including first try) |
| `interval` | Base interval between retries (e.g. `"1s"`, `"500ms"`) |
| `backoff` | Strategy: `"fixed"` (default), `"linear"`, `"exponential"` |
| `max_interval` | Maximum interval cap (e.g. `"30s"`) |
| `timeout` | Total timeout for all attempts (e.g. `"5m"`) |
