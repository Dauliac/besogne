{ inputs, ... }: {
  imports = [
    inputs.rust-flake.flakeModules.default
  ];

  perSystem = { config, pkgs, ... }: {
    # Default crane args applied to all crates
    rust-project.defaults.perCrate.crane.args = {
      # e2e tests need go, node, podman — not available in sandbox.
      # Tests run via `nix run .#test` or `cargo test` in devShell.
      doCheck = false;
      nativeBuildInputs = with pkgs; [
        pkg-config
      ];
      buildInputs = with pkgs; [
      ] ++ pkgs.lib.optionals pkgs.stdenv.isDarwin [
        pkgs.darwin.apple_sdk.frameworks.Security
        pkgs.darwin.apple_sdk.frameworks.SystemConfiguration
      ];
    };

    packages.default = config.packages.besogne;
  };
}
