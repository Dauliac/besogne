let { Contract, .. } = import "besogne/plugin.ncl" in

{
  name = "go/mod-verify",
  version = "0.1.0",
  description = "Verify Go module dependencies are consistent with go.sum",

  params | {},

  defaults = {
    on_fail = "fail",
    expires = "1h",
  },

  produces = fun params => [
    {
      type = "command",
      description = "go mod verify",
      exec = ["go", "mod", "verify"],
      extract = {
        format = "text",
        fields = { GO_MOD_STATUS = "stdout" },
      },
      validate = {
        GO_MOD_STATUS = { type = "string", pattern = "verified" },
      },
    },
  ],
}
