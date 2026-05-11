let { Contract, .. } = import "besogne/plugin.ncl" in

{
  name = "nix/derivation",
  version = "0.1.0",
  description = "Declare any Nix derivation as input — packages, scripts, runCommand, writeText, multi-output derivations",

  params | {
    name | String,
    pname | String | optional,
    version | String | optional,
    system | String | optional,
    outputs | { _ : String },
    bins | Array String | optional,
    executable | String | optional,
    files | Array String | optional,
    meta | { _ : String } | optional,
  },

  defaults = {
    on_fail = "fail",
  },

  produces = fun params =>
    let nix_metadata = {
      nix_name = params.name,
      nix_pname = if std.record.has_field "pname" params then params.pname else null,
      nix_version = if std.record.has_field "version" params then params.version else null,
    } in

    # Every output path must exist as a directory — verified at build time
    let output_checks =
      params.outputs
      |> std.record.to_array
      |> std.array.map (fun { field, value } => {
        type = "file",
        path = value,
        expect = "directory",
        build_check = true,
        validate = {
          path = { type = "regex", pattern = "^/nix/store/[a-z0-9]{32}-" },
        },
        metadata = nix_metadata & { nix_output = field },
      })
    in

    # Binaries in <out>/bin/ or <bin>/bin/ — verified at build time
    let bin_out =
      if std.record.has_field "bin" params.outputs then params.outputs.bin
      else params.outputs.out
    in
    let bin_checks =
      if std.record.has_field "bins" params then
        params.bins
        |> std.array.map (fun bin_name => {
          type = "binary",
          name = bin_name,
          path = "%{bin_out}/bin/%{bin_name}",
          build_check = true,
          metadata = nix_metadata & { nix_store_path = params.outputs.out },
        })
      else []
    in

    # Derivation IS an executable (writeShellScript, writeShellApplication) — verified at build time
    let executable_checks =
      if std.record.has_field "executable" params then
        [{
          type = "binary",
          name = params.executable,
          path = params.outputs.out,
          build_check = true,
          metadata = nix_metadata,
        }]
      else []
    in

    # Specific files relative to out (runCommand outputs, writeText) — verified at build time
    let file_checks =
      if std.record.has_field "files" params then
        params.files
        |> std.array.map (fun rel_path => {
          type = "file",
          path = "%{params.outputs.out}/%{rel_path}",
          build_check = true,
        })
      else []
    in

    # Platform validation if system is specified
    let platform_checks =
      if std.record.has_field "system" params then
        let parts = std.string.split "-" params.system in
        [{
          type = "platform",
          arch = std.array.first parts,
          os = std.array.last parts,
        }]
      else []
    in

    output_checks @ bin_checks @ executable_checks @ file_checks @ platform_checks,
}
