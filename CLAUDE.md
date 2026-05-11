# AGENTS.md — Development directives for besogne

> Read `docs/src/` (mdbook) for full design, tutorials, and reference.
> Run `mdbook serve docs/` to browse locally.

## What besogne is

A Rust tool that compiles TOML/YAML/JSON manifests into self-contained instrumented binaries. Design by Contract for shell commands: typed preconditions, sandboxed execution, memoization, tracing. Inspired by Nix derivations — purity techniques in an impure world.

## Manifest format: nodes are a NAMED MAP

Nodes are `HashMap<String, Node>` — the key IS the name. NOT an array.

```toml
[nodes.go]           # key = binary name
type = "binary"

[nodes.test]         # key = command name (used in parents:)
type = "command"
phase = "exec"
run = ["go", "test"]
parents = ["build"]
```

- `CommandNode` has no `name` — key IS the name
- `ComponentInput` has no `key` — map key IS the component reference
- `BinaryNode.name` optional — defaults to key
- `EnvNode.name` optional — defaults to key

## Terminology (non-negotiable)

| Term | Meaning | NOT |
|---|---|---|
| `besogne build` | Seal manifest into binary | ~~compile~~ |
| `besogne list` | Show discovered manifests | |
| `besogne run` | Build + run in one shot | |
| `phase: "build"` | Sealed at build time | ~~stage: "compile"~~ |
| `phase: "seal"` | Precondition checked at startup | ~~phase: "pre"~~ |
| `phase: "exec"` | Execution DAG step | ~~stage: "runtime"~~ |
| `run:` | Command action field | ~~exec:~~ |
| `parents:` | DAG parents (ordering + binary derivation) | ~~dependencies:~~, ~~after:~~ |
| `side_effects:` | Opt-out of caching (impure) | ~~idempotent: false~~ |
| `sandbox:` | Effect handler config | ~~isolation:~~ |
| `sealed:` | Build-time verified (Nix paths) | ~~build_check:~~ |
| `component` | Reusable manifest (like Kustomize base) | ~~plugin~~ |
| `overrides` | Replace fields on component nodes | |
| `patch` | Array operations (append/prepend/remove) | |
| `components` | Manifest source map (namespace → source) | ~~plugins~~ |

## Components (like Kustomize bases)

A component IS a manifest — same `nodes: {}` format. No separate schema, no params, no templates.

```toml
# Terminal manifest references a component
[nodes."go/deps"]
type = "component"

# Override a node in the component
[nodes."go/deps".overrides.download]
description = "custom download step"

# Patch an array field
[nodes."go/deps".patch.download.run]
append = ["-x", "-v"]
```

Component JSON files live in `components/` directory:
```json
{
  "name": "go/deps",
  "nodes": {
    "go/toolchain": { "type": "component" },
    "go-mod": { "type": "file", "path": "go.mod" },
    "download": { "type": "command", "run": ["go", "mod", "download"],
                   "parents": ["go/toolchain.go", "go-mod"] }
  }
}
```

- Map key IS the component reference: `[nodes."coreutils/shell"]`
- Composition: `type: "component"` nodes inside component `nodes` → recursive expansion
- `components` map in manifest optional — defaults to "builtin"
- `BESOGNE_COMPONENTS_DIR` env var overrides builtin components path

## Project organization convention

```
project/
  besogne/              # Terminal manifests (what people run)
    test.toml           # besogne build → .besogne/test
    build.toml
    deploy.toml
  manifests/            # Shared building blocks (local components)
    go-setup.toml
  .besogne/             # Auto-generated symlinks to cached binaries (gitignored)
    test -> ~/.cache/besogne/store/{hash}/binary
  components/           # Builtin component definitions (JSON)
  besogne.toml          # (optional) Root manifest
```

- `besogne build` with no args builds all discovered manifests (root + `besogne/`)
- `besogne list` shows available tasks with descriptions
- `.besogne/` contains symlinks to global store — same IR = same binary across projects
- Keep composition shallow: 2 levels max (component + terminal manifest)

## Global binary store

Content-addressed: `~/.cache/besogne/store/{blake3_hash}/binary`
- Same IR = same hash = shared across all projects (Nix-store model)
- `besogne build` emits to store, creates `.besogne/` symlinks
- Compiler change → new hash → cache miss (automatic invalidation)

## Formal foundations

Every design decision maps to CS theory:
- **Hoare triple**: `{preconditions} execute {postconditions}`
- **Finite state machine**: INIT → BUILD → SEAL → EXEC → DONE/SKIP/FAIL
- **Algebraic effects**: sandbox = effect handler restricting allowed side effects
- **Memoization**: all commands cached by default (purity assumed), `side_effects: true` opts out
- **Composition**: components = manifests composing manifests (fractal, like Kustomize)
- **Referential transparency**: content-addressed IDs (BLAKE3)
- **Partial order**: DAG execution via topological sort
- **Lenses**: probes = read-only getters on host environment

## Architecture rules

### Pure core, impure effects
- `src/compile/`, `src/ir/`, `src/manifest/` — PURE logic, no I/O
- `src/probe/`, `src/tracer/`, `src/runtime/` — IMPURE, syscalls, I/O
- `src/output/` — rendering (pure transforms of events to strings)

### No unnecessary dependencies
- No tokio, no async. `crossbeam` for parallelism, `std::thread` for tracing.
- No HTTP client crate. Raw TCP + HTTP/1.1 for service checks.
- No template engine. `$VAR` expansion + tree-sitter-bash for analysis.
- No nix crate. Raw `libc` for syscalls.

### Content-addressed everything
- Node IDs: `<type>:<identifier>:<blake3_short>` (e.g., `binary:go:a1b2c3d4`)
- Binary store: `~/.cache/besogne/store/{blake3}/binary`
- BLAKE3 for all hashing (fast, parallel, Rust-native)
- Same content = same hash = same identity (referential transparency)

### Every binary must be declared
- tree-sitter-bash parses all exec forms at build time
- Extracts command names → validates against binary nodes
- Extracts `$VAR` refs → validates against env/extracted nodes
- Script shebangs parsed, sourced files recursively analyzed
- `besogne build` fails if any undeclared dependency found

### Three phases, strict ordering
- **Build**: static validation, seal Nix paths, embed in binary
- **Seal**: parallel precondition probes (crossbeam), no dependencies between them
- **Exec**: DAG execution via `parents:` constraints, tier-based parallelism

## Composition error diagnostics

Errors include full composition provenance chain:
```
error: unknown parent 'go-modd' in node 'download'
  --> component go/deps [nodes.download]
   = note: composition chain: manifest → go/deps → download
   = hint: did you mean 'go-mod'? (edit distance: 1)
```

- Levenshtein distance typo detection on parent references (distance ≤ 2)
- Cycle detection with chain visualization
- Duplicate node detection with location context
- All errors use `DiagBuilder` for rustc-style formatting

## Testing

```bash
nix develop --command cargo test        # all tests
nix develop --command cargo test --test e2e  # e2e only
```

- Unit tests: `src/probe/mod.rs`, `src/runtime/cache.rs`, `src/probe/binary.rs`
- Integration: `tests/integration.rs`, `tests/cache_and_skip.rs`
- E2E: `tests/e2e/` — one dir per use case, each with `besogne.toml` + fixture files
- E2E tests set `XDG_CACHE_HOME` per test to isolate memoization cache
- E2E tests set `BESOGNE_COMPONENTS_DIR` to find builtin components

## When adding a feature

1. Add the manifest field to `src/manifest/types.rs`
2. Add IR representation to `src/ir/types.rs`
3. Wire lowering in `src/compile/lower.rs`
4. Implement probe in `src/probe/` (if new input type)
5. Wire runtime in `src/runtime/mod.rs`
6. Add unit test in the probe module
7. Add e2e test dir in `tests/e2e/<name>/` with `besogne.toml` + fixtures
8. Update docs in `docs/src/`
9. Run `cargo test` — all tests must pass

## Key files

| File | Purpose |
|---|---|
| `src/main.rs` | CLI: `besogne build/run/list/check/adopt`, sealed binary detection |
| `src/manifest/types.rs` | All serde types for manifest |
| `src/manifest/parse.rs` | Manifest loading, discovery (`besogne/` dir), validation |
| `src/ir/types.rs` | IR types: `BesogneIR`, `ResolvedNode`, `ContentId` |
| `src/compile/component.rs` | Component expansion: recursive, overrides, patches |
| `src/compile/lower.rs` | Manifest → IR lowering + dependency resolution + typo detection |
| `src/compile/mod.rs` | Compile pipeline + content-addressed store |
| `src/compile/embed.rs` | Binary trailer protocol (embed/extract IR) |
| `src/ir/dag.rs` | DAG operations with petgraph |
| `src/runtime/mod.rs` | Orchestrator: seal → skip check → exec DAG |
| `src/runtime/cache.rs` | XDG context cache for memoization |
| `src/runtime/cli.rs` | Runtime CLI (flags, dump modes, output format) |
| `src/probe/*.rs` | Native type probes |
| `src/tracer/mod.rs` | fork/exec/wait4 with rusage metrics |
| `src/output/mod.rs` | Human, CI, JSON renderers |
| `src/output/style.rs` | Design tokens + DiagBuilder for rustc-style diagnostics |
| `components/` | Builtin component definitions (JSON) |
