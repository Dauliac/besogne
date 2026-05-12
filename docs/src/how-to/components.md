# Use components

Components are reusable manifests. They expand into one or more native nodes at build time.

## Using a component

```toml
[nodes."aws/session"]
type = "component"
```

The map key IS the component reference (resolves to `components/aws/session.json`).

## Depending on a component

Exec-phase commands can depend on nodes produced by a component:

```toml
[nodes.deploy]
type = "command"
phase = "exec"
run = ["deploy", "--account", "$AWS_ACCOUNT_ID"]
parents = ["aws/session.sts-check"]
```

This waits for the `sts-check` node produced by the `aws/session` component.

## Overriding component internals

```toml
[nodes."k8s/cluster".overrides.kubeconfig]
phase = "build"
```

Override keys are component-internal node names.

## Patching array fields

```toml
[nodes."go/deps".patch.download.run]
append = ["-x", "-v"]
```

## Builtin components

### Cloud & Infrastructure

| Component | Description |
|---|---|
| `aws/session` | Validate AWS session |
| `gcp/auth` | Validate GCP auth |
| `k8s/cluster` | Kubernetes cluster access |
| `k8s/kubeconfig` | Kubeconfig validation |
| `k8s/pod-ready` | Wait for pod readiness |
| `k8s/deployment-ready` | Wait for deployment readiness |
| `k8s/service-available` | Wait for service availability |
| `k8s/dns-resolve` | DNS resolution in cluster |
| `k8s/job-complete` | Wait for job completion |

### Docker

| Component | Description |
|---|---|
| `docker/daemon` | Docker daemon check (socket + `docker info`) |
| `docker/registry` | Docker registry access |
| `docker/socket` | Docker socket check |
| `docker/healthcheck` | Container health check |

### Data stores

| Component | Description |
|---|---|
| `postgres/service` | PostgreSQL availability check |
| `postgres/ready` | PostgreSQL readiness check |
| `redis/service` | Redis availability check |
| `redis/ready` | Redis readiness check |

### Toolchains

| Component | Description |
|---|---|
| `go/toolchain` | Go toolchain binaries |
| `go/deps` | Go module dependencies |
| `go/mod-verify` | Go module verification |
| `node/toolchain` | Node.js toolchain |
| `npm/deps` | npm dependencies |
| `python/toolchain` | Python toolchain |
| `uv/deps` | uv (Python) dependencies |
| `gcc/toolchain` | GCC toolchain |
| `coreutils/shell` | Core shell utilities |

### Environment loaders

| Component | Description |
|---|---|
| `env/direnv` | Load env from direnv |
| `env/mise` | Load env from mise |
| `env/nix` | Load env from Nix |
| `env/venv` | Load env from Python venv |
| `env/dotenv` | Load env from .env file |
| `env/conda` | Load env from Conda |

### Version control

| Component | Description |
|---|---|
| `git/branch` | Branch validation |
| `git/clean` | Clean working tree |
| `git/no-fixup` | No fixup commits |
| `git/commit-count` | Commit count check |

### Nix

| Component | Description |
|---|---|
| `nix/package` | Nix package |
| `nix/app` | Nix app |
| `nix/derivation` | Nix derivation |

## Writing a component

Components are JSON files in `components/<category>/<name>.json`. See [Reference: Components](../reference/components.md).
