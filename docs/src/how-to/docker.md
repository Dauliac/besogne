# Docker integration

## Run a command in a container

```toml
name = "docker-test"
description = "Run tests in Docker"

[nodes.docker]
type = "binary"

[nodes.test]
type = "command"
phase = "exec"
run = ["docker", "run", "--rm", "-v", ".:/app", "-w", "/app",
       "node:22-alpine", "npm", "test"]
```

## Check Docker daemon is available

Use the `docker/daemon` component:

```toml
[nodes."docker/daemon"]
type = "component"
```

This checks the socket exists and `docker info` succeeds.

## Check registry auth

```toml
[nodes."docker/registry"]
type = "component"
```

## Start services with docker compose

```toml
[nodes.start-db]
type = "command"
phase = "exec"
run = ["docker", "compose", "up", "-d", "postgres"]

[nodes.wait-db]
type = "service"
phase = "exec"
tcp = "localhost:5432"
parents = ["start-db"]

[nodes.test]
type = "command"
phase = "exec"
run = ["go", "test", "-tags=integration", "./..."]
parents = ["wait-db"]

[nodes.stop-db]
type = "command"
phase = "exec"
run = ["docker", "compose", "down"]
parents = ["test"]
side_effects = true
```

`side_effects = true` ensures cleanup always runs (never cached, always executes).
