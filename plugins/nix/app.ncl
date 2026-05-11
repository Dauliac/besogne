let { Contract, .. } = import "besogne/plugin.ncl" in
let derivation = import "nix/derivation.ncl" in

# Nix flake app: the structured output from flake.nix `apps.<system>.<name>`
#
# In Nix, a flake app is:
#   {
#     type = "app";
#     program = "${packages.myapp}/bin/myapp";
#   }
#
# It references a derivation but is NOT a derivation itself.
# This plugin accepts the app struct + the underlying derivation metadata,
# and produces inputs for both the program binary and its derivation.

{
  name = "nix/app",
  version = "0.1.0",
  description = "Declare a Nix flake app — resolves the app struct, validates the program and its underlying derivation",

  params | {
    # The flake app struct fields
    name | String,
    program | String,

    # The underlying derivation that provides the program
    drv | {
      name | String,
      pname | String | optional,
      version | String | optional,
      outputs | { _ : String },
      bins | Array String | optional,
      files | Array String | optional,
      system | String | optional,
      meta | { _ : String } | optional,
    } | optional,
  },

  defaults = {
    on_fail = "fail",
  },

  produces = fun params =>
    let nix_metadata = {
      nix_name = params.name,
      nix_type = "app",
      nix_program = params.program,
    } in

    # The program binary — always produced
    let program_input = [{
      type = "binary",
      name = params.name,
      path = params.program,
      build_check = true,
      metadata = nix_metadata,
    }] in

    # If the underlying derivation is provided, produce its inputs too
    # This validates the store path, exposes extra binaries, checks files, etc.
    let drv_inputs =
      if std.record.has_field "drv" params then
        derivation.produces params.drv
      else []
    in

    program_input @ drv_inputs,
}
