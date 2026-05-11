let { Contract, .. } = import "besogne/plugin.ncl" in
let derivation = import "nix/derivation.ncl" in

# Convenience wrapper around nix/derivation for standard Nix packages.
# Packages are derivations that follow stdenv.mkDerivation conventions:
# - Always have pname and version
# - Always have bin/ with executables
# - May have lib/, share/, etc.
# - Have meta (license, description, homepage)

{
  name = "nix/package",
  version = "0.1.0",
  description = "Declare a standard Nix package (stdenv.mkDerivation) — convenience wrapper around nix/derivation",

  params | {
    pname | String,
    version | String,
    out | String,
    bins | Array String,
    system | String | optional,
    meta | { _ : String } | optional,
  },

  defaults = {
    on_fail = "fail",
  },

  produces = fun params =>
    derivation.produces {
      name = "%{params.pname}-%{params.version}",
      pname = params.pname,
      version = params.version,
      outputs = { out = params.out },
      bins = params.bins,
      system = if std.record.has_field "system" params then params.system else null,
      meta = if std.record.has_field "meta" params then params.meta else null,
    },
}
