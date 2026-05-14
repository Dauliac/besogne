{ ... }: {
  # Export the besogne flake-parts module for downstream consumers.
  # We do NOT import besogne.nix here — it's meant for external use.
  # Downstream: imports = [ inputs.besogne.flakeModules.besogne ];
  flake.flakeModules.besogne = ./besogne.nix;
}
