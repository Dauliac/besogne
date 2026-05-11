let { Contract, .. } = import "besogne/plugin.ncl" in

{
  name = "aws/session",
  version = "0.1.0",
  description = "Validate AWS session via STS and extract account metadata",

  params | {
    profile | String | optional,
    region | String | optional,
  },

  defaults = {
    on_fail = "fail",
    expires = "10m",
  },

  produces = fun params => [
    {
      type = "command",
      description = "aws sts identity",
      exec = ["aws", "sts", "get-caller-identity"],
      env =
        (if std.record.has_field "profile" params then { AWS_PROFILE = params.profile } else {})
        & (if std.record.has_field "region" params then { AWS_REGION = params.region } else {}),
      extract = {
        format = "json",
        fields = {
          AWS_ACCOUNT_ID = "$.Account",
          AWS_ARN = "$.Arn",
          AWS_USER_ID = "$.UserId",
        },
      },
      validate = {
        AWS_ACCOUNT_ID = { type = "regex", pattern = "^\\d{12}$" },
        AWS_ARN = "string",
      },
    },
  ],
}
