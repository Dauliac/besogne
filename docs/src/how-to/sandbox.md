# Sandbox execution

The sandbox controls which effects your commands can have.

## Presets

```json
{ "sandbox": "strict" }
```

| Preset | Env | Filesystem | Network |
|---|---|---|---|
| (none) | inherit all | no restriction | host |
| `"strict"` | only declared inputs | tmpdir (file inputs linked) | none |
| `"container"` | only declared inputs | docker/podman | restricted |

## Custom

Override specific parts:

```json
{
  "sandbox": {
    "preset": "strict",
    "network": "host"
  }
}
```

## Env isolation

In `strict` mode, commands see ONLY:
- Declared `env` inputs (validated values)
- Computed env vars (with `value` field)
- Auto-generated binary variables (`$GO`, `$GO_VERSION`, `$GO_DIR`)

If a command needs `HOME`, declare it: `{ "type": "env", "name": "HOME" }`.

## Tmpdir

In `strict` mode, besogne creates an isolated tmpdir and symlinks all relative file inputs into it. Commands run inside this tmpdir. Absolute paths (like `/var/run/docker.sock`) are not linked — they're just validated to exist.
