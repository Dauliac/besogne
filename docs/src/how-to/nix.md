# Use with Nix

besogne integrates deeply with Nix. Store paths are sealed at build time, binaries use absolute paths, and the sandbox is strict by default.

## Nix devShell variant

```toml
name = "npm-install"
description = "Install Node.js dependencies (Nix devShell)"
sandbox = "strict"

[nodes.node]
type = "binary"
sealed = true

[nodes.npm]
type = "binary"
sealed = true

[nodes.package-json]
type = "file"
path = "package.json"

[nodes.lockfile]
type = "file"
path = "package-lock.json"

[nodes.install]
type = "command"
phase = "exec"
run = ["npm", "ci"]

[nodes.node-modules]
type = "file"
path = "node_modules"
expect = "directory"
parents = ["install"]
```

When `sealed = true`, besogne:
- Resolves the binary at build time (during `besogne build`)
- Detects the Nix store path and embeds the absolute path
- Verifies the store path exists (build fails if not)
- Hashes the binary content for cache invalidation

## Nix components

Use builtin components for common Nix patterns:

```toml
[nodes."nix/package"]
type = "component"

[nodes."nix/app"]
type = "component"

[nodes."nix/derivation"]
type = "component"
```

## Source environment from Nix

Use a `source` node to inject Nix environment variables:

```toml
[nodes.nix-env]
type = "source"
format = "json"
path = "nix-env.json"
phase = "build"
select = ["GOPATH", "PATH", "CC"]
```

Or use the `env/nix` component:

```toml
[nodes."env/nix"]
type = "component"
```

## Binary source detection

besogne automatically detects binary sources at build time:
- **Nix**: binaries in `/nix/store/` — version parsed from store path, immutable
- **mise**: binaries in mise install dirs — version parsed from path
- **System**: all other binaries — no safe version detection by default

This detection happens transparently and is embedded in the IR for runtime use.
