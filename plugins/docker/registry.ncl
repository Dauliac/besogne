let { Contract, .. } = import "besogne/plugin.ncl" in

{
  name = "docker/registry",
  version = "0.1.0",
  description = "Validate Docker registry authentication and extract user info",

  params | {
    registry | String,
  },

  defaults = {
    on_fail = "fail",
    expires = "15m",
  },

  produces = fun params => [
    {
      type = "command",
      description = "docker registry auth",
      exec = ["docker", "login", "--get-login", params.registry],
      extract = {
        format = "text",
        fields = { REGISTRY_USER = "stdout" },
      },
      validate = { REGISTRY_USER = "string" },
    },
  ],
}
