let { Contract, .. } = import "besogne/plugin.ncl" in

{
  name = "git/no-fixup",
  version = "0.1.0",
  description = "Validate no fixup! or squash! commits exist on current branch",

  params | {
    base | String | default = "origin/master",
  },

  defaults = {
    on_fail = "fail",
    expires = "never",
  },

  produces = fun params => [
    {
      type = "command",
      description = "no fixup commits",
      exec = "! git log --oneline %{params.base}..HEAD | grep -qE '^(fixup|squash)!'",
    },
    {
      type = "command",
      description = "list fixup commits",
      exec = "git log --oneline %{params.base}..HEAD | grep -E '^(fixup|squash)!' || true",
      extract = {
        format = "text",
        fields = { GIT_FIXUP_COMMITS = "stdout" },
      },
    },
  ],
}
