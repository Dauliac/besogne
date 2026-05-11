# Add preconditions

Preconditions are inputs that must be valid before any command runs. If any fails, the besogne aborts immediately.

## Require an env var

```toml
[inputs.API_TOKEN]
type = "env"
secret = true
```

Set a computed value (not read from shell):

```toml
[inputs.CACHE_DIR]
type = "env"
value = "/tmp/my-cache"
```

## Require a file

```toml
[inputs.config]
type = "file"
path = "config.yaml"
```

## Require a binary in PATH

```toml
[inputs.docker]
type = "binary"
```

With version constraint:

```toml
[inputs.go]
type = "binary"
version = ">=1.22"
```

## Require a service

```toml
[inputs.postgres]
type = "service"
tcp = "localhost:5432"

[inputs.api-health]
type = "service"
http = "http://localhost:8080/health"
```

## Check platform

```toml
[inputs.platform]
type = "platform"
os = "linux"
arch = "x86_64"
```

## Check user/group

```toml
[inputs.docker-group]
type = "user"
in_group = "docker"
```

## Check system resources

```toml
[inputs.memory]
type = "metric"
metric = "memory.available_mb"

[inputs.memory.validate.value]
type = "float"
min = 512

[inputs.disk]
type = "metric"
metric = "disk.available_gb"
path = "/"

[inputs.disk.validate.value]
type = "float"
min = 10
```

## Check DNS

```toml
[inputs.registry-dns]
type = "dns"
host = "registry.internal.io"
```

## Run a probe command

```toml
[inputs.git-clean]
type = "command"
phase = "pre"
run = ["git", "diff", "--quiet", "HEAD"]
```

Note: commands default to `phase = "exec"`. Use `phase = "pre"` to make them preconditions.
