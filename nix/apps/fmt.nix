{ ... }: {
  perSystem = { config, pkgs, ... }:
    let
      fmtScript = pkgs.writeShellApplication {
        name = "besogne-fmt";
        runtimeInputs = [
          config.rust-project.toolchain
        ];
        text = ''
          echo "==> Formatting Rust code"
          cargo fmt
        '';
      };
    in
    {
      apps.fmt = {
        type = "app";
        program = "${fmtScript}/bin/besogne-fmt";
      };
    };
}
