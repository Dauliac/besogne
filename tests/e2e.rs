use std::path::{Path, PathBuf};
use std::process::Command;

fn cargo_bin() -> PathBuf {
    let mut path = std::env::current_exe().unwrap();
    path.pop();
    path.pop();
    path.push("besogne");
    path
}

fn e2e_case(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/e2e")
        .join(name)
}

/// Copy an e2e case to a temp dir so tests don't pollute the repo
fn setup_case(name: &str) -> tempfile::TempDir {
    let src = e2e_case(name);
    let dir = tempfile::tempdir().unwrap();
    copy_dir(&src, dir.path());
    dir
}

/// Compile besogne.toml in the given workdir via auto-discovery.
/// Sets XDG_CACHE_HOME to isolate compile cache between tests.
fn compile_in(workdir: &Path) -> std::process::Output {
    let output_bin = workdir.join("besogne-out");
    Command::new(cargo_bin())
        .args(["build", "-o", output_bin.to_str().unwrap()])
        .current_dir(workdir)
        .env("XDG_CACHE_HOME", workdir.join(".cache"))
        .output()
        .unwrap()
}

/// Run the compiled binary in the given workdir
/// Sets XDG_CACHE_HOME to the workdir to isolate cache between tests
fn run_in(workdir: &Path) -> std::process::Output {
    Command::new(workdir.join("besogne-out"))
        .current_dir(workdir)
        .env("XDG_CACHE_HOME", workdir.join(".cache"))
        .output()
        .unwrap()
}

fn run_in_with_args(workdir: &Path, args: &[&str]) -> std::process::Output {
    Command::new(workdir.join("besogne-out"))
        .args(args)
        .current_dir(workdir)
        .env("XDG_CACHE_HOME", workdir.join(".cache"))
        .output()
        .unwrap()
}

fn run_in_with_env(workdir: &Path, env: &[(&str, &str)]) -> std::process::Output {
    let mut cmd = Command::new(workdir.join("besogne-out"));
    cmd.current_dir(workdir);
    cmd.env("XDG_CACHE_HOME", workdir.join(".cache"));
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd.output().unwrap()
}

fn stderr(out: &std::process::Output) -> String {
    String::from_utf8_lossy(&out.stderr).to_string()
}

fn has_tool(name: &str) -> bool {
    Command::new(name)
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn has_docker() -> bool {
    Command::new("docker")
        .arg("info")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn copy_dir(src: &Path, dst: &Path) {
    for entry in std::fs::read_dir(src).unwrap() {
        let entry = entry.unwrap();
        let dest = dst.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            std::fs::create_dir_all(&dest).unwrap();
            copy_dir(&entry.path(), &dest);
        } else {
            std::fs::copy(entry.path(), &dest).unwrap();
            // Preserve execute permission
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let src_perms = std::fs::metadata(entry.path()).unwrap().permissions();
                if src_perms.mode() & 0o111 != 0 {
                    let mut dst_perms = std::fs::metadata(&dest).unwrap().permissions();
                    dst_perms.set_mode(src_perms.mode());
                    std::fs::set_permissions(&dest, dst_perms).unwrap();
                }
            }
        }
    }
}

// ─── minimal ────────────────────────────────────────────────────

#[test]
fn e2e_minimal() {
    let dir = setup_case("minimal");
    let c = compile_in(dir.path());
    assert!(c.status.success(), "compile: {}", stderr(&c));
    let r = run_in(dir.path());
    assert!(r.status.success(), "run: {}", stderr(&r));
    assert!(stderr(&r).contains("hello from besogne"));
}

// ─── npm-install ────────────────────────────────────────────────

#[test]
fn e2e_npm_install() {
    if !has_tool("npm") {
        eprintln!("SKIP: npm not available");
        return;
    }
    let dir = setup_case("npm-install");
    let c = compile_in(dir.path());
    assert!(c.status.success(), "compile: {}", stderr(&c));

    let r = run_in(dir.path());
    assert!(r.status.success(), "run: {}", stderr(&r));
    assert!(dir.path().join("node_modules").exists(), "node_modules not created");
}

#[test]
fn e2e_npm_install_skip_on_second_run() {
    if !has_tool("npm") {
        eprintln!("SKIP: npm not available");
        return;
    }
    let dir = setup_case("npm-install");
    let c = compile_in(dir.path());
    assert!(c.status.success());

    let r1 = run_in(dir.path());
    assert!(r1.status.success());

    let r2 = run_in(dir.path());
    assert!(r2.status.success());
    assert!(stderr(&r2).contains("\ncached "), "2nd run should skip: {}", stderr(&r2));
}

// ─── npm-ci-pipeline ────────────────────────────────────────────

#[test]
fn e2e_npm_ci_pipeline() {
    if !has_tool("npm") {
        eprintln!("SKIP: npm not available");
        return;
    }
    let dir = setup_case("npm-ci-pipeline");
    let c = compile_in(dir.path());
    assert!(c.status.success(), "compile: {}", stderr(&c));

    let r = run_in(dir.path());
    let err = stderr(&r);
    assert!(r.status.success(), "run: {err}");

    // Verify DAG order: install before lint/test, both before build
    let install_pos = err.find("install:").expect("install not found");
    let build_pos = err.find("build:").expect("build not found");
    assert!(install_pos < build_pos, "install should run before build");
}

// ─── go-test ────────────────────────────────────────────────────

#[test]
fn e2e_go_test() {
    if !has_tool("go") {
        eprintln!("SKIP: go not available");
        return;
    }
    let dir = setup_case("go-test");
    let c = compile_in(dir.path());
    assert!(c.status.success(), "compile: {}", stderr(&c));

    let r = run_in(dir.path());
    let err = stderr(&r);
    assert!(r.status.success(), "run: {err}");
    assert!(err.contains("PASS"), "go tests should pass: {err}");
}

// ─── docker-alpine ──────────────────────────────────────────────

#[test]
fn e2e_docker_alpine() {
    if !has_docker() {
        eprintln!("SKIP: docker not available");
        return;
    }
    let dir = setup_case("docker-alpine");
    let c = compile_in(dir.path());
    assert!(c.status.success(), "compile: {}", stderr(&c));

    let r = run_in(dir.path());
    let err = stderr(&r);
    assert!(r.status.success(), "run: {err}");
    assert!(err.contains("hello from alpine"), "docker output: {err}");
}

// ─── script-build ───────────────────────────────────────────────

#[test]
fn e2e_script_build() {
    let dir = setup_case("script-build");
    let c = compile_in(dir.path());
    assert!(c.status.success(), "compile: {}", stderr(&c));

    let r = run_in(dir.path());
    let err = stderr(&r);
    assert!(r.status.success(), "run: {err}");

    assert!(dir.path().join("dist/result.json").exists(), "result.json missing");
    assert!(dir.path().join("dist/build.log").exists(), "build.log missing");

    let result = std::fs::read_to_string(dir.path().join("dist/result.json")).unwrap();
    assert!(result.contains("\"status\":\"ok\""), "bad result.json: {result}");
    assert!(result.contains("myapp"), "project name missing: {result}");
}

// ─── multi-probe ────────────────────────────────────────────────

#[test]
fn e2e_multi_probe() {
    let dir = setup_case("multi-probe");
    let c = compile_in(dir.path());
    assert!(c.status.success(), "compile: {}", stderr(&c));

    let r = run_in(dir.path());
    let err = stderr(&r);
    assert!(r.status.success(), "run: {err}");

    for label in &["HOME=", "USER=", "CUSTOM=", "data.txt", "sh",
                     "/", "cpu.count=", "localhost"] {
        assert!(err.contains(label), "missing probe {label}: {err}");
    }
    assert!(err.contains("all-probes-passed"));
}

// ─── cache-skip ─────────────────────────────────────────────────

#[test]
fn e2e_cache_skip() {
    let dir = setup_case("cache-skip");
    let c = compile_in(dir.path());
    assert!(c.status.success(), "compile: {}", stderr(&c));

    let r1 = run_in(dir.path());
    let err1 = stderr(&r1);
    assert!(r1.status.success(), "run 1 failed: {err1}");
    assert!(
        !err1.contains("\ncached "),
        "run 1 should NOT skip: {err1}"
    );
    let marker = dir.path().join("marker.txt");
    assert!(marker.exists(), "marker.txt not created. stderr: {err1}");
    assert_eq!(
        std::fs::read_to_string(&marker).unwrap().lines().count(),
        1, "run 1: should execute once"
    );

    let r2 = run_in(dir.path());
    assert!(r2.status.success());
    assert!(stderr(&r2).contains("\ncached "), "run 2 should skip: {}", stderr(&r2));
    assert_eq!(
        std::fs::read_to_string(dir.path().join("marker.txt")).unwrap().lines().count(),
        1, "run 2: marker should still be 1 line"
    );
}

// ─── cache-invalidate ───────────────────────────────────────────

#[test]
fn e2e_cache_invalidate() {
    let dir = setup_case("cache-invalidate");
    let c = compile_in(dir.path());
    assert!(c.status.success());

    let r1 = run_in(dir.path());
    assert!(r1.status.success());

    // Change the input file
    std::fs::write(dir.path().join("input.txt"), "v2-changed\n").unwrap();

    let r2 = run_in(dir.path());
    assert!(r2.status.success());
    assert!(!stderr(&r2).contains("\ncached "), "should NOT skip: {}", stderr(&r2));
    assert_eq!(
        std::fs::read_to_string(dir.path().join("marker.txt")).unwrap().lines().count(),
        2, "should have run twice"
    );
}

// ─── isolation-strict ───────────────────────────────────────────

#[test]
fn e2e_isolation_strict() {
    let dir = setup_case("isolation-strict");
    let c = compile_in(dir.path());
    assert!(c.status.success(), "compile: {}", stderr(&c));

    let r = run_in(dir.path());
    let err = stderr(&r);
    assert!(r.status.success(), "run: {err}");
    assert!(err.contains("ALLOWED=visible"), "ALLOWED_VAR missing: {err}");
    // HOME should be empty in strict mode
    assert!(
        err.contains("HOME=\n") || err.contains("HOME= ") || err.contains("HOME=$HOME"),
        "HOME should be empty in strict: {err}"
    );
}

// ─── command-chain ──────────────────────────────────────────────

#[test]
fn e2e_command_chain() {
    let dir = setup_case("command-chain");
    let c = compile_in(dir.path());
    assert!(c.status.success(), "compile: {}", stderr(&c));

    let r = run_in(dir.path());
    assert!(r.status.success(), "run: {}", stderr(&r));

    let order = std::fs::read_to_string(dir.path().join("order.txt")).unwrap();
    let lines: Vec<&str> = order.lines().collect();
    assert_eq!(lines.len(), 4, "expected 4 steps: {order}");
    assert_eq!(lines[0], "step-1");
    assert_eq!(lines[3], "step-3");
    // 2a/2b can be either order (parallel tier)
    assert!(
        lines[1..3].contains(&"step-2a") && lines[1..3].contains(&"step-2b"),
        "step-2a/2b in middle: {order}"
    );
}

// ─── command-failure ────────────────────────────────────────────

#[test]
fn e2e_command_failure_stops_deps() {
    let dir = setup_case("command-failure");
    let c = compile_in(dir.path());
    assert!(c.status.success());

    let r = run_in(dir.path());
    assert!(!r.status.success(), "should fail");
    assert!(
        !dir.path().join("leaked.txt").exists(),
        "dependent command ran despite failure"
    );
}

// ─── pipe-exec ──────────────────────────────────────────────────

#[test]
fn e2e_pipe_exec() {
    let dir = setup_case("pipe-exec");
    let c = compile_in(dir.path());
    assert!(c.status.success(), "compile: {}", stderr(&c));

    let r = run_in(dir.path());
    let err = stderr(&r);
    assert!(r.status.success(), "run: {err}");
    assert!(err.contains("HELLO WORLD"), "pipe should uppercase: {err}");
}

// ─── env-computed ───────────────────────────────────────────────

#[test]
fn e2e_env_computed() {
    let dir = setup_case("env-computed");
    let c = compile_in(dir.path());
    assert!(c.status.success(), "compile: {}", stderr(&c));

    let r = run_in(dir.path());
    let err = stderr(&r);
    assert!(r.status.success(), "run: {err}");
    assert!(err.contains("MY_VAR=/custom/path"), "computed var: {err}");
    assert!(err.contains("ANOTHER=hello-world"), "another var: {err}");
}

// ─── env-missing ────────────────────────────────────────────────

#[test]
fn e2e_env_missing_fails() {
    let dir = setup_case("env-missing");
    let c = compile_in(dir.path());
    assert!(c.status.success());

    let r = run_in(dir.path());
    assert!(!r.status.success());
    assert_eq!(r.status.code(), Some(2));
    assert!(!stderr(&r).contains("THIS_SHOULD_NOT_APPEAR"));
}

// ─── env-secret ─────────────────────────────────────────────────

#[test]
fn e2e_env_secret_not_leaked() {
    let dir = setup_case("env-secret");
    let c = compile_in(dir.path());
    assert!(c.status.success());

    let r = run_in_with_env(dir.path(), &[("BESOGNE_E2E_SECRET", "super-secret-value")]);
    let err = stderr(&r);
    assert!(r.status.success(), "run: {err}");
    assert!(!err.contains("super-secret-value"), "secret leaked: {err}");
}

// ─── verify-idempotent ──────────────────────────────────────────

#[test]
fn e2e_verify_idempotent_passes() {
    let dir = setup_case("verify-idempotent");
    let c = compile_in(dir.path());
    assert!(c.status.success(), "compile: {}", stderr(&c));

    let r = run_in_with_args(dir.path(), &["--verify"]);
    let err = stderr(&r);
    assert!(r.status.success(), "verify should pass for idempotent command: {err}");
    assert!(
        err.contains("verification PASSED") || err.contains("idempotent"),
        "should report idempotent: {err}"
    );
}

// ─── verify-non-idempotent ──────────────────────────────────────

#[test]
fn e2e_verify_non_idempotent_fails() {
    let dir = setup_case("verify-non-idempotent");
    let c = compile_in(dir.path());
    assert!(c.status.success(), "compile: {}", stderr(&c));

    let r = run_in_with_args(dir.path(), &["--verify"]);
    let err = stderr(&r);
    assert!(!r.status.success(), "verify should FAIL for non-idempotent: {err}");
    assert!(
        err.contains("NOT IDEMPOTENT") || err.contains("verification FAILED"),
        "should report non-idempotent: {err}"
    );
}

// ─── nested-scripts ────────────────────────────────────────────

#[test]
fn e2e_nested_scripts() {
    let dir = setup_case("nested-scripts");
    let c = compile_in(dir.path());
    assert!(c.status.success(), "compile: {}", stderr(&c));

    let r = run_in(dir.path());
    let err = stderr(&r);
    assert!(r.status.success(), "run: {err}");

    // Verify all result files were created by the nested scripts
    let results = dir.path().join("results");

    // Level 1 output
    let l1 = std::fs::read_to_string(results.join("level1.txt")).unwrap();
    assert!(l1.contains("level1: wrote"), "level1 should write: {l1}");

    // Level 2 output (includes subshell)
    let l2 = std::fs::read_to_string(results.join("level2.txt")).unwrap();
    assert!(l2.contains("level2: stamp="), "level2 stamp: {l2}");
    assert!(l2.contains("subshell: end"), "subshell should complete: {l2}");

    // Level 3 output (arithmetic, trap, urandom)
    let l3 = std::fs::read_to_string(results.join("level3.txt")).unwrap();
    assert!(l3.contains("level3: sum=15"), "level3 arithmetic: {l3}");
    assert!(l3.contains("level3: random="), "level3 urandom: {l3}");
    assert!(l3.contains("level3: trap fired"), "level3 trap: {l3}");

    // Forked processes (3 parallel forks from level3)
    let forks = std::fs::read_to_string(results.join("forks.txt")).unwrap();
    assert_eq!(forks.matches("done").count(), 3, "3 forks should complete: {forks}");

    // Background process from level1
    let bg = std::fs::read_to_string(results.join("background.txt")).unwrap();
    assert!(bg.contains("background: done"), "background job: {bg}");

    // Pipe output (tr a-z A-Z)
    let pipe = std::fs::read_to_string(results.join("pipe.txt")).unwrap();
    assert!(pipe.contains("HELLO-FROM-PIPE"), "pipe transform: {pipe}");

    // Heredoc from level2
    let heredoc = std::fs::read_to_string(results.join("heredoc.txt")).unwrap();
    assert!(heredoc.contains("generated by level2"), "heredoc: {heredoc}");
    assert!(heredoc.contains("nested=true"), "heredoc nested: {heredoc}");

    // Verify stderr shows all three levels executing
    assert!(err.contains("level1:"), "stderr should show level1: {err}");
    assert!(err.contains("level2:"), "stderr should show level2: {err}");
    assert!(err.contains("level3:"), "stderr should show level3: {err}");
}

// ─── adopt-npm ──────────────────────────────────────────────────

#[test]
fn e2e_adopt_npm_generates_manifest() {
    let dir = setup_case("adopt-npm");
    let pkg_path = dir.path().join("package.json");
    let manifest_path = dir.path().join("besogne.toml");

    // Run adopt
    let output = Command::new(cargo_bin())
        .args(["adopt", "-s", pkg_path.to_str().unwrap()])
        .current_dir(dir.path())
        .env("XDG_CACHE_HOME", dir.path().join(".cache"))
        .output()
        .unwrap();

    let err = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "adopt failed: {err}");

    // Check besogne.toml was generated
    assert!(manifest_path.exists(), "besogne.toml not created");
    let manifest = std::fs::read_to_string(&manifest_path).unwrap();

    // Check binary inputs
    assert!(manifest.contains("[inputs.echo]") || manifest.contains("type = \"binary\""),
        "should declare binary inputs: {manifest}");

    // Check command inputs with correct ordering
    assert!(manifest.contains("[inputs.build]"), "should have build command: {manifest}");
    assert!(manifest.contains("[inputs.test]"), "should have test command: {manifest}");
    assert!(manifest.contains("[inputs.deploy]"), "should have deploy command: {manifest}");

    // Check side_effects on deploy (curl detected)
    assert!(manifest.contains("side_effects = true"), "deploy should have side_effects: {manifest}");

    // Check lifecycle ordering: build after prebuild
    assert!(manifest.contains("after = [\"prebuild\"]"), "build should depend on prebuild: {manifest}");

    // Check backup was created
    let backup = dir.path().join("package.besogne.old.json");
    assert!(backup.exists(), "backup not created");

    // Check package.json was rewritten
    let rewritten = std::fs::read_to_string(&pkg_path).unwrap();
    let pkg: serde_json::Value = serde_json::from_str(&rewritten).unwrap();
    assert_eq!(pkg["scripts"]["build"], "besogne run build", "scripts should use besogne run");
    assert_eq!(pkg["scripts"]["test"], "besogne run test");

    // Check backup preserves original content
    let backup_content = std::fs::read_to_string(&backup).unwrap();
    let backup_pkg: serde_json::Value = serde_json::from_str(&backup_content).unwrap();
    assert_eq!(backup_pkg["scripts"]["build"], "echo compile", "backup should have original scripts");
}

#[test]
fn e2e_adopt_npm_dry_run() {
    let dir = setup_case("adopt-npm");
    let pkg_path = dir.path().join("package.json");
    let manifest_path = dir.path().join("besogne.toml");

    // Dry run should not modify anything
    let output = Command::new(cargo_bin())
        .args(["adopt", "-s", pkg_path.to_str().unwrap(), "--dry-run"])
        .current_dir(dir.path())
        .env("XDG_CACHE_HOME", dir.path().join(".cache"))
        .output()
        .unwrap();

    assert!(output.status.success(), "dry run failed: {}", String::from_utf8_lossy(&output.stderr));
    assert!(!manifest_path.exists(), "dry run should not create besogne.toml");

    // package.json should be unchanged
    let content = std::fs::read_to_string(&pkg_path).unwrap();
    let pkg: serde_json::Value = serde_json::from_str(&content).unwrap();
    assert_eq!(pkg["scripts"]["build"], "echo compile", "dry run should not modify package.json");
}

