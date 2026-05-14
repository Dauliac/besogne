{ ... }: {
  perSystem = { config, pkgs, ... }:
    let
      lintScript = pkgs.writeShellApplication {
        name = "besogne-lint";
        runtimeInputs = [
          config.rust-project.toolchain
          pkgs.pkg-config
        ];
        text = ''
          echo "==> Running clippy"
          cargo clippy --all-targets -- -D warnings
          echo "==> Checking formatting"
          cargo fmt --check
        '';
      };
    in
    {
      apps.lint = {
        type = "app";
        program = "${lintScript}/bin/besogne-lint";
      };
    };
}
