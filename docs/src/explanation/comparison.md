# Comparison with other tools

besogne is **not a task manager or orchestrator**. It produces a single self-contained binary for one task — with declarative contracts, sandboxing, tracing, and memoization baked in.

## Feature matrix

| Feature | besogne | Task | just | Mage | mise | Dagger |
|---|---|---|---|---|---|---|
| Declarative manifest | TOML/YAML/JSON | YAML | Justfile | Go code | TOML | SDK code |
| Build to single binary | Yes | No | No | Yes | No | No |
| One task = one binary | Yes | No | No | No | No | No |
| Typed seals | 11 native types | No | No | No | No | No |
| Postconditions (DAG nodes) | Yes | No | No | No | No | No |
| Sandbox (env/fs/network) | Yes | No | No | No | No | Container |
| Memoization (default) | Yes | Partial | No | No | No | Yes |
| Process tracing | wait4/rusage | No | No | No | No | No |
| Nix integration | Native (plugins) | No | No | No | No | No |
| Multi-mode output | human/CI/JSON | No | No | No | No | TUI |
| Secret masking | Yes | No | No | No | No | Yes |
| Build-time validation | Full | Schema | Syntax | Compile | No | Types |

## When to use what

**Use besogne when:** you have a shell command (npm install, go test, docker build, deploy script) and you want to declare its seals, sandbox its execution, cache its result, and get structured metrics — all in a self-contained binary callable from any tool.

**Use Task/just when:** you need a lightweight multi-task runner and don't need typed validation, sandboxing, or memoization.

**Use Mage when:** you want compiled task binaries and your team writes Go. You'll implement your own validation.

**Use mise when:** your primary need is polyglot tool version management with integrated tasks.

**Use Dagger when:** you need containerized pipeline steps with caching and your CI has Docker.

## Positioning

```
     Task runners                    Single-task producers
     (many tasks, one config)        (one task, one binary)

 just ── Task ── mise                   besogne ── Mage
                                           │
                                           ├── Declarative (not code)
                                           ├── Typed contracts (Hoare triple)
                                           ├── Sandbox (effect handler)
                                           ├── Memoization
                                           └── Nix-native

     CI/CD platforms
     (pipelines, containers)

          Dagger
```

besogne doesn't orchestrate. It produces hardened executables that orchestrators call.
