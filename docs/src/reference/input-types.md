# Native input types

All 9 native types are implemented in pure Rust with syscalls — no external dependencies.

## env

Read or set an environment variable.

```json
{ "type": "env", "name": "HOME" }
{ "type": "env", "name": "CACHE", "value": "/tmp/cache" }
{ "type": "env", "name": "TOKEN", "secret": true }
```

| Field | Description |
|---|---|
| `name` | Env var name |
| `value` | Set this value (don't read from shell) |
| `from_env` | If true + value set: use value as default, shell overrides |
| `secret` | Mask value in output |
| `expect` | Type: `string`, `integer`, `boolean`, `url`, `path`, `store-path`, `enum`, `regex` |
| `values` | Enum values (when `expect: "enum"`) |
| `on_missing` | `fail` (default) or `skip` (skip entire besogne) |

## file

Check file/directory/socket existence.

```json
{ "type": "file", "path": "go.mod" }
{ "type": "file", "path": "/var/run/docker.sock", "expect": "socket" }
```

| Field | Description |
|---|---|
| `path` | File path (relative = linked in tmpdir) |
| `expect` | `file`, `directory`, `socket` |
| `permissions` | Expected permissions (e.g., `"0600"`) |

## binary

Resolve a binary via PATH, probe version, hash content.

```json
{ "type": "binary", "name": "go" }
{ "type": "binary", "name": "go", "validate": { "version": { "type": "semver", "range": ">=1.22" } } }
```

Auto-generates variables: `$GO`, `$GO_VERSION`, `$GO_DIR`.

## service

Check TCP/HTTP reachability.

```json
{ "type": "service", "tcp": "localhost:5432" }
{ "type": "service", "http": "http://localhost:8080/health" }
```

## command

Execute a command. Default phase: `exec`.

```json
{ "type": "command", "name": "test", "phase": "exec",
  "run": ["go", "test", "./..."],
  "after": ["build"],
  "ensure": [{ "type": "file", "path": "cover.out" }] }
```

## user

Check user identity and group membership. Uses `getuid()`, `getgroups()`.

```json
{ "type": "user" }
{ "type": "user", "in_group": "docker" }
```

Auto-generates: `$USER_NAME`, `$USER_UID`, `$USER_GID`.

## platform

Check OS and architecture. Uses `uname()`.

```json
{ "type": "platform", "os": "linux", "arch": "x86_64" }
```

Auto-generates: `$PLATFORM_OS`, `$PLATFORM_ARCH`, `$PLATFORM_KERNEL`.

## dns

Resolve a hostname. Uses `getaddrinfo()`.

```json
{ "type": "dns", "host": "registry.internal.io" }
```

## metric

Read system metrics. Uses `/proc` on Linux, `statvfs()` for disk.

```json
{ "type": "metric", "metric": "cpu.count" }
{ "type": "metric", "metric": "memory.available_mb",
  "validate": { "value": { "type": "float", "min": 512 } } }
{ "type": "metric", "metric": "disk.available_gb", "path": "/" }
```

Available metrics: `cpu.count`, `cpu.load_1m/5m/15m`, `memory.total_mb`, `memory.available_mb`, `memory.used_mb`, `disk.total_gb`, `disk.available_gb`, `disk.used_gb`, `swap.total_mb`, `swap.used_mb`.
