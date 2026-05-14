{ lib, ... }: {
  lib.besogne = {
    mkManifest = {
      description = "Build a complete besogne manifest from a structured Nix attrset";
      fn = { name, description ? "", nodes, sandbox ? { }, flags ? { }, components ? { } }:
        let
          cleanNodes = lib.filterAttrs (_: v: v != null) nodes;
        in
        { inherit name description sandbox flags components; nodes = cleanNodes; };
      tests."basic manifest" = {
        args = {
          name = "test";
          nodes = {
            go = { type = "binary"; };
          };
        };
        expected = {
          name = "test";
          description = "";
          sandbox = { };
          flags = { };
          components = { };
          nodes = { go = { type = "binary"; }; };
        };
      };
    };

    mkCommand = {
      description = "Create a command node with sensible defaults";
      fn = { run, phase ? "exec", parents ? [ ], side_effects ? false, description ? ""
           , priority ? null, memory_limit ? null, on_missing ? null }:
        { type = "command"; inherit run phase parents side_effects description; }
        // lib.optionalAttrs (priority != null) { inherit priority; }
        // lib.optionalAttrs (memory_limit != null) { inherit memory_limit; }
        // lib.optionalAttrs (on_missing != null) { inherit on_missing; };
      tests."simple command" = {
        args = {
          run = [ "go" "test" "./..." ];
          parents = [ "build" ];
        };
        expected = {
          type = "command";
          run = [ "go" "test" "./..." ];
          phase = "exec";
          parents = [ "build" ];
          side_effects = false;
          description = "";
        };
      };
    };

    mkBinary = {
      description = "Create a binary node";
      fn = { name ? null, sealed ? null }:
        { type = "binary"; }
        // lib.optionalAttrs (name != null) { inherit name; }
        // lib.optionalAttrs (sealed != null) { inherit sealed; };
      tests."bare binary" = {
        args = { };
        expected = { type = "binary"; };
      };
    };

    mkFile = {
      description = "Create a file node";
      fn = { path, phase ? "seal", parents ? [ ] }:
        { type = "file"; inherit path phase; }
        // lib.optionalAttrs (parents != [ ]) { inherit parents; };
      tests."file node" = {
        args = { path = "go.mod"; };
        expected = { type = "file"; path = "go.mod"; phase = "seal"; };
      };
    };

    mkEnv = {
      description = "Create an env node";
      fn = { value ? null, default ? null, phase ? "seal" }:
        { type = "env"; inherit phase; }
        // lib.optionalAttrs (value != null) { inherit value; }
        // lib.optionalAttrs (default != null) { inherit default; };
      tests."env with default" = {
        args = { default = "production"; };
        expected = { type = "env"; phase = "seal"; default = "production"; };
      };
    };

    mkService = {
      description = "Create a service node (host:port probe)";
      fn = { host, port, phase ? "seal" }:
        { type = "service"; inherit host port phase; };
      tests."service node" = {
        args = { host = "localhost"; port = 5432; };
        expected = { type = "service"; host = "localhost"; port = 5432; phase = "seal"; };
      };
    };

    mkComponent = {
      description = "Create a component node reference";
      fn = { overrides ? { }, patch ? { } }:
        { type = "component"; }
        // lib.optionalAttrs (overrides != { }) { inherit overrides; }
        // lib.optionalAttrs (patch != { }) { inherit patch; };
      tests."bare component" = {
        args = { };
        expected = { type = "component"; };
      };
    };

    mkDns = {
      description = "Create a DNS resolution probe node";
      fn = { hostname, phase ? "seal" }:
        { type = "dns"; inherit hostname phase; };
      tests."dns node" = {
        args = { hostname = "example.com"; };
        expected = { type = "dns"; hostname = "example.com"; phase = "seal"; };
      };
    };

    mkPlatform = {
      description = "Create a platform constraint node";
      fn = { os ? null, arch ? null }:
        { type = "platform"; }
        // lib.optionalAttrs (os != null) { inherit os; }
        // lib.optionalAttrs (arch != null) { inherit arch; };
      tests."linux constraint" = {
        args = { os = "linux"; };
        expected = { type = "platform"; os = "linux"; };
      };
    };

    mkSource = {
      description = "Create a source node (dotenv, json, etc.)";
      fn = { path, format ? null, phase ? "exec" }:
        { type = "source"; inherit path phase; }
        // lib.optionalAttrs (format != null) { inherit format; };
      tests."dotenv source" = {
        args = { path = ".env"; format = "dotenv"; };
        expected = { type = "source"; path = ".env"; phase = "exec"; format = "dotenv"; };
      };
    };

    mkStd = {
      description = "Create a std stream capture node (stdout, stderr, exit_code)";
      fn = { stream, parents ? [ ] }:
        assert builtins.elem stream [ "stdout" "stderr" "exit_code" "stdin" ];
        { type = "std"; inherit stream; }
        // lib.optionalAttrs (parents != [ ]) { inherit parents; };
      tests."stdout capture" = {
        args = { stream = "stdout"; parents = [ "build" ]; };
        expected = { type = "std"; stream = "stdout"; parents = [ "build" ]; };
      };
    };

    mkMetric = {
      description = "Create a metric node";
      fn = { name, value ? null, unit ? null }:
        { type = "metric"; inherit name; }
        // lib.optionalAttrs (value != null) { inherit value; }
        // lib.optionalAttrs (unit != null) { inherit unit; };
      tests."metric node" = {
        args = { name = "response_time"; unit = "ms"; };
        expected = { type = "metric"; name = "response_time"; unit = "ms"; };
      };
    };
  };
}
