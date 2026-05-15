{ inputs, ... }:
let
  src = ../..;
in
{
  imports = [
    ./test.nix
  ];

  perSystem = { config, pkgs, ... }: {
    besogne.nix = {
      enable = true;
      besognePackage = config.packages.besogne;
      componentsDir = ../../components;
      inherit src;

      # Rust source files — build-phase pins.
      # Changed Cargo.toml/Cargo.lock/src → different hash → Nix rebuilds
      # the sealed binary → fresh besogne cache → command re-executes.
      extraNodes = {
        cargo-toml = { type = "file"; path = "${src}/Cargo.toml"; phase = "build"; };
        cargo-lock = { type = "file"; path = "${src}/Cargo.lock"; phase = "build"; };
      };

      tools.devourFlake = {
        enable = true;
        package = pkgs.callPackage inputs.devour-flake { };
      };

      build = {
        enable = true;
        packages = [
          "besogne"
          "besogne-doc"
        ];
      };

      check.enable = true;
      gc.enable = true;
      update.enable = true;
    };
  };
}
