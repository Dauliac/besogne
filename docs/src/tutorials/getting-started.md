# Getting started

This tutorial walks you through creating your first besogne: a "hello world" that validates a seal and runs a command.

## Prerequisites

- besogne binary built and in your PATH
- A terminal

## Step 1: Create a manifest

Create `besogne.toml`:

```toml
name = "hello"
description = "My first besogne"

[nodes.USER]
type = "env"

[nodes.greet]
type = "command"
phase = "exec"
run = ["echo", "hello from besogne"]
```

This declares:
- A seal: the `USER` env var must be set (key = env var name)
- An execution step named `greet`: run `echo hello from besogne`

## Step 2: Build and run

```bash
# Build explicitly:
besogne build -o ./hello
./hello

# Or build + run in one shot:
besogne run
```

Output:
```
hello — My first besogne
  checking 1 seal...
  ✓ env:USER

▶ greet: echo hello from besogne
    hello from besogne
  ✓ greet  0.001s

✅ 0.002s
```

The binary:
1. Checked that `USER` env var exists (seal)
2. Ran `echo` (execution)
3. Reported timing (tracing)

## Step 3: See what happens on failure

Unset the required env var:

```bash
env -u USER ./hello
```

Output:
```
hello — My first besogne
  checking 1 seal...
  ✗ env:USER — env var 'USER' is not set

✗ FAILED exit 2  0.001s
```

The command never ran. besogne failed fast at the seal check.

## Next steps

- [Wrap npm install](./npm-install.md) — a real-world example with caching
- [Reference: manifest schema](../reference/manifest.md) — all fields explained
