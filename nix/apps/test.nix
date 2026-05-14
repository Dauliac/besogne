{ ... }: {
  perSystem = { config, pkgs, ... }:
    let
      testScript = pkgs.writeShellApplication {
        name = "besogne-test";
        runtimeInputs = [
          config.rust-project.toolchain
          pkgs.pkg-config
          pkgs.nodejs
          pkgs.go
        ];
        text = ''
          echo "==> Running unit + integration tests"
          cargo test
          echo "==> Running e2e tests"
          cargo test --test e2e
        '';
      };
    in
    {
      apps.test = {
        type = "app";
        program = "${testScript}/bin/besogne-test";
      };
    };
}
