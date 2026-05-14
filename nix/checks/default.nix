{ ... }: {
  perSystem = { config, pkgs, ... }: {
    checks = {
      # clippy is auto-wired by rust-flake as besogne-clippy

      formatting = pkgs.runCommand "besogne-fmt-check" {
        nativeBuildInputs = [ config.rust-project.toolchain ];
        src = config.rust-project.src;
      } ''
        cd $src
        cargo fmt --check
        touch $out
      '';
    };
  };
}
