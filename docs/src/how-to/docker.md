# Docker integration

## Run a command in a container

```json
{
  "name": "docker-test",
  "inputs": [
    { "type": "binary", "name": "docker" },
    { "type": "command", "name": "test", "phase": "exec",
      "run": ["docker", "run", "--rm", "-v", ".:/app", "-w", "/app",
              "node:22-alpine", "npm", "test"] }
  ]
}
```

## Check Docker daemon is available

Use the `docker/daemon` plugin:

```json
{ "key": "docker", "type": "plugin", "plugin": "docker/daemon" }
```

This checks the socket exists and `docker info` succeeds.

## Check registry auth

```json
{ "key": "registry", "type": "plugin", "plugin": "docker/registry",
  "registry": "ghcr.io" }
```

## Start services with docker compose

```json
{ "type": "command", "name": "start-db", "phase": "exec",
  "run": ["docker", "compose", "up", "-d", "postgres"] },
{ "type": "service", "name": "wait-db", "phase": "exec",
  "tcp": "localhost:5432",
  "after": ["start-db"] },
{ "type": "command", "name": "test", "phase": "exec",
  "run": ["go", "test", "-tags=integration", "./..."],
  "after": ["wait-db"] },
{ "type": "command", "name": "stop-db", "phase": "exec",
  "run": ["docker", "compose", "down"],
  "after": ["test"],
  "always_run": true }
```

`always_run: true` ensures cleanup runs even if tests fail.
