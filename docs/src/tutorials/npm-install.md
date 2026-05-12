# Wrap npm install

This tutorial creates a besogne that wraps `npm install` with seal checking and memoization. If `package.json` and `package-lock.json` haven't changed, the install is skipped entirely.

## The manifest

Create `besogne.toml`:

```toml
name = "npm-install"
description = "Install npm dependencies"

[nodes.node]
type = "binary"
version = ">=18"

[nodes.npm]
type = "binary"

[nodes.package-json]
type = "file"
path = "package.json"

[nodes.lockfile]
type = "file"
path = "package-lock.json"

[nodes.install]
type = "command"
phase = "exec"
run = ["npm", "install"]

[nodes.node-modules]
type = "file"
path = "node_modules"
expect = "directory"
parents = ["install"]
```

What this declares:
- **Seals**: `node` (>= 18) and `npm` in PATH, `package.json` and lock file exist
- **Execution**: run `npm install`
- **Postcondition**: `node_modules/` must exist after (declared as a file node with `parents = ["install"]`)
- **Memoization**: cached by default — skip if nothing changed

## Build and run

```bash
besogne run              # first run: installs
besogne run              # second run: SKIP (nothing changed)
echo '{}' >> package.json
besogne run              # third run: re-runs (input changed)
```

## Use in a Makefile

```makefile
node_modules: package.json package-lock.json
	besogne run
```
