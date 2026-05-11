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
├── user
├── platform
├── dns
└── metric
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
│   ├── compile/      Manifest → IR lowering + binary embedding
│   └── content/      ContentFormat trait + per-format parsers (planned)
│
├── effects (impure)
│   ├── probe/        Lenses: read host environment (10 native types)
│   ├── tracer/       Command execution + wait4 metrics
│   └── runtime/      Orchestrator: incremental DAG evaluation
│
└── ui
    └── output/       Renderers: human, CI, JSON (DRY via Metrics struct)
```

## Content validation pipeline

Every node with a `content.<format>` section runs the same pipeline:

```
raw bytes → parse(format) → inline constraints → schema validation → custom check → extract fields → BLAKE3 hash
```

One `ContentFormat` trait per format. Same code path for `std`, `env`, `file`.

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

All nodes form a DAG via `parents:` constraints. besogne computes parallel tiers using topological sort:

```
tier 0: [go-mod, test-go]       ← probes, no parents, parallel
tier 1: [mod-tidy]              ← depends on go-mod
tier 2: [go-sum]                ← postcondition of mod-tidy
tier 3: [mod-download]          ← depends on go-sum
tier 4: [test]                  ← depends on mod-download-exit
tier 5: [test-stdout, test-exit] ← postconditions of test
```

Dirty-bit propagation determines what actually runs. Stable outputs cut propagation — downstream nodes skip if their inputs haven't changed.

## Metrics display

Unified via a `Metrics` struct with `From` impls for `CommandResult`, `CachedCommand`, and `ProcessMetrics`. One formatting function (`format_metrics_human`, `format_metrics_ci`, `format_metrics_json`) used for both command footers and process tree nodes. Emojis, colors, and vocabulary are consistent everywhere.

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

No tokio, no async, no HTTP client crate. Pure Rust + syscalls.
