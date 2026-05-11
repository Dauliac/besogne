let { Contract, .. } = import "besogne/plugin.ncl" in

{
  name = "gcp/auth",
  version = "0.1.0",
  description = "Validate GCP authentication and extract project metadata",

  params | {
    project | String | optional,
  },

  defaults = {
    on_fail = "fail",
    expires = "30m",
  },

  produces = fun params => [
    {
      type = "command",
      description = "gcp auth token",
      exec = ["gcloud", "auth", "print-identity-token", "--quiet"],
      env =
        if std.record.has_field "project" params then
          { CLOUDSDK_CORE_PROJECT = params.project }
        else {},
    },
    {
      type = "command",
      description = "gcp project metadata",
      exec = ["gcloud", "config", "list", "--format=json(core.project,core.account)"],
      extract = {
        format = "json",
        fields = {
          GCP_PROJECT_ID = "$.core.project",
          GCP_ACCOUNT = "$.core.account",
        },
      },
      validate = {
        GCP_PROJECT_ID = "string",
        GCP_ACCOUNT = "string",
      },
    },
  ],
}
