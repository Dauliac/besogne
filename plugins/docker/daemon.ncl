let { Contract, .. } = import "besogne/plugin.ncl" in

{
  name = "docker/daemon",
  version = "0.1.0",
  description = "Validate Docker daemon is running and extract version metadata",

  params | {
    host | String | optional,
  },

  defaults = {
    on_fail = "fail",
    expires = "5m",
  },

  produces = fun params =>
    let has_host = std.record.has_field "host" params in
    [
      (if !has_host then
        { type = "file", path = "/var/run/docker.sock", expect = "socket" }
      else
        { type = "service", tcp = params.host }),
      {
        type = "command",
        description = "docker daemon info",
        exec = ["docker", "info", "--format", "{{json .}}"],
        env = if has_host then { DOCKER_HOST = params.host } else {},
        extract = {
          format = "json",
          fields = {
            DOCKER_SERVER_VERSION = "$.ServerVersion",
            DOCKER_API_VERSION = "$.ClientInfo.APIVersion",
            DOCKER_OS = "$.OSType",
            DOCKER_ARCH = "$.Architecture",
          },
        },
        validate = {
          DOCKER_SERVER_VERSION = "string",
          DOCKER_OS = "string",
        },
      },
    ],
}
