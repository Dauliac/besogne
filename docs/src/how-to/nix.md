# Use with Nix

besogne integrates deeply with Nix. Store paths are sealed at build time, binaries use absolute paths, and the sandbox is strict by default.

## Nix devShell variant

```json
{
  "name": "npm-install",
  "version": "0.1.0",
  "description": "Install Node.js dependencies (Nix devShell)",
  "sandbox": "strict",
  "inputs": [
    { "key": "nodejs", "type": "plugin", "plugin": "nix/package",
      "pname": "nodejs", "version": "20.11.0",
      "out": "/nix/store/abc-nodejs-20.11.0", "bins": ["node", "npm", "npx"] },
    { "type": "file", "path": "package.json" },
    { "type": "file", "path": "package-lock.json" },
    { "type": "command", "name": "install", "phase": "exec",
      "run": ["npm", "ci"],
      "ensure": [
        { "type": "file", "path": "node_modules", "expect": "directory", "required": true }
      ] }
  ]
}
```

The `nix/package` plugin:
- Verifies the store path exists at build time (`sealed`)
- Exposes `node`, `npm`, `npx` as binary inputs with absolute paths
- Auto-generates `$NODE`, `$NPM`, `$NPX` variables

## Nix plugins

- `nix/package` — standard packages (stdenv.mkDerivation)
- `nix/app` — flake apps (`apps.<system>.<name>`)
- `nix/derivation` — any derivation (runCommand, writeText, multi-output)

## mkBesogneApp

In your `flake.nix`, use `mkBesogneApp` to generate the manifest and compile in one step:

```nix
packages.my-task = besogne.mkBesogneApp {
  name = "my-task";
  version = "0.1.0";
  inputs = [ pkgs.go pkgs.golangci-lint ];
  # ... generates manifest with nix/package plugins, calls besogne build
};
```
