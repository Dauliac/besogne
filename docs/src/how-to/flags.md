# Use flags

Flags add CLI arguments to the produced besogne binary. Each flag maps to an env var that commands can read.

## Define flags

```toml
[[flags]]
name = "env"
kind = "string"
values = ["staging", "production"]
required = true
description = "Target environment"

[[flags]]
name = "dry-run"
kind = "bool"
description = "Preview without applying"

[[flags]]
name = "target"
kind = "positional"
description = "Deploy target"
```

## Flag kinds

| Kind | CLI usage | Default |
|---|---|---|
| `bool` | `--dry-run` | false |
| `string` | `--env staging` | (none or `default`) |
| `positional` | `./deploy myapp` | (none or `default`) |

## Env var mapping

Each flag auto-generates an env var: `<BESOGNE_NAME>_<FLAG_NAME>`.

| Flag | Besogne name | Env var |
|---|---|---|
| `env` | `deploy` | `DEPLOY_ENV` |
| `dry-run` | `deploy` | `DEPLOY_DRY_RUN` |

Override with explicit `env` field:

```toml
[[flags]]
name = "verbose"
kind = "bool"
env = "VERBOSE"
```

## Use in commands

Flags are available as env vars in all commands:

```toml
[nodes.deploy]
type = "command"
phase = "exec"
run = ["sh", "-c", "deploy --env=$DEPLOY_ENV ${DEPLOY_DRY_RUN:+--dry-run}"]
```

## Short flags

Override with `short`:

```toml
[[flags]]
name = "verbose"
kind = "bool"
short = "v"
```

## Subcommands

Group flags under subcommands:

```toml
[[flags]]
name = "timeout"
kind = "string"
default = "30"
subcommand = "integration"
description = "Test timeout in seconds"
```

## Config files

Load flag values from a config file (JSON/YAML/TOML):

```bash
./my-task --config config.toml
```

Priority: CLI arg > env var > config file > default.

Config file uses flag names as keys:
```toml
env = "staging"
timeout = "60"
```

For subcommand flags, use nested keys:
```toml
[integration]
timeout = "600"
```

## Long-form documentation

Add `doc` for `--help` long descriptions:

```toml
[[flags]]
name = "env"
kind = "string"
description = "Target environment"
doc = """
The deployment environment to target.
Use 'staging' for pre-production testing.
Use 'production' only after staging validation.
"""
```
