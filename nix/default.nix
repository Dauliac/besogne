{ ... }: {
  imports = [
    ./overlays
    ./packages
    ./devshells
    ./checks
    ./apps
    ./modules
    # Dogfooding: import our own nix integration module
    ./modules/nix
  ];
}
