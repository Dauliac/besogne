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
    let components_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("components");
    Command::new(cargo_bin())
        .args(["build", "-o", output_bin.to_str().unwrap()])
        .current_dir(workdir)
        .env("XDG_CACHE_HOME", workdir.join(".cache"))
        .env("BESOGNE_COMPONENTS_DIR", components_dir)
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

    // First run — execute
    let r1 = run_in(dir.path());
    assert!(r1.status.success(), "run 1: {}", stderr(&r1));

    // Second run should skip
    let r2 = run_in(dir.path());
    assert!(r2.status.success());
    let err2 = stderr(&r2);
    assert!(
        err2.contains("cached") || err2.contains("skip"),
        "2nd run should skip: {err2}"
    );
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

    // First run — executes + idempotency verification (runs command twice)
    let r1 = run_in(dir.path());
    let err1 = stderr(&r1);
    assert!(r1.status.success(), "run 1 failed: {err1}");
    let marker = dir.path().join("marker.txt");
    assert!(marker.exists(), "marker.txt not created. stderr: {err1}");
    let count1 = std::fs::read_to_string(&marker).unwrap().lines().count();
    assert_eq!(count1, 2, "run 1: should execute twice (verification)");

    // Second run — should skip (cache populated)
    let r2 = run_in(dir.path());
    assert!(r2.status.success());
    let err2 = stderr(&r2);
    assert!(
        err2.contains("cached") || err2.contains("nothing to do"),
        "run 2 should be cached: {err2}"
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("marker.txt")).unwrap().lines().count(),
        count1, "run 2: marker should not grow (cached)"
    );
}

// ─── cache-invalidate ───────────────────────────────────────────

#[test]
fn e2e_cache_invalidate() {
    let dir = setup_case("cache-invalidate");
    let c = compile_in(dir.path());
    assert!(c.status.success());

    // First run: exec + verification double-run = 2 lines
    let r1 = run_in(dir.path());
    assert!(r1.status.success());

    // Change the input file
    std::fs::write(dir.path().join("input.txt"), "v2-changed\n").unwrap();

    // Second run: exec only (already verified) = +1 line
    let r2 = run_in(dir.path());
    assert!(r2.status.success());
    assert!(!stderr(&r2).contains("skip"), "should NOT skip after file change: {}", stderr(&r2));
    assert_eq!(
        std::fs::read_to_string(dir.path().join("marker.txt")).unwrap().lines().count(),
        3, "run1(2 verify) + run2(1 exec) = 3"
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
    // 4 commands x 2 (verification re-run) = 8 lines on first run
    assert_eq!(lines.len(), 8, "expected 8 steps (4 commands x 2 verify): {order}");
    // First execution: step-1, 2a/2b, step-3
    assert_eq!(lines[0], "step-1");
    // Dedup to check all steps were present
    let unique: std::collections::HashSet<&&str> = lines.iter().collect();
    assert!(unique.contains(&"step-1"), "missing step-1: {order}");
    assert!(unique.contains(&"step-2a"), "missing step-2a: {order}");
    assert!(unique.contains(&"step-2b"), "missing step-2b: {order}");
    assert!(unique.contains(&"step-3"), "missing step-3: {order}");
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
    assert!(manifest.contains("[nodes.echo]") || manifest.contains("type = \"binary\""),
        "should declare binary nodes: {manifest}");

    // Check command inputs with correct ordering
    assert!(manifest.contains("[nodes.build]"), "should have build command: {manifest}");
    assert!(manifest.contains("[nodes.test]"), "should have test command: {manifest}");
    assert!(manifest.contains("[nodes.deploy]"), "should have deploy command: {manifest}");

    // Check side_effects on deploy (curl detected)
    assert!(manifest.contains("side_effects = true"), "deploy should have side_effects: {manifest}");

    // Check lifecycle ordering: build depends on prebuild
    assert!(manifest.contains("parents = [\"prebuild\"]"), "build should depend on prebuild: {manifest}");

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

// ─── compile-error-missing-binary ────────────────────────────────

#[test]
fn e2e_compile_error_missing_binary() {
    let dir = setup_case("compile-error-missing-binary");
    let c = compile_in(dir.path());

    // Should FAIL to compile
    assert!(!c.status.success(), "should fail to compile");

    let err = stderr(&c);

    // Should mention both missing binaries
    assert!(
        err.contains("nonexistent-tool"),
        "error should mention 'nonexistent-tool': {err}"
    );
    assert!(
        err.contains("this-binary-absolutely-does-not-exist-xyz-123"),
        "error should mention the other binary: {err}"
    );

    // Should have Rust-style error formatting
    assert!(
        err.contains("-->") || err.contains("error"),
        "should have structured error format: {err}"
    );

    // Should have hints
    assert!(
        err.contains("hint") || err.contains("PATH"),
        "should have actionable hint: {err}"
    );

    // Binary should NOT be created
    assert!(
        !dir.path().join("besogne-out").exists(),
        "binary should not be created on compile error"
    );
}

// ─── nested-scripts ─────────────────────────────────────────────

// nested-scripts: tested manually (tests/e2e/nested-scripts/)

// ─── go-testcontainers ──────────────────────────────────────────

#[test]
fn e2e_go_testcontainers() {
    if !has_tool("go") {
        eprintln!("SKIP: go not available");
        return;
    }
    if !has_docker() {
        eprintln!("SKIP: docker not available");
        return;
    }
    let dir = setup_case("go-testcontainers");
    let c = compile_in(dir.path());
    assert!(c.status.success(), "compile: {}", stderr(&c));

    let r = run_in(dir.path());
    let err = stderr(&r);
    assert!(r.status.success(), "run: {err}");

    // Go test output should show PASS
    assert!(err.contains("PASS"), "go tests should pass: {err}");

    // Process tree should be visible (testcontainers spawns docker processes)
    assert!(err.contains("process tree"), "should show process tree: {err}");

    // JSON output should include container metadata from Docker API
    let r_json = run_in_with_args(dir.path(), &["--log-format", "json"]);
    assert!(r_json.status.success(), "json run: {}", stderr(&r_json));
    let stdout = String::from_utf8_lossy(&r_json.stdout);
    // Check that process tree data is present in JSON output
    let has_tree = stdout.lines().any(|line| line.contains("process_tree"));
    assert!(has_tree, "JSON output should include process tree: {stdout}");
}

// ─── source-dotenv ──────────────────────────────────────────────

#[test]
fn e2e_source_dotenv() {
    let dir = setup_case("source-dotenv");
    let c = compile_in(dir.path());
    assert!(c.status.success(), "compile: {}", stderr(&c));

    let r = run_in(dir.path());
    assert!(r.status.success(), "run: {}", stderr(&r));
    let err = stderr(&r);
    assert!(err.contains("DB=postgres://localhost:5432/mydb"), "source env vars should be injected: {err}");
    assert!(err.contains("NAME=besogne-test"), "source env vars should be injected: {err}");
}

// ─── source-json ────────────────────────────────────────────────

#[test]
fn e2e_source_json() {
    let dir = setup_case("source-json");
    let c = compile_in(dir.path());
    assert!(c.status.success(), "compile: {}", stderr(&c));

    let r = run_in(dir.path());
    assert!(r.status.success(), "run: {}", stderr(&r));
    let err = stderr(&r);
    assert!(err.contains("GOPATH=/home/user/go"), "source env vars should be injected: {err}");
    assert!(err.contains("CGO=0"), "source env vars should be injected: {err}");
}

