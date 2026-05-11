let { Contract, .. } = import "besogne/plugin.ncl" in

{
  name = "git/branch",
  version = "0.1.0",
  description = "Capture current git branch, optionally validate against expected name or pattern",

  params | {
    expect | String | optional,
    pattern | String | optional,
  },

  defaults = {
    on_fail = "skip",
    expires = "never",
  },

  produces = fun params =>
    let branch_pattern =
      if std.record.has_field "pattern" params then params.pattern
      else if std.record.has_field "expect" params then "^%{params.expect}$"
      else null
    in
    [
      {
        type = "command",
        description = "git current branch",
        exec = ["git", "rev-parse", "--abbrev-ref", "HEAD"],
        extract = {
          format = "text",
          fields = { GIT_BRANCH = "stdout" },
        },
        validate =
          if branch_pattern != null then
            { GIT_BRANCH = { type = "string", pattern = branch_pattern } }
          else {},
      },
    ],
}
