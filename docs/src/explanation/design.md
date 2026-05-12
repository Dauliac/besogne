# Design: contracts for scripts

## The problem

Shell scripts are the glue of software engineering. Every CI pipeline, every Makefile, every deploy script is ultimately shell commands. But shell scripts are:

- **Fragile** — undeclared dependencies break silently
- **Opaque** — no way to know what a script needs without reading it
- **Impure** — they read from and write to the global environment
- **Unobservable** — no metrics, no tracing, no structured output
- **Uncacheable** — re-run everything every time

## The insight

Nix solved this for builds: declare your inputs, isolate the environment, content-address everything, cache by input hash. But Nix is for building software. What about running it?

besogne applies the same principles to shell command execution:

```
{preconditions}  execute  {postconditions}
```

This is a Hoare triple — the foundation of program verification.

## The unified DAG model

Everything is a **node** in a single directed acyclic graph. Two kinds:

- **Probe** — reads the environment (file, env, binary, service, std, ...). Hashable without execution.
- **Action** — executes a command. May produce outputs.

The `parents:` field is the only relationship. Edge meaning is derived from node types:

| Edge | Meaning |
|---|---|
| Probe → Action | **Precondition** — input must be valid before action runs |
| Action → Probe | **Postcondition** — probe verifies action's output |
| Action → Action | **Ordering** — sequencing constraint |

```toml
# precondition          action              postcondition
[nodes.go-mod]    →    [nodes.install]  → [nodes.node-modules]  → [nodes.test]
  type = "file"           type = "command"    type = "file"             type = "command"
  path = "go.mod"         run = [...]         path = "node_modules"     run = [...]
                          parents = ["go-mod"] parents = ["install"]    parents = ["node-modules"]
```

Commands depend on **outputs**, not on other commands. The output nodes are the connectors. This gives finer-grained caching — if `node_modules` doesn't change after `install`, `test` can skip.

## The `std` type: command I/O as DAG nodes

Command stdout, stderr, and exit code are explicit nodes:

```toml
[nodes.test-stdout]
type = "std"
stream = "stdout"
parents = ["test"]
contains = ["PASS"]
```

This replaces the need for special `output:` or `postconditions:` fields — command I/O is just another probe node in the DAG.

Piping is just a `std` node connecting two commands:

```toml
[nodes.generate-out]
type = "std"
stream = "stdout"
parents = ["generate"]

[nodes.format]
type = "command"
run = ["jq", ".data"]
parents = ["generate-out"]
```

## The formal model

| Concept | Primitive | Theory |
|---|---|---|
| A besogne | Sealed pure function | Lambda calculus |
| Unified DAG | Bipartite graph (probes + actions) | Graph theory |
| `parents:` edges | Partial order | Order theory |
| Content hashing | Referential transparency | Lambda calculus |
| Memoization (default) | Caching pure functions | Dynamic programming |
| Dirty propagation | Fixed-point on DAG | Dataflow analysis |
| Stable output shortcut | Output-addressed caching | Content-addressed storage |
| `side_effects: true` | IO monad (opt-out of purity) | Type theory |
| Sandbox | Effect handler | Algebraic effects |
| Component expansion | Composition (manifests compose manifests) | Category theory |
| Probes | Lenses (getters) | Optics |

## Purity in an impure world

besogne is not pure (shell commands are inherently impure). It uses purity *techniques*:

- **Declare all inputs** — like Nix derivation inputs
- **Content-address everything** — same inputs = same identity
- **Isolate the environment** — like Nix's build sandbox
- **Memoize by default** — like Nix's substitution; `side_effects: true` opts out
- **Validate at every stage** — like Design by Contract
- **Stable outputs cut propagation** — like incremental build systems (Make, Bazel)

The result: shell commands that are declarative, observable, cacheable, and fail-fast.
