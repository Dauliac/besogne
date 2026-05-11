//! Impurity detection heuristics for shell commands.
//!
//! Besogne assumes purity by default (everything is cached).
//! This module detects patterns that indicate side effects,
//! so `besogne adopt` can mark them with `side_effects = true`.

/// Known impure command patterns: commands that mutate external state.
const IMPURE_COMMANDS: &[&str] = &[
    // Network mutations
    "curl",
    "wget",
    "ssh",
    "scp",
    "rsync",
    "ftp",
    "sftp",
    // Container/deploy
    "docker",
    "podman",
    "kubectl",
    "helm",
    "terraform",
    "ansible",
    "ansible-playbook",
    // Package publishing
    "npm",  // only with publish subcommand — checked below
    "yarn", // only with publish
    "pnpm", // only with publish
    // Git remote mutations
    "git",  // only with push/tag subcommand — checked below
    // System mutation
    "rm",
    "mkfs",
    "systemctl",
    "kill",
    "killall",
    "reboot",
    "shutdown",
    // Notifications
    "mail",
    "sendmail",
    "slack",
    "notify-send",
];

/// Commands that are only impure with specific subcommands
const CONDITIONAL_IMPURE: &[(&str, &[&str])] = &[
    ("npm", &["publish", "unpublish", "deprecate", "dist-tag", "access"]),
    ("yarn", &["publish", "npm publish"]),
    ("pnpm", &["publish"]),
    ("git", &["push", "tag", "remote"]),
    ("docker", &["push", "rmi", "system prune", "volume rm", "network rm"]),
    ("kubectl", &["apply", "delete", "scale", "rollout", "drain", "cordon"]),
    ("terraform", &["apply", "destroy"]),
    ("helm", &["install", "upgrade", "delete", "rollback"]),
];

/// Script name patterns that suggest side effects
const IMPURE_NAME_PATTERNS: &[&str] = &[
    "deploy",
    "publish",
    "release",
    "push",
    "notify",
    "send",
    "migrate",
    "seed",
    "provision",
    "destroy",
    "teardown",
    "cleanup",
];

/// Detect whether a script body likely has side effects.
///
/// Returns true if impure patterns are found (should mark `side_effects = true`).
/// Returns false if the script appears pure (default: cached).
pub fn detect_impurity(body: &str, script_name: &str, commands: &[String]) -> bool {
    // 1. Check script name patterns
    let name_lower = script_name.to_lowercase();
    for pattern in IMPURE_NAME_PATTERNS {
        if name_lower.contains(pattern) {
            return true;
        }
    }

    // 2. Check for unconditionally impure commands
    for cmd in commands {
        let base = cmd.rsplit('/').next().unwrap_or(cmd);
        if IMPURE_COMMANDS.contains(&base) {
            // For conditionally impure commands, check subcommands
            if let Some((_, subcmds)) = CONDITIONAL_IMPURE.iter().find(|(c, _)| *c == base) {
                // Check if any impure subcommand appears after the command
                let body_lower = body.to_lowercase();
                if subcmds
                    .iter()
                    .any(|sub| body_lower.contains(&format!("{base} {sub}")))
                {
                    return true;
                }
                // If none of the impure subcommands matched, this use is safe
                // e.g., `npm test`, `npm run build`, `git status`
                continue;
            }

            // Unconditionally impure (no conditional entry)
            if !CONDITIONAL_IMPURE.iter().any(|(c, _)| *c == base) {
                return true;
            }
        }
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pure_script() {
        assert!(!detect_impurity("tsc && webpack", "build", &["tsc".into(), "webpack".into()]));
    }

    #[test]
    fn test_impure_curl() {
        assert!(detect_impurity("curl -X POST https://api.example.com", "notify", &["curl".into()]));
    }

    #[test]
    fn test_impure_name() {
        assert!(detect_impurity("echo done", "deploy-prod", &["echo".into()]));
    }

    #[test]
    fn test_npm_test_is_pure() {
        assert!(!detect_impurity("npm test", "test", &["npm".into()]));
    }

    #[test]
    fn test_npm_publish_is_impure() {
        assert!(detect_impurity("npm publish", "release", &["npm".into()]));
    }

    #[test]
    fn test_git_status_is_pure() {
        assert!(!detect_impurity("git status", "check", &["git".into()]));
    }

    #[test]
    fn test_git_push_is_impure() {
        assert!(detect_impurity("git push origin main", "push", &["git".into()]));
    }

    #[test]
    fn test_docker_build_is_pure() {
        assert!(!detect_impurity("docker build -t app .", "build", &["docker".into()]));
    }

    #[test]
    fn test_docker_push_is_impure() {
        assert!(detect_impurity("docker push myimage:latest", "push-image", &["docker".into()]));
    }

    #[test]
    fn test_kubectl_apply_is_impure() {
        assert!(detect_impurity("kubectl apply -f k8s/", "deploy", &["kubectl".into()]));
    }

    #[test]
    fn test_kubectl_get_is_pure() {
        assert!(!detect_impurity("kubectl get pods", "status", &["kubectl".into()]));
    }
}
