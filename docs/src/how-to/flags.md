# Use flags

Flags add CLI arguments to the produced besogne binary. Each flag maps to an env var that commands can read.

## Define flags

```json
{
  "name": "deploy",
  "flags": [
    { "name": "env", "kind": "string", "values": ["staging", "production"],
      "required": true, "description": "Target environment" },
    { "name": "dry-run", "kind": "bool",
      "description": "Preview without applying" },
    { "name": "target", "kind": "positional",
      "description": "Deploy target" }
  ]
}
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

```json
{ "name": "verbose", "kind": "bool", "env": "VERBOSE" }
```

## Use in commands

Flags are available as env vars in all commands:

```json
{ "type": "command", "name": "deploy", "phase": "exec",
  "run": ["sh", "-c", "deploy --env=$DEPLOY_ENV ${DEPLOY_DRY_RUN:+--dry-run}"] }
```

## Short flags

Auto-derived from the flag name. Override with `short`:

```json
{ "name": "verbose", "kind": "bool", "short": "v" }
```
