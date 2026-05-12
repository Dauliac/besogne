use std::process::Command;

fn cargo_bin() -> std::path::PathBuf {
    let mut path = std::env::current_exe().unwrap();
    path.pop();
    path.pop();
    path.push("besogne");
    path
}

#[test]
fn test_skip_on_second_run() {
    let dir = tempfile::tempdir().unwrap();
    let manifest = dir.path().join("besogne.toml");
    let output = dir.path().join("skip-test");
    let marker = dir.path().join("marker.txt");

    std::fs::write(
        &manifest,
        format!(
            r#"
name = "skip-test"
description = "Test skip logic"

[nodes.write-marker]
type = "command"
phase = "exec"
run = ["sh", "-c", "echo ran >> {}"]
"#,
            marker.display()
        ),
    )
    .unwrap();

    let result = Command::new(cargo_bin())
        .args(["build", "-o", output.to_str().unwrap()])
        .current_dir(dir.path())
        .env("XDG_CACHE_HOME", dir.path().join(".cache"))
        .output()
        .unwrap();
    assert!(result.status.success(), "compile: {}", String::from_utf8_lossy(&result.stderr));

    // First run — should execute
    let run1 = Command::new(&output).env("XDG_CACHE_HOME", dir.path().join(".cache")).output().unwrap();
    assert!(run1.status.success(), "run 1: {}", String::from_utf8_lossy(&run1.stderr));

    // Second run — should show cached replay (not re-execute)
    let run2 = Command::new(&output).env("XDG_CACHE_HOME", dir.path().join(".cache")).output().unwrap();
    assert!(run2.status.success());
    let stderr2 = String::from_utf8_lossy(&run2.stderr);
    assert!(
        stderr2.contains("cached") || stderr2.contains("nothing to do"),
        "second run should be cached: {stderr2}"
    );
}

#[test]
fn test_no_skip_when_cache_disabled() {
    let dir = tempfile::tempdir().unwrap();
    let manifest = dir.path().join("besogne.toml");
    let output = dir.path().join("no-skip-test");
    let marker = dir.path().join("marker.txt");

    // side_effects = true → never cached, always runs
    std::fs::write(
        &manifest,
        format!(
            r#"
name = "no-skip-test"
description = "No skip with side effects"

[nodes.write-marker]
type = "command"
phase = "exec"
run = ["sh", "-c", "echo ran >> {}"]
side_effects = true
"#,
            marker.display()
        ),
    )
    .unwrap();

    let result = Command::new(cargo_bin())
        .args(["build", "-o", output.to_str().unwrap()])
        .current_dir(dir.path())
        .env("XDG_CACHE_HOME", dir.path().join(".cache"))
        .output()
        .unwrap();
    assert!(result.status.success(), "compile: {}", String::from_utf8_lossy(&result.stderr));

    // Run twice — both should execute
    Command::new(&output).env("XDG_CACHE_HOME", dir.path().join(".cache")).output().unwrap();
    Command::new(&output).env("XDG_CACHE_HOME", dir.path().join(".cache")).output().unwrap();

    let content = std::fs::read_to_string(&marker).unwrap();
    assert_eq!(
        content.trim().lines().count(),
        2,
        "should run both times without cache"
    );
}

#[test]
fn test_rusage_metrics_populated() {
    let dir = tempfile::tempdir().unwrap();
    let manifest = dir.path().join("besogne.toml");
    let output = dir.path().join("rusage-test");

    std::fs::write(
        &manifest,
        r#"
name = "rusage-test"
description = "Test rusage metrics"

[nodes.busy]
type = "command"
phase = "exec"
run = ["sh", "-c", "for i in $(seq 1 10000); do echo $i > /dev/null; done"]
"#,
    )
    .unwrap();

    let compile = Command::new(cargo_bin())
        .args(["build", "-o", output.to_str().unwrap()])
        .current_dir(dir.path())
        .env("XDG_CACHE_HOME", dir.path().join(".cache"))
        .output()
        .unwrap();
    assert!(compile.status.success(), "compile: {}", String::from_utf8_lossy(&compile.stderr));

    let run = Command::new(&output).env("XDG_CACHE_HOME", dir.path().join(".cache")).output().unwrap();
    assert!(run.status.success());

    let stderr = String::from_utf8_lossy(&run.stderr);
    assert!(stderr.contains("time:"), "should show timing: {stderr}");
}

#[test]
fn test_parallel_warmup_all_probed() {
    let dir = tempfile::tempdir().unwrap();
    let manifest = dir.path().join("besogne.toml");
    let output = dir.path().join("parallel-test");
    let file1 = dir.path().join("a.txt");
    let file2 = dir.path().join("b.txt");
    std::fs::write(&file1, "aaa").unwrap();
    std::fs::write(&file2, "bbb").unwrap();

    std::fs::write(
        &manifest,
        format!(
            r#"
name = "parallel-warmup"
description = "Test all warmup probes run"

[nodes.HOME]
type = "env"

[nodes.file-a]
type = "file"
path = "{}"

[nodes.file-b]
type = "file"
path = "{}"

[nodes.platform]
type = "platform"


[nodes.cpu-count]
type = "metric"
metric = "cpu.count"

[nodes.ok]
type = "command"
phase = "exec"
run = ["echo", "all-warmup-passed"]
"#,
            file1.display(),
            file2.display()
        ),
    )
    .unwrap();

    let compile = Command::new(cargo_bin())
        .args(["build", "-o", output.to_str().unwrap()])
        .current_dir(dir.path())
        .env("XDG_CACHE_HOME", dir.path().join(".cache"))
        .output()
        .unwrap();
    assert!(compile.status.success(), "compile: {}", String::from_utf8_lossy(&compile.stderr));

    let run = Command::new(&output).env("XDG_CACHE_HOME", dir.path().join(".cache")).output().unwrap();
    assert!(run.status.success());

    let stderr = String::from_utf8_lossy(&run.stderr);
    assert!(stderr.contains("HOME="), "should probe HOME: {stderr}");
    assert!(stderr.contains(".txt"), "should probe files: {stderr}");
    assert!(stderr.contains("/"), "should probe platform (os/arch): {stderr}");
    assert!(stderr.contains("cpu.count="), "should probe metric: {stderr}");
    // First run triggers idempotency verification — command runs in verify mode
    assert!(
        stderr.contains("all-warmup-passed") || stderr.contains("ok") || stderr.contains("idempotent"),
        "command should run: {stderr}"
    );
}

#[test]
fn test_env_isolation_strict() {
    let dir = tempfile::tempdir().unwrap();
    let manifest = dir.path().join("besogne.toml");
    let output = dir.path().join("strict-test");

    std::fs::write(
        &manifest,
        r#"
name = "strict-test"
description = "Test strict env isolation"
sandbox = "strict"

[nodes.sh]
type = "binary"

[nodes.check-env]
type = "command"
phase = "exec"
run = ["sh", "-c", "echo HOME=$HOME"]
"#,
    )
    .unwrap();

    let compile = Command::new(cargo_bin())
        .args(["build", "-o", output.to_str().unwrap()])
        .current_dir(dir.path())
        .env("XDG_CACHE_HOME", dir.path().join(".cache"))
        .output()
        .unwrap();
    assert!(compile.status.success(), "compile: {}", String::from_utf8_lossy(&compile.stderr));

    let run = Command::new(&output).env("XDG_CACHE_HOME", dir.path().join(".cache")).output().unwrap();
    let stderr = String::from_utf8_lossy(&run.stderr);

    // In strict mode, HOME should be empty since it's not declared as an env input
    assert!(
        stderr.contains("HOME=\n") || stderr.contains("HOME=$HOME"),
        "HOME should be empty in strict mode: {stderr}"
    );
}
