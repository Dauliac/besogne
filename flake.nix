{
  description = "besogne — compile manifests into self-contained instrumented binaries";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-parts.url = "github:hercules-ci/flake-parts";

    # Rust build tooling (replaces crane + rust-overlay)
    rust-flake.url = "github:juspay/rust-flake";
    rust-flake.inputs.nixpkgs.follows = "nixpkgs";

    # Build all flake outputs in one shot
    devour-flake.url = "github:srid/devour-flake";
    devour-flake.flake = false;

    # Typed, tested Nix library functions
    nix-lib.url = "github:Dauliac/nix-lib";
    nix-lib.inputs.nixpkgs.follows = "nixpkgs";
  };

  outputs = inputs:
    inputs.nix-lib.mkFlake {
      inherit inputs;
      modules = [
        ./nix/lib
      ];
      flake-parts = inputs.flake-parts;
    } ({ ... }: {
      imports = [
        ./nix
      ];

      systems = [
        "x86_64-linux"
        "aarch64-linux"
        "x86_64-darwin"
        "aarch64-darwin"
      ];
    });
}
