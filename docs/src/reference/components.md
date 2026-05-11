# Plugins

Plugins are Nickel files that expand into native inputs. They live in `plugins/<category>/<name>.ncl`.

## Plugin structure

```nickel
{
  name = "aws/session",
  version = "0.1.0",
  description = "Validate AWS session via STS",

  params | {
    profile | String | optional,
    region | String | optional,
  },

  defaults = {
    on_fail = "fail",
  },

  produces = fun params => [
    {
      type = "command",
      description = "aws sts identity",
      run = ["aws", "sts", "get-caller-identity"],
      env = { AWS_PROFILE = params.profile },
      extract = {
        format = "json",
        fields = { AWS_ACCOUNT_ID = "$.Account" },
      },
    },
  ],
}
```

- `params` — Nickel contract on parameters. Validated when the plugin is used.
- `produces` — function from params to a list of native inputs.
- `defaults` — default values applied to produced inputs.

## Plugin composition

Plugins can import other plugins:

```nickel
let kubeconfig_plugin = import "k8s/kubeconfig.ncl" in
{
  produces = fun params =>
    kubeconfig_plugin.produces { path = params.kubeconfig }
    @ own_inputs,
}
```

## Multi-phase plugins

Produced inputs can have different phases:

```nickel
produces = fun params => [
  { type = "env", name = "KUBECONFIG", phase = `seal` },
  { type = "file", path = params.path, phase = "exec" },
  { type = "command", name = "cluster-info", phase = "exec",
    after = ["kubeconfig-file"] },
]
```

## Overrides

Users override plugin internals:

```json
{ "key": "k8s", "type": "plugin", "plugin": "k8s/cluster",
  "overrides": { "KUBECONFIG": { "phase": "build" } } }
```
