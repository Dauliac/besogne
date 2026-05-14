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
        description = "The besogne binary to use for building manifests";
      };

      componentsDir = mkOption {
        type = types.nullOr types.path;
        default = null;
        description = "Path to builtin components directory. Defaults to the one bundled with the besogne package.";
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
              description = "Human-readable description of this besogne task";
            };

            nodes = mkOption {
              type = types.attrsOf types.attrs;
              default = { };
              description = "Node definitions (binary, command, file, env, service, etc.)";
            };

            sandbox = mkOption {
              type = types.attrs;
              default = { };
              description = "Sandbox configuration (priority, network, filesystem restrictions)";
            };

            flags = mkOption {
              type = types.attrs;
              default = { };
              description = "Manifest-level flags";
            };

            components = mkOption {
              type = types.attrs;
              default = { };
              description = "Component source map (namespace to source)";
            };
          };
        }));
        default = { };
        description = "Besogne manifest definitions. Each key becomes a besogne task.";
      };
    };

    config =
      let
        cfg = config.besogne;

        # Generate a TOML manifest file for each declared besogne
        manifestFiles = lib.mapAttrs (name: manifest:
          pkgs.writeText "${name}.toml" (toToml {
            inherit (manifest) name description sandbox flags components nodes;
          })
        ) cfg.manifests;

        # Minimal TOML serializer (besogne manifests are simple enough)
        toTomlValue = v:
          if builtins.typeOf v == "string" then ''"${v}"''
          else if builtins.typeOf v == "int" || builtins.typeOf v == "float" then toString v
          else if builtins.typeOf v == "bool" then (if v then "true" else "false")
          else if builtins.typeOf v == "list" then "[${builtins.concatStringsSep ", " (map toTomlValue v)}]"
          else throw "besogne: unsupported TOML type ${builtins.typeOf v}";

        renderNode = name: node:
          let
            header = ''[nodes."${name}"]'';
            keys = builtins.sort builtins.lessThan (builtins.attrNames node);
            lines = map (k:
              let v = node.${k}; in
              if builtins.typeOf v == "set" then ""
              else "${k} = ${toTomlValue v}"
            ) keys;
          in
          builtins.concatStringsSep "\n" ([ header ] ++ builtins.filter (s: s != "") lines);

        toToml = manifest:
          let
            topFields = lib.filterAttrs (k: v: k != "nodes" && v != { } && v != "") manifest;
            topLines = map (k:
              let v = topFields.${k}; in
              if builtins.typeOf v == "set" then ""
              else "${k} = ${toTomlValue v}"
            ) (builtins.sort builtins.lessThan (builtins.attrNames topFields));
            nodeNames = builtins.sort builtins.lessThan (builtins.attrNames (manifest.nodes or { }));
            nodeBlocks = map (n: renderNode n manifest.nodes.${n}) nodeNames;
          in
          builtins.concatStringsSep "\n\n" (
            builtins.filter (s: s != "") topLines ++ nodeBlocks
          ) + "\n";

        # Build sealed binaries from manifests
        builtBesognes = lib.mapAttrs (name: manifestFile:
          pkgs.runCommand "besogne-${name}" ({
            nativeBuildInputs = [ cfg.package ];
          } // lib.optionalAttrs (cfg.componentsDir != null) {
            BESOGNE_COMPONENTS_DIR = toString cfg.componentsDir;
          }) ''
            mkdir -p $out/bin
            besogne build -i ${manifestFile} -o $out/bin/${name}
          ''
        ) manifestFiles;
      in
      lib.mkIf (cfg.manifests != { }) {
        # Wire each manifest as a package and an app
        packages = lib.mapAttrs' (name: drv:
          lib.nameValuePair "besogne-${name}" drv
        ) builtBesognes;

        apps = lib.mapAttrs' (name: drv:
          lib.nameValuePair "besogne-${name}" {
            type = "app";
            program = "${drv}/bin/${name}";
          }
        ) builtBesognes;
      };
  });
}
