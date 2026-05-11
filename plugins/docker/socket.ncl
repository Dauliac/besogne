let { Contract, .. } = import "besogne/plugin.ncl" in

{
  name = "docker/socket",
  version = "0.1.0",
  description = "Validate Docker socket exists and is accessible",

  params | {
    path | String | default = "/var/run/docker.sock",
  },

  defaults = {
    on_fail = "fail",
    expires = "5m",
  },

  produces = fun params => [
    {
      type = "file",
      path = params.path,
      expect = "socket",
    },
  ],
}
