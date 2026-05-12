# CLI

## Builder CLI

```bash
besogne build                              # Auto-discover manifests, build all
besogne build -i besogne.toml -o ./task    # Explicit input/output
besogne check                              # Validate without building
besogne check besogne.toml                 # Validate specific manifest
besogne run                                # Build + run in one shot
besogne run test                           # Build + run specific task
besogne run -- -l json                     # Build + run with JSON output
besogne list                               # List discovered manifests
besogne list -v                            # List with details
besogne adopt -s package.json              # Adopt scripts from package.json
```

### Build

```bash
besogne build                    # Build all discovered manifests
besogne build -i besogne.toml   # Build specific manifest
besogne build -i a.toml -i b.toml  # Build multiple manifests
besogne build -i besogne.toml -o ./my-binary  # Custom output path
```

Multiple manifests are compiled in parallel. Output goes to `.besogne/` symlinks pointing to the content-addressed store (`~/.cache/besogne/store/{hash}/binary`).

### Run

```bash
besogne run                      # Auto-discover, build + run
besogne run test                 # Run a specific task by name
besogne run -i besogne.toml      # Explicit manifest
besogne run -- --force           # Forward flags to produced binary
besogne run -- -l json           # Forward JSON output mode
```

`besogne run` builds the binary if needed (or uses cached), then `exec()`s it. The binary replaces the process — no subprocess overhead.

### List

```bash
besogne list                     # Show discovered tasks
besogne list -v                  # Verbose: show descriptions, node counts
```

### Adopt

```bash
besogne adopt -s package.json           # Generate besogne.toml from package.json
besogne adopt -s package.json --dry-run # Preview without writing
besogne adopt -s package.json -o ci.toml # Custom output path
```

## Produced binary CLI

The produced binary has its own CLI, auto-generated from the manifest:

```bash
./my-task                          # Run (seal + exec phases)
./my-task -l json                  # JSON output (NDJSON)
./my-task -l ci                    # CI output (GitHub Actions annotations)
./my-task --force                  # Force re-probe all seals + append force_args
./my-task --debug                  # Append debug_args + skip cache writes
./my-task --verbose                # Show all details (cached probes, env values, process trees)
./my-task --status                 # Show cached status without re-running
./my-task --dump                   # Show human-friendly summary and exit
./my-task --dump-internal          # Dump raw IR as JSON and exit
./my-task --all                    # Run all phases in one shot
./my-task --config config.toml     # Load flag values from config file
./my-task --completions bash       # Generate shell completions
./my-task --man                    # Generate man page
./my-task --markdown               # Generate markdown documentation
```

### Runtime flags

| Flag | Short | Description |
|---|---|---|
| `--log-format` | `-l` | Output format: `human` (default), `json`, `ci` |
| `--force` | `-f` | Force re-probe all seals and append `force_args` to commands |
| `--debug` | `-d` | Append `debug_args` to commands and skip cache writes |
| `--verbose` | `-v` | Show all details including cached probes, env values, process trees |
| `--status` | `-s` | Show cached status with full details without re-running |
| `--all` | | Run all phases (build + seal + exec) in one shot |
| `--config` | `-c` | Config file path (JSON/YAML/TOML) for flag values |
| `--dump` | | Show human-friendly summary and exit |
| `--dump-internal` | | Dump raw IR as JSON and exit |
| `--completions` | | Generate shell completions (bash, zsh, fish, elvish, powershell) |
| `--man` | | Generate man page and exit |
| `--markdown` | | Generate markdown documentation and exit |

### User-defined flags

Flags defined in the manifest appear as CLI arguments:

```bash
./deploy --env staging --dry-run
./deploy --env production
```

Each flag maps to an env var accessible in commands. Priority: CLI arg > env var > config file > default.

### Output formats

| Format | Flag | When |
|---|---|---|
| Human | `-l human` (default) | Interactive terminal |
| CI | `-l ci` | `CI=true` or explicit |
| JSON | `-l json` | Machine consumption, piped to jq |

### Exit codes

| Code | Meaning |
|---|---|
| 0 | Success (or skipped) |
| 1-125 | Command exit code (pass-through) |
| 2 | Seal violation or build error |
| 126 | Command not executable |
| 127 | Command not found |
