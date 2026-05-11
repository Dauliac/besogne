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

As a **postcondition** (child of a command):

```toml
[nodes.node-modules]
type = "file"
path = "node_modules"
expect = "directory"
parents = ["install"]       # postcondition of install command
```

## binary

Resolve a binary via PATH, probe version, hash content.

```toml
[nodes.go]
type = "binary"

[nodes.compile]
type = "binary"
parents = ["go"]            # toolchain internal, hash derived from parent
```

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

## command

Execute a command. The only **action** node type.

```toml
[nodes.test]
type = "command"
run = ["go", "test", "./..."]
parents = ["mod-download-exit", "test-go"]
force_args = ["-count=1"]
debug_args = ["-v"]
```

| Field | Description |
|---|---|
| `run` | Command to execute (array, string, pipe, or script) |
| `parents` | DAG parents (must complete before this runs) |
| `stdin` | Single `std` node name to pipe as stdin |
| `env` | Extra env vars for this command |
| `workdir` | Working directory (relative to manifest dir) |
| `side_effects` | If true, always run, never cache |
| `force_args` | Extra args appended when `--force` is passed |
| `debug_args` | Extra args appended when `--debug` is passed |

## std

Probe on command I/O — stdout, stderr, exit code, or stdin. Replaces `output:`, `postconditions:`, and `pipe:`.

```toml
# Exit code check
[nodes.test-exit]
type = "std"
stream = "exit_code"
parents = ["test"]

[nodes.test-exit.content.int]
expect = 0

# Stdout validation
[nodes.test-stdout]
type = "std"
stream = "stdout"
parents = ["test"]

[nodes.test-stdout.content.text]
contains = ["PASS"]

# Structured output with extraction
[nodes.test-json]
type = "std"
stream = "stdout"
parents = ["test"]

[nodes.test-json.content.jsonline]
schema = "./schemas/go-test.schema.json"

[nodes.test-json.content.jsonline.extract]
status = ".Action"
elapsed = ".Elapsed"

# Piping: connect stdout to next command's stdin
[nodes.generate-out]
type = "std"
stream = "stdout"
parents = ["generate"]

[nodes.format]
type = "command"
run = ["jq", ".data"]
stdin = "generate-out"          # exactly one stdin source
parents = ["generate-out"]
```

| Field | Description |
|---|---|
| `stream` | `stdout`, `stderr`, `exit_code`, or `stdin` |
| `parents` | The command this probes (for stdout/stderr/exit_code) |

A command can have at most ONE `stdin` source — multiple is a compile error.

## user

Check user identity and group membership.

```toml
[nodes.docker-user]
type = "user"
in_group = "docker"
```

## platform

Check OS and architecture.

```toml
[nodes.linux-only]
type = "platform"
os = "linux"
arch = "x86_64"
```

## dns

Resolve a hostname.

```toml
[nodes.registry]
type = "dns"
host = "registry.internal.io"
```

## metric

Read system metrics from `/proc` or `statvfs()`.

```toml
[nodes.memory]
type = "metric"
metric = "memory.available_mb"

[nodes.memory.content.float]
min = 512
```

## Content validation (all types)

Any node can have typed content validation via `content.<format>`:

```toml
[nodes.config.content.json]
schema = "./config.schema.json"
has_fields = ["database"]

[nodes.config.content.json.extract]
db_host = ".database.host"
```

See [content types](./content-types.md) for the full format reference.

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

Source nodes are **probes** — they read environment state without executing anything. The parsed env vars flow into `all_variables` and are available to all commands.

### Using plugins for common tools

Instead of writing source nodes manually, use builtin `env/*` plugins:

```toml
[nodes.dev-env]
type = "plugin"
plugin = "env/direnv"

[nodes.secrets]
type = "plugin"
plugin = "env/dotenv"
```

Available: `env/direnv`, `env/mise`, `env/nix`, `env/venv`, `env/dotenv`, `env/conda`.

## plugin

Expands into native inputs at build time. Not a runtime node.

```toml
[nodes.coreutils]
type = "plugin"
plugin = "coreutils/shell"
```
