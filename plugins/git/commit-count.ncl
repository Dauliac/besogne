let { Contract, .. } = import "besogne/plugin.ncl" in

{
  name = "git/commit-count",
  version = "0.1.0",
  description = "Count commits on current branch relative to base",

  params | {
    base | String | default = "origin/master",
    max | Number | optional,
  },

  defaults = {
    on_fail = "warn",
    expires = "never",
  },

  produces = fun params => [
    {
      type = "command",
      description = "git commit count",
      exec = ["git", "rev-list", "--count", "%{params.base}..HEAD"],
      extract = {
        format = "text",
        fields = { GIT_COMMIT_COUNT = "stdout" },
      },
      validate =
        let base = { type = "integer", min = 0 } in
        if std.record.has_field "max" params then
          { GIT_COMMIT_COUNT = base & { max = params.max } }
        else
          { GIT_COMMIT_COUNT = base },
    },
  ],
}
