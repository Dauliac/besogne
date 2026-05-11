# besogne

**Declarative contracts for shell commands.**

besogne takes a manifest describing what a shell command needs (seals), what it does (execution), and what it produces (postconditions) — then compiles it into a self-contained binary that validates, sandboxes, traces, and memoizes the execution.

```
{seals valid}  execute commands  {postconditions valid}
```

If seals haven't changed since the last successful run and postconditions are still valid, the entire besogne is skipped.

## Key ideas

- **Everything is a named input.** Env vars, files, binaries, services, DNS, platform, user identity, system metrics — all declared, typed, and validated before execution.
- **Three phases.** `build` (seal at compile time), `seal` (check seals at startup), `exec` (run commands in a DAG).
- **Sandbox by default.** Control which env vars, files, and network endpoints your commands can access.
- **Content-addressed.** Every input is identified by its content hash. Same inputs = same result = skip.
- **Plugins in Nickel.** Extend besogne with reusable input definitions (AWS auth, k8s cluster, git checks, etc.).
- **Always-on tracing.** Process tree, CPU, memory, I/O metrics for every subprocess.

## Quick example

TOML:

```toml
name = "npm-install"
description = "Install npm dependencies"

[inputs.npm]
type = "binary"

[inputs.package-json]
type = "file"
path = "package.json"

[inputs.lockfile]
type = "file"
path = "package-lock.json"

[inputs.install]
type = "command"
phase = "exec"
run = ["npm", "install"]

[[inputs.install.ensure]]
type = "file"
path = "node_modules"
expect = "directory"
required = true
```

YAML:

```yaml
name: npm-install
description: Install npm dependencies

inputs:
  npm:
    type: binary
  package-json:
    type: file
    path: package.json
  lockfile:
    type: file
    path: package-lock.json
  install:
    type: command
    phase: exec
    run: ["npm", "install"]
    ensure:
      - type: file
        path: node_modules
        expect: directory
```

```bash
besogne build -i besogne.toml -o ./npm-install
./npm-install          # first run: checks inputs, runs npm install
./npm-install          # second run: SKIP (nothing changed)
echo "change" >> package.json
./npm-install          # third run: re-runs (input changed)

# Or use `besogne run` for build+exec in one shot:
besogne run
```
