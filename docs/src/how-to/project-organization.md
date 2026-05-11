# Project organization

besogne uses a convention-over-configuration approach for organizing manifests in a project. This page describes the recommended layout and the tools that support it.

## Convention

```
project/
  besogne/                    # Terminal manifests — what people run
    test.toml                 # besogne build → .besogne/test
    build.toml                # besogne build → .besogne/build
    deploy.toml               # besogne build → .besogne/deploy
    ci.toml                   # besogne build → .besogne/ci
  manifests/                  # Shared building blocks (local components)
    go-setup.toml             # Referenced via [components] local = "./manifests"
    docker-build.toml
  .besogne/                   # Auto-generated — symlinks to cached binaries
    test -> ~/.cache/besogne/store/{hash}/binary
    build -> ~/.cache/besogne/store/{hash}/binary
  besogne.toml                # (optional) Root manifest — the default task
```

### Terminal vs non-terminal manifests

Like Kustomize's bases and overlays, besogne distinguishes:

- **Components** (`components/` or `manifests/`): building blocks, never run directly. They define reusable node sets (toolchains, deps, environment loaders).
- **Terminal manifests** (`besogne/`): what people actually run. They compose components and add project-specific nodes.

### The `besogne/` directory

Files in `besogne/` are auto-discovered by `besogne build` and `besogne list`:

```sh
# Build all terminal manifests
besogne build

# List available tasks
besogne list
  test      Run Go tests with race detection
  build     Build Docker image
  deploy    Deploy to staging
  4 tasks in besogne/
```

### The `.besogne/` directory

When `besogne build` runs without `-o`, it:
1. Compiles each manifest into the global store (`~/.cache/besogne/store/{hash}/binary`)
2. Creates symlinks in `.besogne/` pointing to the store entries

Same IR = same hash = same binary. Two projects with identical manifests share the same cached binary. Add `.besogne/` to `.gitignore`.

## Keep composition shallow

Two levels of composition is the sweet spot:
1. Builtin or local components (the building blocks)
2. Terminal manifest that composes them

Deep nesting (3+ levels) is a code smell — it makes error diagnosis harder and manifests harder to reason about.

## Integration with existing tools

besogne wraps existing tools (npm, go, make, cargo) — it doesn't replace them:

```toml
# besogne/test.toml
[nodes."go/deps"]
type = "component"

[nodes.test]
type = "command"
run = ["go", "test", "-race", "./..."]
parents = ["go/deps.download"]
```

What besogne adds: precondition probes, caching, telemetry, DAG ordering, idempotency verification, and Design by Contract.
