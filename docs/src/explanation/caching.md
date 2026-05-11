# How caching works

## Idempotent by default

Every besogne command is assumed idempotent and cached by default. There is no `idempotent: true` field — purity is the norm, like in Haskell where functions are pure unless marked `IO`.

To opt out, declare the command as impure:

```toml
[nodes.deploy]
type = "command"
run = ["kubectl", "apply", "-f", "k8s/"]
side_effects = true   # never cached, always runs
```

## The incremental DAG algorithm

besogne evaluates nodes in topological order with dirty-bit propagation:

```
for v in topological_sort(G):
  if v is Probe:
    v.hash  = probe(v)
    v.dirty = (v.hash ≠ cache[v])
    cache[v] = v.hash

  if v is Action:
    v.dirty = any parent is dirty

    if not dirty and not side_effects:
      for each postcondition child (probe with this action as parent):
        if probe(child) ≠ cache[child]:
          v.dirty = true    # output drifted
          break

    if dirty or side_effects:
      execute(v)
      re-probe and cache all postcondition children
    else:
      SKIP
```

### Key property: stable outputs cut dirty propagation

If `install` reruns because `package.json` changed, but `node_modules/` hash is the same after — `test` does NOT rerun. The dirty bit stops at stable output nodes.

## Skip scenarios

| Scenario | Preconditions | Postconditions | Action | Downstream |
|---|---|---|---|---|
| First run | FRESH | ∅ | RUN | RUN |
| Nothing changed | CACHED | CACHED | **SKIP** | **SKIP** |
| Source changed | DIRTY | — | RUN | depends on output hash |
| Output deleted | CACHED | DIRTY | RUN (postcond invalidated) | RUN |
| Output same after rebuild | DIRTY | same hash | RUN | **SKIP** |

## Postconditions are just nodes

Instead of a special `postconditions:` field, declare a probe node with the command as parent:

```toml
[nodes.install]
type = "command"
run = ["npm", "ci"]

[nodes.node-modules]
type = "file"
path = "node_modules"
expect = "directory"
parents = ["install"]          # postcondition of install

[nodes.test]
type = "command"
run = ["npm", "test"]
parents = ["node-modules"]     # precondition: depends on the file, not the command
```

The DAG: `install → node-modules → test`

## Command I/O as cache nodes (`std` type)

Exit codes and stdout/stderr are explicit nodes too:

```toml
[nodes.test-exit]
type = "std"
stream = "exit_code"
parents = ["test"]

[nodes.test-exit.content.int]
expect = 0
```

Terminal commands (no postcondition children) use their input hash for skip decisions — if all inputs are unchanged and the command is pure, memoization guarantees the same result.

## Cache layout

```
$XDG_CACHE_HOME/besogne/<compiler_hash>/<besogne_hash>/context.json
```

Two content-addressed levels:

- **`<compiler_hash>`**: BLAKE3 of the sealed besogne binary. Compiler update = new dir = old caches GC'd.
- **`<besogne_hash>`**: BLAKE3 of the IR JSON. Same manifest across repos = shared cache.

## Cache invalidation

- Change any input file → hash changes → re-run
- Change manifest → different besogne hash → new cache dir
- Update compiler → different compiler hash → new cache dir + GC old
- Delete a postcondition file → probe fails → re-run
- `--force` → re-probe everything + append `force_args` to commands
- `--debug` → skip all cache writes (debug output would poison cache)
- `rm -rf ~/.cache/besogne/` → clear all caches

## `force_args` and `debug_args`

Commands can declare extra args for `--force` and `--debug` flags:

```toml
[nodes.build]
type = "command"
run = ["go", "build", "./..."]
force_args = ["-a"]           # appended when --force (invalidate Go cache)
debug_args = ["-v"]           # appended when --debug (verbose output)
```

`--debug` skips all cache writes because debug output differs from normal output and would break output assertions or pollute cached replays.
