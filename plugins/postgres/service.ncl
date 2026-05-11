let { Contract, .. } = import "besogne/plugin.ncl" in

{
  name = "postgres/service",
  version = "0.1.0",
  description = "Validate PostgreSQL is reachable and extract server version",

  params | {
    host | String | default = "localhost",
    port | Number | default = 5432,
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
      description = "postgres version",
      exec = ["psql", "-h", params.host, "-p", std.to_string params.port, "-t", "-c", "SELECT version();"],
      extract = {
        format = "regex",
        fields = { POSTGRES_VERSION = "PostgreSQL ([\\d.]+)" },
      },
      validate = { POSTGRES_VERSION = "string" },
    },
  ],
}
