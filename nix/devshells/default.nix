{ inputs, ... }: {
  perSystem = { pkgs, system, ... }:
    let
      rustToolchain = pkgs.rust-bin.stable.latest.default.override {
        extensions = [ "rust-src" "rust-analyzer" "clippy" "rustfmt" ];
      };
    in
    {
      devShells.default = pkgs.mkShell {
        nativeBuildInputs = with pkgs; [
          rustToolchain
          pkg-config
          cargo-watch
          cargo-nextest
          bacon
          nickel
          # e2e test dependencies
          nodejs
          go
          podman
        ];

        buildInputs = with pkgs; [
        ] ++ pkgs.lib.optionals pkgs.stdenv.isDarwin [
          pkgs.darwin.apple_sdk.frameworks.Security
          pkgs.darwin.apple_sdk.frameworks.SystemConfiguration
        ];

        RUST_SRC_PATH = "${rustToolchain}/lib/rustlib/src/rust/library";
      };
    };
}
