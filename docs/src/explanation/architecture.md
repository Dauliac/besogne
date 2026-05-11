# Architecture

## The pipeline

```
manifest.json → besogne build → sealed binary → ./binary → pre checks → exec DAG → done
```

## Module structure

```
src/
├── core (pure)
│   ├── manifest/     Serde types for JSON manifest
│   ├── ir/           Intermediate representation + DAG operations
│   └── compile/      Manifest → IR lowering + binary embedding
│
├── effects (impure)
│   ├── probe/        Lenses: read host environment (9 native types)
│   ├── tracer/       Command execution + wait4 metrics
│   └── runtime/      Orchestrator: pre checks → skip → exec DAG
│
└── ui
    └── output/       Renderers: human, CI, JSON
```

## Binary embedding (trailer protocol)

```
┌────────────────────┐
│ besogne runtime ELF │ ← the Rust binary
├────────────────────┤
│ serialized IR (JSON) │ ← appended by besogne build
├────────────────────┤
│ IR length (8 bytes)  │
│ magic: "BESOGNE\0"  │
└────────────────────┘
```

At runtime, the binary reads its own trailer to extract the IR.

## DAG execution

Exec-phase inputs form a directed acyclic graph via `after:` constraints. besogne computes parallel tiers using topological sort:

```
tier 0: [test, lint]        ← no dependencies, run in parallel
tier 1: [coverage]          ← after: [test]
tier 2: [verify-coverage]   ← after: [coverage]
```

## Dependencies

- `serde` + `serde_json` — manifest and IR serialization
- `blake3` — content hashing
- `petgraph` — DAG operations
- `crossbeam` — parallel precondition probing
- `clap` — CLI for both builder and produced binaries
- `libc` — syscalls (wait4, fork, exec, getuid, uname, statvfs)
- `regex` — pattern matching
- `semver` — version range checking
- `chrono` — timestamps

No tokio, no async, no HTTP client crate. Pure Rust + syscalls.
