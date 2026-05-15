{ lib, flake-parts-lib, ... }:
let
  inherit (lib) mkOption types;
  inherit (flake-parts-lib) mkPerSystemOption;
in
{
  options.perSystem = mkPerSystemOption ({ config, pkgs, ... }: {
    options.besogne = {
      package = mkOption {
        type = types.package;
        description = "The besogne compiler binary";
      };

      componentsDir = mkOption {
        type = types.nullOr types.path;
        default = null;
        description = "Path to builtin components directory.";
      };

      extraBuildInputs = mkOption {
        type = types.listOf types.package;
        default = [ ];
        description = ''
          Extra packages available during `besogne build` for binary pinning.
          These must be the SAME derivations as in the devShell for cache sharing.
        '';
      };

      manifests = mkOption {
        type = types.attrsOf (types.submodule ({ name, ... }: {
          options = {
            name = mkOption {
              type = types.str;
              default = name;
              description = "Manifest name (defaults to attribute key)";
            };

            description = mkOption {
              type = types.str;
              default = "";
              description = "Human-readable description";
            };

            nodes = mkOption {
              type = types.attrsOf types.attrs;
              default = { };
              description = "Node definitions — same schema as besogne JSON/TOML";
            };

            sandbox = mkOption {
              type = types.attrs;
              default = { };
              description = "Sandbox configuration";
            };

            flags = mkOption {
              type = types.attrs;
              default = { };
              description = "Manifest-level flags";
            };

            components = mkOption {
              type = types.attrs;
              default = { };
              description = "Component source map";
            };
          };
        }));
        default = { };
        description = ''
          Besogne manifest definitions. Each key becomes a sealed besogne binary
          exposed as both a package and a flake app.
          Uses the same schema as besogne JSON manifests — serialized via builtins.toJSON.
        '';
      };
    };

    config =
      let
        cfg = config.besogne;

        # Serialize manifest to JSON and build sealed binary.
        # builtins.toJSON handles all nesting — no custom serializer needed.
        builtBesognes = lib.mapAttrs (key: manifest:
          let
            manifestJson = pkgs.writeText "${key}.json" (builtins.toJSON {
              inherit (manifest) name description nodes;
            } // lib.optionalAttrs (manifest.sandbox != {}) {
              inherit (manifest) sandbox;
            } // lib.optionalAttrs (manifest.flags != {}) {
              inherit (manifest) flags;
            } // lib.optionalAttrs (manifest.components != {}) {
              inherit (manifest) components;
            });
          in
          pkgs.runCommand "besogne-${key}" {
            nativeBuildInputs = [ cfg.package ] ++ cfg.extraBuildInputs;
          } (''
            mkdir -p $out/bin
          '' + lib.optionalString (cfg.componentsDir != null) ''
            export BESOGNE_COMPONENTS_DIR=${cfg.componentsDir}
          '' + ''
            besogne build -i ${manifestJson} -o $out/bin/${manifest.name}
          '')
        ) cfg.manifests;
      in
      lib.mkIf (cfg.manifests != { }) {
        packages = lib.mapAttrs' (key: drv:
          lib.nameValuePair "besogne-${key}" drv
        ) builtBesognes;

        apps = lib.mapAttrs' (key: drv:
          let name = cfg.manifests.${key}.name; in
          lib.nameValuePair "besogne-${key}" {
            type = "app";
            program = "${drv}/bin/${name}";
          }
        ) builtBesognes;
      };
  });
}
