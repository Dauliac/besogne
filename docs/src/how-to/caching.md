# Cache and skip

All commands are cached by default. If all seal hashes match the last successful run, the besogne is skipped. No configuration needed.

## Default behavior (nothing to do)

```toml
name = "my-task"
description = "This is cached automatically"

[nodes.build]
type = "command"
phase = "exec"
run = ["go", "build", "./..."]
```

This command is cached. If inputs haven't changed, it won't re-run.

## Opt out for impure commands

Commands with side effects (deploy, notifications, database migrations) must declare it:

```toml
[nodes.deploy]
type = "command"
phase = "exec"
run = ["kubectl", "apply", "-f", "k8s/"]
side_effects = true
```

`side_effects = true` means: always run, never skip. This is the only way to disable caching.

## How it works

1. All seal nodes are probed and hashed
2. The combined hash is compared to the cached hash from the last run
3. If they match AND all postcondition files still exist, the besogne exits 0 immediately
4. If any input changed or any output is missing, the besogne runs normally

## What triggers a re-run

- Any file node's content hash changed
- Any env var value changed
- Any binary changed (different version, different hash)
- A postcondition file was deleted
- The manifest itself changed (different besogne hash)

## Clear the cache

Delete the XDG cache directory:

```bash
rm -rf ~/.cache/besogne/
```
