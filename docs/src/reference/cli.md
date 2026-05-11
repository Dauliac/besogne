# CLI

## Builder CLI

```bash
besogne build -i manifest.json -o ./my-task    # Build a besogne
besogne check manifest.json                     # Validate without building
```

## Produced binary CLI

The produced binary has its own CLI, auto-generated from the manifest:

```bash
./my-task                          # Run (pre + exec phases)
./my-task --log-format json        # JSON output (NDJSON)
./my-task --log-format ci          # CI output (GitHub Actions annotations)
./my-task --dump                   # Show human-friendly summary and exit
./my-task --dump-internal          # Dump raw IR as JSON and exit
./my-task --all                    # Run all phases (build + pre + exec)
```

### User-defined flags

Flags defined in the manifest appear as CLI arguments:

```bash
./deploy --env staging --dry-run
./deploy --env production
```

Each flag maps to an env var accessible in commands.

### Output formats

| Format | Flag | When |
|---|---|---|
| Human | `--log-format human` (default) | Interactive terminal |
| CI | `--log-format ci` | `CI=true` or explicit |
| JSON | `--log-format json` | Machine consumption, piped to jq |

### Exit codes

| Code | Meaning |
|---|---|
| 0 | Success (or skipped) |
| 1-125 | Command exit code (pass-through) |
| 2 | Precondition violation |
| 126 | Command not executable |
| 127 | Command not found |
