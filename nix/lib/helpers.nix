{ lib, config, ... }: {
  lib.besogne = {
    toToml = {
      description = "Convert a besogne manifest attrset to TOML string";
      fn = manifest:
        let
          inherit (builtins) typeOf concatStringsSep attrNames;

          toTomlValue = v:
            if typeOf v == "string" then ''"${v}"''
            else if typeOf v == "int" || typeOf v == "float" then toString v
            else if typeOf v == "bool" then (if v then "true" else "false")
            else if typeOf v == "list" then "[${concatStringsSep ", " (map toTomlValue v)}]"
            else throw "besogne.toToml: unsupported type ${typeOf v}";

          renderNode = name: node:
            let
              header = ''[nodes."${name}"]'';
              keys = builtins.sort builtins.lessThan (attrNames node);
              lines = map (k:
                let v = node.${k}; in
                if typeOf v == "set" then ""
                else ''${k} = ${toTomlValue v}''
              ) keys;
            in
            concatStringsSep "\n" ([ header ] ++ builtins.filter (s: s != "") lines);

          topFields = lib.filterAttrs (k: v: k != "nodes" && v != { } && v != "") manifest;
          topLines = map (k:
            let v = topFields.${k}; in
            if typeOf v == "set" then ""
            else "${k} = ${toTomlValue v}"
          ) (builtins.sort builtins.lessThan (attrNames topFields));

          nodeNames = builtins.sort builtins.lessThan (attrNames (manifest.nodes or { }));
          nodeBlocks = map (n: renderNode n manifest.nodes.${n}) nodeNames;
        in
        concatStringsSep "\n\n" (
          builtins.filter (s: s != "") topLines ++ nodeBlocks
        ) + "\n";
      tests."simple manifest to toml" = {
        args = {
          name = "test";
          description = "";
          sandbox = { };
          flags = { };
          components = { };
          nodes = {
            go = { type = "binary"; };
          };
        };
        expected = ''
          name = "test"

          [nodes."go"]
          type = "binary"
        '' + "\n";
      };
    };

    writeManifest = {
      description = "Write a besogne manifest attrset to a TOML file in the Nix store";
      fn = { pkgs, name, manifest }:
        pkgs.writeText "${name}.toml" (config.lib.besogne.toToml manifest);
    };

    buildBesogne = {
      description = "Build a sealed besogne binary from a Nix manifest definition";
      fn = { pkgs, besogne, name, manifest }:
        let
          manifestFile = config.lib.besogne.writeManifest { inherit pkgs name manifest; };
        in
        pkgs.runCommand "besogne-${name}" {
          nativeBuildInputs = [ besogne ];
        } ''
          mkdir -p $out/bin
          besogne build -i ${manifestFile} -o $out/bin/${name}
        '';
    };
  };
}
