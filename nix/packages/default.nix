{ inputs, ... }: {
  imports = [
    inputs.rust-flake.flakeModules.default
  ];

  perSystem = { config, pkgs, ... }: {
    # Default crane args applied to all crates
    rust-project.defaults.perCrate.crane.args = {
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
