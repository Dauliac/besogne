# Composition model

besogne's composition model draws lessons from Kustomize while avoiding its pain points.

## Parallels with Kustomize

| Kustomize | besogne | Role |
|-----------|---------|------|
| `kustomization.yaml` | `besogne.toml` | Manifest |
| Base | Component | Building block, never applied directly |
| Overlay | Terminal manifest (`besogne/`) | What you actually run |
| Component | Component with overrides | Reusable cross-cutting concern |
| `resources:` | `nodes:` | The nodes in the graph |
| `patchesStrategicMerge:` | `overrides:` | Replace fields |
| `patchesJson6902:` | `patch:` | Array surgery (append/prepend/remove) |

## Components are manifests

A component IS a besogne manifest — same `nodes: {}` format. No separate schema, no template language, no params. This is the "fractal" property: manifests compose manifests.

```json
{
  "name": "go/deps",
  "nodes": {
    "go/toolchain": { "type": "component" },
    "go-mod": { "type": "file", "path": "go.mod" },
    "download": { "type": "command", "run": ["go", "mod", "download"],
                   "parents": ["go/toolchain.go", "go-mod"] }
  }
}
```

## Overrides and patches

Two mechanisms, clear separation:

- **`overrides`**: replace fields entirely (like Kustomize's Strategic Merge Patch)
- **`patch`**: array operations — `append`, `prepend`, `remove` by value (better than Kustomize's JSON Patch because it's not index-based)

```toml
[nodes."go/deps"]
type = "component"

[nodes."go/deps".overrides.download]
description = "custom download"

[nodes."go/deps".patch.download.run]
append = ["-x", "-v"]

[nodes."go/deps".patch.download.parents]
remove = ["go-mod"]
```

## Error provenance

When composition fails, besogne shows the full chain:

```
error: unknown parent 'go-modd' in node 'download'
  --> component go/deps [nodes.download]
   |  "parents": ["go/toolchain.go", "go-modd"]
   |
   = note: composition chain: manifest → go/deps → download
   = hint: did you mean 'go-mod'? (edit distance: 1)
```

## Lessons from Kustomize

1. **Don't over-parameterize**: Kustomize was created as an alternative to Helm's template hell. besogne removed params — overrides and patches are enough.

2. **Keep composition shallow**: Deep overlay chains are Kustomize's biggest anti-pattern. Two levels (component + terminal manifest) is the sweet spot.

3. **Pin remote references**: When remote components arrive, they will be content-addressed (BLAKE3 hash). No unpinned remote bases.

4. **Validate the merged result**: Kustomize patches can silently produce invalid YAML. besogne validates everything at build time.

## Content-addressed store

Built binaries live in `~/.cache/besogne/store/{blake3_hash}/binary`. Same IR = same hash = same binary. This enables:

- Cross-project sharing (two projects with identical manifests share one binary)
- Cache coherence (compiler change → new hash → cache miss)
- Nix-store semantics without Nix
