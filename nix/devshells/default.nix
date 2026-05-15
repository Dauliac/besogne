{ ... }: {
  perSystem = { config, pkgs, lib, ... }:
    let
      hasBesogneNix = config.devShells ? besogne-nix;
      besogneNixShell = config.devShells.besogne-nix;
    in
    {
      devShells.default = pkgs.mkShell ({
        inputsFrom = [
          config.devShells.rust
        ] ++ lib.optional hasBesogneNix besogneNixShell;

        nativeBuildInputs = with pkgs; [
          cargo-watch
          cargo-nextest
          bacon

          # e2e test dependencies
          nodejs
          go
          podman
        ];
      }
      # Forward env vars from besogne-nix shell (inputsFrom doesn't propagate them)
      // lib.optionalAttrs (hasBesogneNix && besogneNixShell ? BESOGNE_MANIFESTS_DIR) {
        inherit (besogneNixShell) BESOGNE_MANIFESTS_DIR;
      }
      // lib.optionalAttrs (hasBesogneNix && besogneNixShell ? BESOGNE_COMPONENTS_DIR) {
        inherit (besogneNixShell) BESOGNE_COMPONENTS_DIR;
      });
    };
}
