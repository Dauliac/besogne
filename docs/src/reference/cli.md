# CLI

## Builder CLI

```bash
besogne build                              # Auto-discover manifest, build
besogne build -i besogne.toml -o ./task    # Explicit input/output
besogne check                              # Validate without building
besogne run                                # Build + run in one shot
besogne run -- --verify                    # Build + run with verification
besogne run -- -l json                     # Build + run with JSON output
```

## Produced binary CLI

The produced binary has its own CLI, auto-generated from the manifest:

```bash
./my-task                          # Run (pre + exec phases)
./my-task -l json                  # JSON output (NDJSON)
./my-task -l ci                    # CI output (GitHub Actions annotations)
./my-task --force                  # Force re-probe all preconditions
./my-task --verify                 # Idempotency verification
./my-task --dump                   # Show human-friendly summary and exit
./my-task --dump-internal          # Dump raw IR as JSON and exit
```

### User-defined flags

Flags defined in the manifest appear as CLI arguments:

```bash
./deploy --env staging --dry-run
./deploy --env production
```

Each flag maps to an env var accessible in commands.

### Idempotency verification

```bash
./my-task --verify
```

Runs the exec phase **twice** and compares fingerprints for each command:
- Exit code
- Stdout hash (BLAKE3)
- Stderr hash (BLAKE3)
- Ensure file hashes (BLAKE3)

Commands with `side_effects = true` are skipped (they're declared non-idempotent).

Output:
```
=== idempotency verification ===
Running exec phase twice to detect non-determinism...

Run 1/2:
  ✓ install  exit=0  stdout=a1b2c3d4  ensure=1
  ✓ test     exit=0  stdout=e5f6a7b8  ensure=0

Run 2/2:
  ✓ install  exit=0  stdout=a1b2c3d4  ensure=1
  ✓ test     exit=0  stdout=e5f6a7b8  ensure=0

=== verification results ===
  ✓ install idempotent
  ✓ test idempotent

verification PASSED — all commands are idempotent
```

If a command produces different output:
```
  ✗ generate-report NOT IDEMPOTENT
    stdout: run1=a1b2c3d4 run2=e5f6a7b8
    hint: if this is expected, add side_effects = true
```

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
| 2 | Precondition violation |
| 3 | Idempotency verification failed |
| 126 | Command not executable |
| 127 | Command not found |
