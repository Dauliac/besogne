# Sandbox execution

The sandbox controls which effects your commands can have.

## Presets

```toml
sandbox = "strict"
```

| Preset | Env | Filesystem | Network |
|---|---|---|---|
| (none) | inherit all | no restriction | host |
| `"strict"` | only declared nodes | tmpdir (file nodes linked) | none |
| `"container"` | only declared nodes | docker/podman | restricted |

## Custom

Override specific parts:

```toml
[sandbox]
preset = "strict"
network = "host"
```

Options:
- `env`: `"strict"` (declared only) or `"inherit"` (all)
- `tmpdir`: `true` (isolated) or `false`
- `network`: `"none"`, `"host"`, or `"restricted"`

## Env isolation

In `strict` mode, commands see ONLY:
- Declared `env` nodes (validated values)
- Computed env vars (with `value` field)
- Auto-generated binary variables (`$GO`, `$GO_VERSION`, `$GO_DIR`)

If a command needs `HOME`, declare it:

```toml
[nodes.home]
type = "env"
name = "HOME"
```

## Tmpdir

In `strict` mode, besogne creates an isolated tmpdir and symlinks all relative file nodes into it. Commands run inside this tmpdir. Absolute paths (like `/var/run/docker.sock`) are not linked — they're just validated to exist.
