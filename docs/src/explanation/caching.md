# How caching works

## Idempotent by default

Every besogne command is assumed idempotent and cached by default. There is no `idempotent: true` field -- purity is the norm, like in Haskell where functions are pure unless marked `IO`.

To opt out, declare the command as impure:

```toml
[inputs.deploy]
type = "command"
phase = "exec"
run = ["kubectl", "apply", "-f", "k8s/"]
side_effects = true   # never cached, always runs
```

## The skip decision

```
compute input_hash = blake3(all precondition hashes sorted)
load cached input_hash from XDG cache

input_hash matches?
  no  -> MUST RUN
  yes -> check postconditions still valid on disk
    any missing? -> MUST RUN
    all valid?   -> SKIP (exit 0)
```

## Cache decision matrix

| Has `ensure:`? | `side_effects`? | Behavior |
|---|---|---|
| yes | no | Pure function: inputs -> outputs. Cached in CAS. |
| no | no | Idempotent by assumption. Cached by input hash. |
| no | yes | Impure: deploy, notify, etc. Always runs. |
| yes | yes | Invalid (compile error) -- outputs imply purity. |

## What feeds into the input hash

- Every env var value (or its BLAKE3 hash for secrets)
- Every file's BLAKE3 content hash
- Every binary's content hash + version
- Platform, user, DNS, metric probe results
- Plugin-produced input hashes

## Binary change detection

For binaries (which can be large), besogne uses a hybrid strategy:

1. First run: `stat()` + BLAKE3 hash + version probe -> store all
2. Subsequent runs: check mtime + size (cheap). If unchanged -> trust cached hash. If changed -> re-hash.

BLAKE3 hashes at ~3 GB/s on a single core. A 100MB binary takes ~33ms.

## Two-tier cache

### Tier 1: per-repo (automatic)

```
$XDG_CACHE_HOME/besogne/repo/<repo_hash>/context.json
```

The repo hash is computed from the full IR. Different manifest = different cache. This is always active and safe -- no cross-contamination between projects.

### Tier 2: global CAS (with `sandbox: strict`)

```
$XDG_CACHE_HOME/besogne/cas/<output_hash>/
```

When `sandbox: strict` is set, besogne can guarantee that all inputs are captured (empty env, isolated filesystem, no network). This makes the input hash a **complete** description of the computation, enabling:

- **Cross-repo sharing**: `binary:go:a1b2c3d4` producing the same output in repo A and repo B -> single cache entry
- **Output restoration**: if inputs haven't changed but outputs disappeared, restore from CAS without re-running

The invariant: a command only enters the global CAS if the sandbox enforces complete input capture. Without `sandbox: strict`, implicit dependencies (env vars, filesystem state, network) could make same-hash produce different results.

## Cache invalidation

- Change any input file -> hash changes -> re-run
- Change manifest -> different besogne hash -> new cache
- Delete a postcondition file -> postcondition check fails -> re-run
- `rm -rf ~/.cache/besogne/` -> clear all caches
- Compiler binary hash is part of cache key -> compiler update invalidates
