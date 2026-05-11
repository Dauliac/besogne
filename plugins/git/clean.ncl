let { Contract, .. } = import "besogne/plugin.ncl" in

{
  name = "git/clean",
  version = "0.1.0",
  description = "Validate git working tree has no uncommitted changes",

  params | {
    ref | String | default = "HEAD",
  },

  defaults = {
    on_fail = "skip",
    expires = "never",
  },

  produces = fun params => [
    {
      type = "command",
      description = "git tree clean",
      exec = ["git", "diff", "--quiet", params.ref],
    },
    {
      type = "command",
      description = "git dirty files",
      exec = ["git", "status", "--porcelain"],
      extract = {
        format = "text",
        fields = { GIT_DIRTY_FILES = "stdout" },
      },
    },
  ],
}
