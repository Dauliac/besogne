let { Contract, .. } = import "besogne/plugin.ncl" in
let kubeconfig_plugin = import "k8s/kubeconfig.ncl" in

# k8s/cluster: multi-stage plugin
# Composes k8s/kubeconfig plugin for env + file validation.
#
# Produces (via k8s/kubeconfig):
#   build:   (optional) KUBECONFIG env hardcoded if env_build_check=true
#   warmup:  KUBECONFIG env read + cached
#   runtime: kubeconfig file verified before cluster probes
#
# Produces (own):
#   warmup:  kubectl binary resolved
#   runtime: cluster-info probe, context/namespace metadata extraction

{
  name = "k8s/cluster",
  version = "0.1.0",
  description = "Full Kubernetes cluster validation across build/warmup/runtime stages",

  params | {
    context | String | optional,
    contexts | Array String | optional,
    kubeconfig | String | default = "~/.kube/config",
    namespace | String | optional,
    namespaces | Array String | optional,
    env_build_check | Bool | default = false,
    file_stage | String | default = "runtime",
    probe_stage | String | default = "runtime",
  },

  defaults = {
    on_fail = "fail",
  },

  produces = fun params =>
    let context_flag =
      if std.record.has_field "context" params then "--context=%{params.context}" else ""
    in
    let ns_flag =
      if std.record.has_field "namespace" params then "--namespace=%{params.namespace}" else ""
    in
    let context_validate =
      if std.record.has_field "contexts" params then
        { type = "enum", values = params.contexts }
      else "string"
    in
    let ns_validate =
      if std.record.has_field "namespaces" params then
        { type = "enum", values = params.namespaces }
      else "string"
    in

    # Delegate to k8s/kubeconfig plugin for KUBECONFIG env + file check
    # Stage defaults flow through: env_build_check and file_stage are forwarded
    let kubeconfig_inputs = kubeconfig_plugin.produces {
      path = params.kubeconfig,
      env_build_check = params.env_build_check,
      file_stage = params.file_stage,
    } in

    # kubectl binary — warmup (resolve + version + cache)
    let binary_inputs = [
      {
        type = "binary",
        name = "kubectl",
        stage = "warmup",
      },
    ] in

    # Cluster probe — runtime by default (needs the cluster to be reachable NOW)
    let probe_inputs = [
      {
        type = "command",
        name = "k8s-cluster-info",
        description = "k8s cluster reachable",
        exec = ["kubectl", "cluster-info"]
          @ (if context_flag != "" then [context_flag] else [])
          @ (if ns_flag != "" then [ns_flag] else []),
        env = { KUBECONFIG = "{{ KUBECONFIG }}" },
        extract = {
          format = "regex",
          fields = {
            K8S_SERVER = "running at (https?://[^\\s]+)",
          },
        },
        validate = { K8S_SERVER = "url" },
        stage = params.probe_stage,
      },
      {
        type = "command",
        name = "k8s-context-metadata",
        description = "k8s context metadata",
        exec = ["kubectl", "config", "view", "--minify", "-o", "json"]
          @ (if context_flag != "" then [context_flag] else []),
        env = { KUBECONFIG = "{{ KUBECONFIG }}" },
        extract = {
          format = "json",
          fields = {
            K8S_CONTEXT = "$.contexts[0].name",
            K8S_NAMESPACE = "$.contexts[0].context.namespace",
            K8S_CLUSTER = "$.contexts[0].context.cluster",
            K8S_USER = "$.contexts[0].context.user",
          },
        },
        validate = {
          K8S_CONTEXT = context_validate,
          K8S_NAMESPACE = ns_validate,
        },
        stage = params.probe_stage,
        dependencies = ["k8s-cluster-info"],
      },
    ] in

    kubeconfig_inputs @ binary_inputs @ probe_inputs,
}
