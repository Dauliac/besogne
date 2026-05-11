# Verify idempotency

besogne can detect non-idempotent commands by running them twice and comparing outputs.

## Run verification

```bash
./my-task --verify

# Or via besogne run:
besogne run -- --verify
```

This:
1. Runs all precondition checks (pre phase)
2. Executes all commands (exec phase) — **first run**
3. Cleans `ensure` files (postcondition outputs)
4. Executes all commands again — **second run**
5. Compares fingerprints: exit code, stdout hash, stderr hash, ensure file hashes
6. Reports which commands are idempotent and which are not

## What it catches

- Commands that output timestamps or random values
- Commands that append instead of overwrite (`>>` vs `>`)
- Commands that depend on mutable global state
- Commands with undeclared network dependencies

## Declaring side effects

Commands that are intentionally non-idempotent (deploys, notifications, database migrations) should declare it:

```toml
[inputs.deploy]
type = "command"
phase = "exec"
run = ["kubectl", "apply", "-f", "k8s/"]
side_effects = true
```

Commands with `side_effects = true` are **skipped** during verification — they're known to be impure.

## Reading the output

```
=== verification results ===
  ✓ build idempotent              # same output both runs
  ✓ test idempotent               # same output both runs
  ⊘ deploy (side_effects, skipped) # declared impure, not checked
  ✗ generate-report NOT IDEMPOTENT # different output!
    stdout: run1=a1b2c3d4 run2=e5f6a7b8
    hint: if this is expected, add side_effects = true
```

## When to use

- **CI**: add `--verify` to your first pipeline run to catch non-determinism early
- **Development**: run once when writing a new besogne to validate your assumptions
- **Debugging**: when a besogne produces different results on different machines

## Exit codes

| Code | Meaning |
|---|---|
| 0 | All commands are idempotent (or declared side_effects) |
| 3 | At least one command is NOT idempotent |
