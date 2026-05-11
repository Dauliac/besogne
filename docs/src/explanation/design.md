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

## The formal model

Every besogne concept maps to a CS primitive:

| Concept | Primitive | Theory |
|---|---|---|
| A besogne | Sealed pure function | Lambda calculus |
| Lifecycle | Finite state machine | Automata theory |
| Inputs | Product type (record) | ADTs |
| Phases (build/pre/exec) | Evaluation strategy | Compilation |
| Plugin expansion | Monadic bind (List monad) | Category theory |
| DAG execution | Partial order | Order theory |
| Content hashing | Referential transparency | Lambda calculus |
| Memoization (default) | Caching pure functions | Dynamic programming |
| `side_effects: true` | IO monad (opt-out of purity) | Type theory |
| Sandbox | Effect handler | Algebraic effects |
| Probes | Lenses (getters) | Optics |
| tree-sitter analysis | Abstract interpretation | Program analysis |

## Three phases: shift-left validation

Move checks as early as possible:

1. **Build** — static validation at `besogne build`. Seal Nix paths, check platform, compile templates. If it fails here, you find out before deployment.
2. **Pre** — precondition checks at startup. Probe env vars, files, binaries, services. All in parallel. If any fails, no command runs.
3. **Exec** — execute commands in DAG order. Check postconditions after each step.

## Purity in an impure world

besogne is not pure (shell commands are inherently impure). It uses purity *techniques*:

- **Declare all inputs** — like Nix derivation inputs
- **Content-address everything** — same inputs = same identity
- **Isolate the environment** — like Nix's build sandbox
- **Memoize by default** — like Nix's substitution; `side_effects: true` opts out (like Haskell's IO)
- **Validate at every stage** — like Design by Contract

The result: shell commands that are declarative, observable, cacheable, and fail-fast.
