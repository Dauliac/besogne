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

## Global cache layout

```
$XDG_CACHE_HOME/besogne/<compiler_hash>/<besogne_hash>/context.json
```

Two content-addressed levels:

- **`<compiler_hash>`**: BLAKE3 of the sealed besogne binary. When the compiler is updated or the manifest is rebuilt, a new directory is created. Old directories are garbage-collected on the next save.
- **`<besogne_hash>`**: BLAKE3 of the IR JSON. Two repos with identical manifests produce the same IR, so they share this cache directory automatically.

### Cross-repo sharing

Because the cache is keyed by content (not by repo path), two repos with the same manifest share probe results and skip decisions. The `input_hash` (combined hash of all probe results) still differs if the environments differ (different file contents, different binary versions, etc.), so there is no false sharing.

### Compiler update invalidation

The sealed binary hash changes whenever:
- The besogne compiler itself is updated
- The manifest is modified (different IR -> different binary)

This means compiler updates automatically invalidate all caches. Old `<compiler_hash>` directories are cleaned up (GC) on the next successful save. Only directories that look like hex hashes (16 chars) are removed -- other directories like `run/` and `compiled/` are preserved.

## Cache invalidation

- Change any input file -> hash changes -> re-run
- Change manifest -> different besogne hash -> new cache dir
- Update compiler binary -> different compiler hash -> new cache dir + GC old
- Delete a postcondition file -> postcondition check fails -> re-run
- `rm -rf ~/.cache/besogne/` -> clear all caches
- `--force` flag -> bypass cache, re-probe everything
