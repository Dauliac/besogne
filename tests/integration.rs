use std::process::Command;

fn cargo_bin() -> std::path::PathBuf {
    // Find the compiled binary
    let mut path = std::env::current_exe().unwrap();
    path.pop(); // remove test binary name
    path.pop(); // remove deps/
    path.push("besogne");
    path
}

/// Compile a manifest in the given dir with isolated cache
fn compile_in(dir: &std::path::Path, output: &std::path::Path) -> std::process::Output {
    Command::new(cargo_bin())
        .args(["build", "-o", output.to_str().unwrap()])
        .current_dir(dir)
        .env("XDG_CACHE_HOME", dir.join(".cache"))
        .output()
        .unwrap()
}

#[test]
fn test_compile_and_run_hello() {
    let dir = tempfile::tempdir().unwrap();
    let manifest = dir.path().join("besogne.toml");
    let output = dir.path().join("hello");

    std::fs::write(
        &manifest,
        r#"
name = "hello"
description = "Say hello"

[inputs.echo]
type = "binary"

[inputs.hello]
type = "command"
phase = "exec"
run = ["echo", "hello from besogne"]
"#,
    )
    .unwrap();

    // Compile
    let compile_result = compile_in(dir.path(), &output);

    assert!(
        compile_result.status.success(),
        "compile failed: {}",
        String::from_utf8_lossy(&compile_result.stderr)
    );
    assert!(output.exists(), "output binary not created");

    // Run
    let run_result = Command::new(&output).env("XDG_CACHE_HOME", dir.path().join(".cache")).output().unwrap();

    let stderr = String::from_utf8_lossy(&run_result.stderr);
    let stdout = String::from_utf8_lossy(&run_result.stdout);

    assert!(
        run_result.status.success(),
        "run failed (exit {}): stderr={stderr} stdout={stdout}",
        run_result.status.code().unwrap_or(-1)
    );
    assert!(stderr.contains("hello"), "expected 'hello' in output: {stderr}");
}

#[test]
fn test_compile_and_run_with_env() {
    let dir = tempfile::tempdir().unwrap();
    let manifest = dir.path().join("besogne.toml");
    let output = dir.path().join("env-test");

    std::fs::write(
        &manifest,
        r#"
name = "env-test"
description = "Test env input"

[inputs.HOME]
type = "env"

[inputs.show-home]
type = "command"
phase = "exec"
run = ["echo", "home-is-set"]
"#,
    )
    .unwrap();

    let compile_result = compile_in(dir.path(), &output);
    assert!(compile_result.status.success());

    let run_result = Command::new(&output).env("XDG_CACHE_HOME", dir.path().join(".cache")).output().unwrap();
    assert!(run_result.status.success());

    let stderr = String::from_utf8_lossy(&run_result.stderr);
    assert!(stderr.contains("HOME="), "should show HOME value: {stderr}");
    assert!(stderr.contains("home-is-set"));
}

#[test]
fn test_compile_and_run_missing_env_fails() {
    let dir = tempfile::tempdir().unwrap();
    let manifest = dir.path().join("besogne.toml");
    let output = dir.path().join("missing-env");

    std::fs::write(
        &manifest,
        r#"
name = "missing-env"
description = "Test missing env"

[inputs.BESOGNE_NONEXISTENT_REQUIRED_VAR_XYZ]
type = "env"

[inputs.noop]
type = "command"
phase = "exec"
run = ["echo", "should not run"]
"#,
    )
    .unwrap();

    let compile_result = compile_in(dir.path(), &output);
    assert!(compile_result.status.success());

    let run_result = Command::new(&output).env("XDG_CACHE_HOME", dir.path().join(".cache")).output().unwrap();
    assert!(!run_result.status.success(), "should fail on missing env");
    assert_eq!(run_result.status.code(), Some(2));

    let stderr = String::from_utf8_lossy(&run_result.stderr);
    assert!(stderr.contains("not set"), "expected 'not set' error: {stderr}");
    assert!(!stderr.contains("should not run"), "command should not have run");
}

#[test]
fn test_compile_and_run_file_input() {
    let dir = tempfile::tempdir().unwrap();
    let manifest = dir.path().join("besogne.toml");
    let output = dir.path().join("file-test");
    let test_file = dir.path().join("data.txt");
    std::fs::write(&test_file, "test data").unwrap();

    std::fs::write(
        &manifest,
        format!(
            r#"
name = "file-test"
description = "Test file input"

[inputs.data-file]
type = "file"
path = "{}"

[inputs.cat-it]
type = "command"
phase = "exec"
run = ["cat", "{}"]
"#,
            test_file.display(),
            test_file.display()
        ),
    )
    .unwrap();

    let compile_result = compile_in(dir.path(), &output);
    assert!(compile_result.status.success());

    let run_result = Command::new(&output).env("XDG_CACHE_HOME", dir.path().join(".cache")).output().unwrap();
    assert!(run_result.status.success());

    let stderr = String::from_utf8_lossy(&run_result.stderr);
    assert!(stderr.contains(".txt"));
    assert!(stderr.contains("test data"));
}

#[test]
fn test_compile_and_run_command_chain() {
    let dir = tempfile::tempdir().unwrap();
    let manifest = dir.path().join("besogne.toml");
    let output = dir.path().join("chain-test");

    std::fs::write(
        &manifest,
        r#"
name = "chain-test"
description = "Test command dependencies"

[inputs.first]
type = "command"
phase = "exec"
run = ["echo", "step-1"]

[inputs.second]
type = "command"
phase = "exec"
run = ["echo", "step-2"]
after = ["first"]
"#,
    )
    .unwrap();

    let compile_result = compile_in(dir.path(), &output);
    assert!(compile_result.status.success());

    let run_result = Command::new(&output).env("XDG_CACHE_HOME", dir.path().join(".cache")).output().unwrap();
    assert!(run_result.status.success());

    let stderr = String::from_utf8_lossy(&run_result.stderr);
    let step1_pos = stderr.find("step-1").expect("step-1 not found");
    let step2_pos = stderr.find("step-2").expect("step-2 not found");
    assert!(step1_pos < step2_pos, "step-1 should run before step-2");
}

#[test]
fn test_compile_and_run_failing_command() {
    let dir = tempfile::tempdir().unwrap();
    let manifest = dir.path().join("besogne.toml");
    let output = dir.path().join("fail-test");

    std::fs::write(
        &manifest,
        r#"
name = "fail-test"
description = "Test failing command"

[inputs.fail]
type = "command"
phase = "exec"
run = ["sh", "-c", "exit 42"]

[inputs.after]
type = "command"
phase = "exec"
run = ["echo", "should-not-run"]
after = ["fail"]
"#,
    )
    .unwrap();

    let compile_result = compile_in(dir.path(), &output);
    assert!(compile_result.status.success());

    let run_result = Command::new(&output).env("XDG_CACHE_HOME", dir.path().join(".cache")).output().unwrap();
    assert!(!run_result.status.success());

    let stderr = String::from_utf8_lossy(&run_result.stderr);
    assert!(stderr.contains("exit 42") || stderr.contains("FAILED"));
    assert!(!stderr.contains("should-not-run"), "dependent command should not run after failure");
}

#[test]
fn test_check_valid_manifest() {
    let dir = tempfile::tempdir().unwrap();
    let manifest = dir.path().join("besogne.toml");

    std::fs::write(
        &manifest,
        r#"
name = "check-test"
description = "Valid manifest"

[inputs.HOME]
type = "env"
"#,
    )
    .unwrap();

    let result = Command::new(cargo_bin())
        .args(["check"])
        .current_dir(dir.path())
        .env("XDG_CACHE_HOME", dir.path().join(".cache"))
        .output()
        .unwrap();

    assert!(result.status.success());
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(stderr.contains("is valid"));
}

#[test]
fn test_check_invalid_manifest() {
    let dir = tempfile::tempdir().unwrap();
    let manifest = dir.path().join("besogne.toml");

    std::fs::write(&manifest, r#"not_a_field = "a manifest""#).unwrap();

    let result = Command::new(cargo_bin())
        .args(["check"])
        .current_dir(dir.path())
        .env("XDG_CACHE_HOME", dir.path().join(".cache"))
        .output()
        .unwrap();

    assert!(!result.status.success());
}

#[test]
fn test_compile_command_missing_run_fails() {
    let dir = tempfile::tempdir().unwrap();
    let manifest = dir.path().join("besogne.toml");

    std::fs::write(
        &manifest,
        r#"
name = "bad"
description = "Command missing run field"

[inputs.broken]
type = "command"
phase = "exec"
"#,
    )
    .unwrap();

    let result = Command::new(cargo_bin())
        .args(["check"])
        .current_dir(dir.path())
        .env("XDG_CACHE_HOME", dir.path().join(".cache"))
        .output()
        .unwrap();

    assert!(!result.status.success());
}

#[test]
fn test_platform_probe_runs() {
    let dir = tempfile::tempdir().unwrap();
    let manifest = dir.path().join("besogne.toml");
    let output = dir.path().join("platform-test");

    std::fs::write(
        &manifest,
        format!(
            r#"
name = "platform-test"
description = "Test platform input"

[inputs.platform]
type = "platform"
os = "{}"
arch = "{}"

[inputs.ok]
type = "command"
phase = "exec"
run = ["echo", "platform-ok"]
"#,
            std::env::consts::OS,
            std::env::consts::ARCH
        ),
    )
    .unwrap();

    let compile_result = compile_in(dir.path(), &output);
    assert!(compile_result.status.success());

    let run_result = Command::new(&output).env("XDG_CACHE_HOME", dir.path().join(".cache")).output().unwrap();
    assert!(run_result.status.success());

    let stderr = String::from_utf8_lossy(&run_result.stderr);
    assert!(stderr.contains("platform-ok"));
}
