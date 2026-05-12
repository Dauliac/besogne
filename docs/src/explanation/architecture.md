# Architecture

## The pipeline

```
manifest.toml → besogne build → sealed binary → ./binary → DAG eval → done
```

## The unified DAG

Everything is a node. Two kinds:

```
Probe (hashable, no execution)         Action (requires execution)
├── env                                └── command
├── file
├── binary
├── service
├── std  (stdout/stderr/exit_code)
├── source (env map from file/tool)
├── platform
├── dns
├── metric
└── component (expanded at build time)
```

Nodes are connected by `parents:` edges. Edge meaning is derived from types:
- Probe → Action = precondition
- Action → Probe = postcondition
- Action → Action = ordering

## Module structure

```
src/
├── core (pure)
│   ├── manifest/     Serde types for TOML/YAML/JSON manifest
│   ├── ir/           Intermediate representation + DAG operations
│   └── compile/      Manifest → IR lowering + component expansion + binary embedding
│
├── effects (impure)
│   ├── probe/        Lenses: read host environment (9 native probe types)
│   ├── tracer/       Command execution + wait4 metrics + LD_PRELOAD interposer
│   └── runtime/      Orchestrator: incremental DAG evaluation + cache + verify
│
└── ui
    └── output/
        ├── mod.rs    Renderers: human, CI, JSON (OutputRenderer trait)
        ├── style/    Design token system (l1 palette, l2 semantic tokens, l3 components)
        └── views/    View modules: build, run, list, check, dump, status
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

## Content-addressed store

Built binaries live in `~/.cache/besogne/store/{blake3_hash}/binary`. Same IR = same hash = same binary. This enables:

- Cross-project sharing (two projects with identical manifests share one binary)
- Cache coherence (compiler change → new hash → cache miss)
- `.besogne/` symlinks point to store entries

## DAG execution

All nodes form a DAG via `parents:` constraints. besogne computes parallel tiers using topological sort:

```
tier 0: [go-mod, test-go]       ← probes, no parents, parallel
tier 1: [mod-tidy]              ← depends on go-mod
tier 2: [go-sum]                ← postcondition of mod-tidy
tier 3: [mod-download]          ← depends on go-sum
tier 4: [test]                  ← depends on mod-download
tier 5: [test-stdout, test-exit] ← postconditions of test
```

Dirty-bit propagation determines what actually runs. Stable outputs cut propagation — downstream nodes skip if their inputs haven't changed.

## Dependencies

- `serde` + `serde_json` + `serde_yaml` + `toml` — manifest and IR serialization
- `blake3` — content hashing
- `petgraph` — DAG operations
- `crossbeam` — parallel probe evaluation
- `clap` + `clap_complete` + `clap_mangen` — CLI for both builder and produced binaries
- `libc` — syscalls (wait4, fork, exec, getuid, uname, statvfs)
- `regex` — pattern matching
- `semver` — version range checking
- `chrono` — timestamps
- `tree-sitter` + `tree-sitter-bash` — static analysis of shell scripts

No tokio, no async, no HTTP client crate. Pure Rust + syscalls.
