# Add seals

Seals are nodes that must be valid before any command runs. If any fails, the besogne aborts immediately.

## Require an env var

```toml
[nodes.API_TOKEN]
type = "env"
secret = true
```

Set a computed value (not read from shell):

```toml
[nodes.CACHE_DIR]
type = "env"
value = "/tmp/my-cache"
```

## Require a file

```toml
[nodes.config]
type = "file"
path = "config.yaml"
```

## Require a binary in PATH

```toml
[nodes.docker]
type = "binary"
```

With version constraint:

```toml
[nodes.go]
type = "binary"
version = ">=1.22"
```

## Require a service

```toml
[nodes.postgres]
type = "service"
tcp = "localhost:5432"

[nodes.api-health]
type = "service"
http = "http://localhost:8080/health"
```

## Check platform

```toml
[nodes.platform]
type = "platform"
os = "linux"
arch = "x86_64"
```

## Check system resources

```toml
[nodes.memory]
type = "metric"
metric = "memory.available_mb"

[nodes.disk]
type = "metric"
metric = "disk.available_gb"
path = "/"
```

## Check DNS

```toml
[nodes.registry-dns]
type = "dns"
host = "registry.internal.io"
```

## Load environment from file

```toml
[nodes.dev-env]
type = "source"
format = "dotenv"
path = ".env"
```

## Run a probe command

```toml
[nodes.git-clean]
type = "command"
phase = "seal"
run = ["git", "diff", "--quiet", "HEAD"]
```

Note: commands default to `phase = "exec"`. Use `phase = "seal"` to make them seals.
