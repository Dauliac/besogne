# Use plugins

Plugins are reusable input definitions written in Nickel. They expand into one or more native inputs at build time.

## Using a plugin

```json
{ "key": "aws", "type": "plugin", "plugin": "aws/session", "profile": "deploy" }
```

- `key` — reference name (for dependencies and overrides)
- `plugin` — plugin path (resolves to `plugins/aws/session.ncl`)
- Remaining fields — plugin parameters

## Depending on a plugin

Exec-phase commands can depend on a plugin's key:

```json
{ "type": "command", "name": "deploy", "phase": "exec",
  "run": ["deploy", "--account", "$AWS_ACCOUNT_ID"],
  "after": ["aws"] }
```

This waits for ALL exec-phase inputs produced by the `aws` plugin.

## Overriding plugin internals

```json
{ "key": "k8s", "type": "plugin", "plugin": "k8s/cluster",
  "context": "staging",
  "overrides": {
    "KUBECONFIG": { "phase": "build" }
  } }
```

Override keys are plugin-internal names (from the Nickel source).

## Standard plugins

| Plugin | Produces | Parameters |
|---|---|---|
| `aws/session` | command (STS check) | `profile`, `region` |
| `gcp/auth` | command (auth check) | `project` |
| `k8s/cluster` | file + binary + commands | `context`, `namespace`, `kubeconfig` |
| `k8s/kubeconfig` | env + file | `path` |
| `docker/daemon` | file + command | `host` |
| `docker/registry` | command | `registry` |
| `docker/socket` | file | `path` |
| `git/branch` | command | `expect`, `pattern` |
| `git/clean` | command | `ref` |
| `git/no-fixup` | command | `base` |
| `git/commit-count` | command | `base`, `max` |
| `go/mod-verify` | command | (none) |
| `nix/package` | file + binaries | `pname`, `version`, `out`, `bins` |
| `nix/app` | binary | `name`, `program` |
| `nix/derivation` | file + binaries + platform | `name`, `outputs`, `bins`, `executable`, `files` |
| `postgres/service` | service + command | `host`, `port` |
| `redis/service` | service + command | `host`, `port` |

## Writing a plugin

Plugins are Nickel files in `plugins/<category>/<name>.ncl`. See [Reference: Plugins](../reference/plugins.md).
