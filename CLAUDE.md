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
- `PluginNode` has no `key` — key IS the plugin reference
- `BinaryNode.name` optional — defaults to key
- `EnvNode.name` optional — defaults to key

## Terminology (non-negotiable)

| Term | Meaning | NOT |
|---|---|---|
| `besogne build` | Seal manifest into binary | ~~compile~~ |
| `phase: "build"` | Sealed at build time | ~~stage: "compile"~~ |
| `phase: "pre"` | Precondition checked at startup | ~~stage: "warmup"~~ |
| `phase: "exec"` | Execution DAG step | ~~stage: "runtime"~~ |
| `run:` | Command action field | ~~exec:~~ |
| `postconditions:` | What must be true after command | ~~outputs:~~, ~~ensure:~~ |
| `parents:` | DAG parents (ordering + binary derivation) | ~~dependencies:~~, ~~after:~~ |
| `side_effects:` | Opt-out of caching (impure) | ~~idempotent: false~~ |
| `sandbox:` | Effect handler config | ~~isolation:~~ |
| `sealed:` | Build-time verified (Nix paths) | ~~build_check:~~ |

## Formal foundations

Every design decision maps to CS theory:
- **Hoare triple**: `{preconditions} execute {postconditions}`
- **Finite state machine**: INIT → BUILD → PRE → EXEC → DONE/SKIP/FAIL
- **Algebraic effects**: sandbox = effect handler restricting allowed side effects
- **Memoization**: all commands cached by default (purity assumed), `side_effects: true` opts out
- **Monadic composition**: plugins = `Params → [NativeInput]` (List monad bind)
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
- **Pre**: parallel precondition probes (crossbeam), no dependencies between them
- **Exec**: DAG execution via `parents:` constraints, tier-based parallelism

### Plugins are Nickel
- `.ncl` files in `plugins/` directory
- `params | { ... }` contract + `produces = fun params => [native nodes]`
- Plugins compose by importing other plugins
- `key` for external reference, `overrides` for internal customization

## Testing

```bash
nix develop --command cargo test        # all tests
nix develop --command cargo test --test e2e  # e2e only
```

- Unit tests: `src/probe/mod.rs`, `src/runtime/cache.rs`, `src/probe/binary.rs`
- Integration: `tests/integration.rs`, `tests/cache_and_skip.rs`
- E2E: `tests/e2e/` — one dir per use case, each with `manifest.json` + fixture files + `mise.toml`
- E2E tests set `XDG_CACHE_HOME` per test to isolate memoization cache

## When adding a feature

1. Add the manifest field to `src/manifest/types.rs`
2. Add IR representation to `src/ir/types.rs`
3. Wire lowering in `src/compile/lower.rs`
4. Implement probe in `src/probe/` (if new input type)
5. Wire runtime in `src/runtime/mod.rs`
6. Add unit test in the probe module
7. Add e2e test dir in `tests/e2e/<name>/` with manifest.json + fixtures + mise.toml
8. Update docs in `docs/src/`
9. Run `cargo test` — all tests must pass

## Key files

| File | Purpose |
|---|---|
| `src/main.rs` | CLI: `besogne build` or sealed binary detection |
| `src/manifest/types.rs` | All serde types for JSON manifest |
| `src/ir/types.rs` | IR types: `BesogneIR`, `ResolvedNode`, `ContentId` |
| `src/compile/lower.rs` | Manifest → IR lowering + dependency resolution |
| `src/compile/embed.rs` | Binary trailer protocol (embed/extract IR) |
| `src/ir/dag.rs` | DAG operations with petgraph |
| `src/runtime/mod.rs` | Orchestrator: pre → skip check → exec DAG |
| `src/runtime/cache.rs` | XDG context cache for memoization |
| `src/runtime/cli.rs` | Runtime CLI (flags, dump modes, output format) |
| `src/probe/*.rs` | 9 native type probes |
| `src/tracer/mod.rs` | fork/exec/wait4 with rusage metrics |
| `src/output/mod.rs` | Human, CI, JSON renderers |
