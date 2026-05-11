let { Contract, .. } = import "besogne/plugin.ncl" in

{
  name = "redis/service",
  version = "0.1.0",
  description = "Validate Redis is reachable and extract server version",

  params | {
    host | String | default = "localhost",
    port | Number | default = 6379,
  },

  defaults = {
    on_fail = "skip",
    expires = "1m",
  },

  produces = fun params => [
    {
      type = "service",
      tcp = "%{params.host}:%{std.to_string params.port}",
    },
    {
      type = "command",
      description = "redis version",
      exec = ["redis-cli", "-h", params.host, "-p", std.to_string params.port, "INFO", "server"],
      extract = {
        format = "regex",
        fields = { REDIS_VERSION = "redis_version:([\\d.]+)" },
      },
      validate = { REDIS_VERSION = "string" },
    },
  ],
}
