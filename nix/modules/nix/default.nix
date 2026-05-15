{ lib, flake-parts-lib, ... }:
let
  inherit (lib) mkOption mkEnableOption types mkIf mkMerge;
  inherit (flake-parts-lib) mkPerSystemOption;
in
{
  options.perSystem = mkPerSystemOption ({ config, pkgs, ... }:
    let
      cfg = config.besogne.nix;

      # ── Single source of truth for packages ──────────────────────────
      # Both devShell and Nix build derivations use this set.
      # Same Nix derivation → same /nix/store path → same BLAKE3 pin
      # → same besogne IR → shared content-addressed store entry.
      toolPackages = lib.concatLists [
        [ cfg.besognePackage pkgs.bash pkgs.nix ]
        (lib.optional cfg.tools.nom.enable cfg.tools.nom.package)
        (lib.optional cfg.tools.nh.enable cfg.tools.nh.package)
        (lib.optional cfg.tools.devourFlake.enable cfg.tools.devourFlake.package)
        (lib.optional cfg.tools.flakeFile.enable cfg.tools.flakeFile.package)
        (lib.optional cfg.tools.treefmt.enable cfg.tools.treefmt.package)
        (lib.optional cfg.systemManager.enable cfg.systemManager.package)
        cfg.extraPackages
      ];

      # ── Sealed besogne builder ──────────────────────────────────────
      # Each app IS a sealed besogne binary, built at `nix build` time.
      #
      # Flow:
      #   Nix attrset (besogne schema) → JSON manifest → besogne build → sealed binary
      #
      # `besogne build` runs inside the Nix derivation with all tool
      # packages in PATH. Binary pinning resolves to /nix/store/ paths.
      # The resulting sealed binary has the IR embedded with all pins.
      #
      # Cache sharing invariant:
      #   devShell has the SAME packages → `besogne build` in devShell
      #   pins the SAME /nix/store/ paths → produces the SAME IR hash
      #   → the global store at ~/.cache/besogne/store/{hash} is shared.
      # Pinned flake source nodes — injected when cfg.src is set.
      # Nix store paths are immutable → seal phase always passes.
      # Cache invalidation: different flake content → different store path
      # → different manifest JSON → different sealed binary (Nix rebuilds).
      pinnedFlakeNodes = lib.optionalAttrs (cfg.src != null) {
        flake-nix = { type = "file"; path = "${cfg.src}/flake.nix"; phase = "build"; };
        flake-lock = { type = "file"; path = "${cfg.src}/flake.lock"; phase = "build"; };
      };

      # Returns { drv, manifest } — the sealed binary AND the manifest file.
      # The manifest is also exported to BESOGNE_MANIFESTS_DIR for devshell discovery.
      mkBesogneBinary = { name, description ? "", nodes, sandbox ? cfg.sandbox }:
        let
          manifest = {
            inherit name description;
            nodes = pinnedFlakeNodes // cfg.extraNodes // nodes;
          } // lib.optionalAttrs (sandbox != {}) { inherit sandbox; };

          manifestJson = pkgs.writeText "${name}.json" (builtins.toJSON manifest);

          drv = pkgs.runCommand "besogne-${name}" {
            nativeBuildInputs = toolPackages;
          } (''
            cp ${manifestJson} ${name}.json
            mkdir -p $out/bin
          '' + (if cfg.componentsDir != null then ''
            export BESOGNE_COMPONENTS_DIR=${cfg.componentsDir}
          '' else "") + ''
            besogne build -i ./${name}.json -o $out/bin/${name}
          '');
        in
        { inherit drv manifestJson; };

      # Wrap the sealed besogne binary so it runs from the caller's CWD
      # with all tool packages in PATH (needed at runtime for exec-phase commands).
      mkApp = drv: name:
        let
          wrapper = pkgs.writeShellApplication {
            inherit name;
            runtimeInputs = toolPackages;
            text = ''
              exec ${drv}/bin/${name} "$@"
            '';
          };
        in {
          type = "app";
          program = "${wrapper}/bin/${name}";
        };

      # ── Per-package build apps ──────────────────────────────────────
      # Each entry → sealed besogne that runs `nom build .#<name>`.
      # --no-link: don't create ./result symlink
      # --print-out-paths: emit store path to stdout
      # stdout → std node → file child node (exec-phase postcondition)
      # stdout captures the /nix/store/... path of the built derivation.
      # besogne hashes this as a persistent child → backward cache check:
      # if the store path changes (new build), dependent commands re-execute.
      mkPackageBuildBinary = name:
        mkBesogneBinary {
          name = "build-${name}";
          description = "Build .#${name} with nom";
          nodes = {
            "nix/nom" = { type = "component"; };
            build = {
              type = "command";
              description = "Build .#${name} with nix-output-monitor TUI";
              run = [ "nom" "build" ".#${name}" "--no-link" "--print-out-paths" ];
            };
            build-output = {
              type = "std";
              stream = "stdout";
              parents = [ "build" ];
              description = "Nix store path of the built derivation";
            };
          };
        };

      packageBuildResults = lib.listToAttrs (
        map (name:
          lib.nameValuePair "build-${name}" (mkPackageBuildBinary name)
        ) cfg.build.packages
      );

      packageBuildApps = lib.mapAttrs (name: result:
        mkApp result.drv name
      ) packageBuildResults;

      # ── Build-all (devour-flake + nom) ──────────────────────────────
      buildAll = mkBesogneBinary {
        name = "build-all";
        description = "Build every flake output with devour-flake + nom";
        nodes = {
          "nix/nom" = { type = "component"; };
          "nix/flake" = { type = "component"; };
          devour-flake = { type = "binary"; };
          bash = { type = "binary"; };
          build-all = {
            type = "command";
            description = "Build every flake output with nom TUI progress";
            run = [ "bash" "-c" "devour-flake \"path:$(pwd)\" 2>&1 >/dev/null | nom" ];
            side_effects = true;
          };
        };
      };

      # ── Check ───────────────────────────────────────────────────────
      check = mkBesogneBinary {
        name = "nix-check";
        description = "Run nix flake check with nom TUI";
        nodes = {
          "nix/nom" = { type = "component"; };
          "nix/flake" = { type = "component"; };
          check = {
            type = "command";
            description = "Evaluate and build all flake checks";
            run = [ "nom" "flake" "check" ];
            parents = [ "nix/nom.nom" "nix/flake.flake-nix" "nix/flake.flake-lock" ];
            side_effects = true;
          };
        };
      };

      # ── Fmt ─────────────────────────────────────────────────────────
      fmt = mkBesogneBinary {
        name = "nix-fmt";
        description = "Format repository with treefmt";
        nodes = {
          "nix/fmt" = { type = "component"; };
        };
      };

      # ── GC ──────────────────────────────────────────────────────────
      gc = mkBesogneBinary {
        name = "nix-gc";
        description = "Nix garbage collection";
        nodes = {
          "nix/gc" = { type = "component"; };
        };
      };

      # ── Update ──────────────────────────────────────────────────────
      update = mkBesogneBinary {
        name = "nix-update";
        description = "Update flake inputs";
        nodes = {
          "nix/update" = { type = "component"; };
        };
      };

      # ── NixOS switch ────────────────────────────────────────────────
      nixos = mkBesogneBinary {
        name = "nix-switch-nixos";
        description = "Switch NixOS configuration with nh";
        nodes = {
          "nix/switch-nixos" = { type = "component"; }
            // lib.optionalAttrs (cfg.nixos.hostname != "") {
              patch.switch.run = { append = [ "--hostname" cfg.nixos.hostname ]; };
            };
        };
      };

      # ── Home-manager switch ─────────────────────────────────────────
      home = mkBesogneBinary {
        name = "nix-switch-home";
        description = "Switch home-manager configuration with nh";
        nodes = {
          "nix/switch-home" = { type = "component"; }
            // lib.optionalAttrs (cfg.homeManager.configuration != "") {
              patch.switch.run = { append = [ "--configuration" cfg.homeManager.configuration ]; };
            };
        };
      };

      # ── System-manager switch ───────────────────────────────────────
      system = mkBesogneBinary {
        name = "nix-switch-system";
        description = "Switch system-manager configuration";
        nodes = {
          "nix/switch-system" = { type = "component"; };
        };
      };

      # ── Write-flake ─────────────────────────────────────────────────
      writeFlake = mkBesogneBinary {
        name = "nix-write-flake";
        description = "Regenerate flake.nix with flake-file";
        nodes = {
          "nix/flake-file" = { type = "component"; };
          "nix/flake" = { type = "component"; };
          write-flake = {
            type = "command";
            description = "Generate flake.nix from structured definition";
            run = [ "write-flake" ];
            side_effects = true;
          };
        };
      };
    in
    {
      options.besogne.nix = {
        enable = mkEnableOption "Nix workflow integration via besogne components";

        besognePackage = mkOption {
          type = types.package;
          description = ''
            The besogne compiler. Used in both Nix build derivations and devShell.
            Must be the SAME derivation in both contexts for cache sharing.
          '';
        };

        componentsDir = mkOption {
          type = types.nullOr types.path;
          default = null;
          description = "Path to besogne components directory. Defaults to builtin.";
        };

        src = mkOption {
          type = types.nullOr types.path;
          default = null;
          description = ''
            Flake source (typically `self`). When set, file nodes for flake.nix
            and flake.lock use pinned Nix store paths instead of relative paths.
            Nix handles cache invalidation: different content → different store
            path → different manifest → different sealed binary.
          '';
        };

        sandbox = mkOption {
          type = types.attrs;
          default = { priority = "low"; };
          description = "Default sandbox config for all nix workflows";
        };

        extraNodes = mkOption {
          type = types.attrsOf types.attrs;
          default = { };
          description = ''
            Extra nodes injected into ALL generated manifests.
            Use build-phase file nodes to pin source files — Nix rebuilds
            the sealed binary when they change.
          '';
        };

        extraPackages = mkOption {
          type = types.listOf types.package;
          default = [ ];
          description = ''
            Extra packages for binary pinning. Present in both Nix build
            derivations and devShell → same pins → shared cache.
          '';
        };

        # ── Tool toggles ───────────────────────────────────────────
        tools = {
          nom = {
            enable = mkOption { type = types.bool; default = true; description = "nix-output-monitor"; };
            package = mkOption { type = types.package; default = pkgs.nix-output-monitor; };
          };
          nh = {
            enable = mkOption { type = types.bool; default = true; description = "nh (nix helper)"; };
            package = mkOption { type = types.package; default = pkgs.nh; };
          };
          devourFlake = {
            enable = mkOption { type = types.bool; default = true; description = "devour-flake"; };
            package = mkOption { type = types.package; description = "devour-flake package (from flake input)"; };
          };
          flakeFile = {
            enable = mkOption { type = types.bool; default = false; description = "flake-file"; };
            package = mkOption { type = types.package; description = "flake-file package (from flake input)"; };
          };
          treefmt = {
            enable = mkOption { type = types.bool; default = false; description = "treefmt"; };
            package = mkOption { type = types.package; default = pkgs.treefmt2; };
          };
        };

        # ── Build workflows ─────────────────────────────────────────
        build = {
          enable = mkOption {
            type = types.bool;
            default = true;
            description = "build-all app: devour-flake + nom for every flake output.";
          };
          packages = mkOption {
            type = types.listOf types.str;
            default = [ ];
            description = ''
              Package output names. Each generates a sealed besogne `build-<name>` app.
              Example: [ "besogne" "besogne-doc" ] → apps.build-besogne, apps.build-besogne-doc
            '';
          };
        };

        check.enable = mkOption { type = types.bool; default = true; description = "nom flake check"; };
        fmt.enable = mkOption { type = types.bool; default = false; description = "treefmt formatting"; };
        gc.enable = mkOption { type = types.bool; default = false; description = "Nix garbage collection"; };
        update.enable = mkOption { type = types.bool; default = false; description = "Flake input update"; };

        nixos = {
          enable = mkEnableOption "NixOS switch workflow";
          hostname = mkOption { type = types.str; default = ""; };
        };
        homeManager = {
          enable = mkEnableOption "Home-manager switch workflow";
          configuration = mkOption { type = types.str; default = ""; };
        };
        systemManager = {
          enable = mkEnableOption "System-manager switch workflow";
          package = mkOption { type = types.package; description = "system-manager package"; };
        };
        writeFlake.enable = mkOption { type = types.bool; default = false; };
      };

      config =
        let
          # Collect all enabled workflow results for manifest export
          allResults = lib.filterAttrs (_: v: v != null) ({
            build-all = if cfg.build.enable then buildAll else null;
            nix-check = if cfg.check.enable then check else null;
            nix-fmt = if cfg.fmt.enable then fmt else null;
            nix-gc = if cfg.gc.enable then gc else null;
            nix-update = if cfg.update.enable then update else null;
            nix-switch-nixos = if cfg.nixos.enable then nixos else null;
            nix-switch-home = if cfg.homeManager.enable then home else null;
            nix-switch-system = if cfg.systemManager.enable then system else null;
            nix-write-flake = if cfg.writeFlake.enable then writeFlake else null;
          } // packageBuildResults);

          # ── Manifests directory ─────────────────────────────────
          # All terminal manifests in one store path.
          # DevShell sets BESOGNE_MANIFESTS_DIR → `besogne list` discovers them.
          manifestsDir = pkgs.runCommand "besogne-manifests" { } (
            "mkdir -p $out\n" +
            builtins.concatStringsSep "\n" (
              lib.mapAttrsToList (name: result:
                "cp ${result.manifestJson} $out/${name}.json"
              ) allResults
            )
          );
        in
        mkIf cfg.enable (mkMerge [
          # ── Sealed besogne apps ──────────────────────────────────
          (mkIf (cfg.build.packages != [ ]) { apps = packageBuildApps; })
          (mkIf cfg.build.enable { apps.build-all = mkApp buildAll.drv "build-all"; })
          {
            apps = mkMerge [
              (mkIf cfg.check.enable { nix-check = mkApp check.drv "nix-check"; })
              (mkIf cfg.fmt.enable { nix-fmt = mkApp fmt.drv "nix-fmt"; })
              (mkIf cfg.gc.enable { nix-gc = mkApp gc.drv "nix-gc"; })
              (mkIf cfg.update.enable { nix-update = mkApp update.drv "nix-update"; })
              (mkIf cfg.nixos.enable { nix-switch-nixos = mkApp nixos.drv "nix-switch-nixos"; })
              (mkIf cfg.homeManager.enable { nix-switch-home = mkApp home.drv "nix-switch-home"; })
              (mkIf cfg.systemManager.enable { nix-switch-system = mkApp system.drv "nix-switch-system"; })
              (mkIf cfg.writeFlake.enable { nix-write-flake = mkApp writeFlake.drv "nix-write-flake"; })
            ];
          }

          # ── DevShell ─────────────────────────────────────────────
          # Includes besogne CLI + all tools + BESOGNE_MANIFESTS_DIR
          # so `besogne list` discovers all generated terminal manifests.
          {
            devShells.besogne-nix = pkgs.mkShell ({
              name = "besogne-nix";
              packages = toolPackages;
              BESOGNE_MANIFESTS_DIR = "${manifestsDir}";
            } // lib.optionalAttrs (cfg.componentsDir != null) {
              BESOGNE_COMPONENTS_DIR = toString cfg.componentsDir;
            });
          }
        ]);
    }
  );
}
