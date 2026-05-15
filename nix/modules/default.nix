{ ... }: {
  # Export flake-parts modules for downstream consumers.
  # Dendritic: each module is a self-contained branch.
  #
  # Usage:
  #   imports = [
  #     inputs.besogne.flakeModules.besogne     # base: declare manifests in Nix
  #     inputs.besogne.flakeModules.nix          # nix workflows (build, check, switch, etc.)
  #   ];
  flake.flakeModules = {
    besogne = ./besogne.nix;
    nix = ./nix;
  };
}
