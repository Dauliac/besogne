let { Contract, .. } = import "besogne/plugin.ncl" in

# k8s/kubeconfig: produces env (warmup) + file check (runtime)
#
# Default behavior:
#   - KUBECONFIG env var read at warmup, cached
#   - kubeconfig file verified at runtime (before commands use it)
#
# Override for Nix context:
#   - env_build_check=true → KUBECONFIG hardcoded in binary
#   - file_stage="warmup" → file verified at warmup (immutable store path)

{
  name = "k8s/kubeconfig",
  version = "0.1.0",
  description = "Validate KUBECONFIG env var and kubeconfig file across build/warmup/runtime stages",

  params | {
    path | String | default = "~/.kube/config",
    env_build_check | Bool | default = false,
    file_stage | String | default = "runtime",
  },

  defaults = {
    on_fail = "fail",
  },

  produces = fun params => [
    {
      type = "env",
      name = "KUBECONFIG",
      expect = "path",
      build_check = params.env_build_check,
      # warmup by default (read + cache), build if env_build_check=true
    },
    {
      type = "file",
      path = params.path,
      permissions = "0600",
      stage = params.file_stage,
      # "runtime" by default → goes into runtime.steps
      # "warmup" if overridden → stays in inputs
      # "build" if overridden → verified at compile time
    },
  ],
}
