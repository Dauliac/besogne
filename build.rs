use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let src = "src/tracer/besogne_preload.c";

    println!("cargo:rerun-if-changed={src}");

    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();

    let (lib_name, extra_flags): (&str, Vec<&str>) = match target_os.as_str() {
        "macos" => ("libbesogne_preload.dylib", vec!["-dynamiclib"]),
        _ => ("libbesogne_preload.so", vec!["-shared", "-fPIC"]),
    };

    let output = out_dir.join(lib_name);

    // Use system compiler with clean env to produce a .so linked against
    // system glibc (not Nix glibc). Required because the interposer gets
    // LD_PRELOAD'd into arbitrary binaries.
    let system_cc = if target_os == "linux" && std::path::Path::new("/usr/bin/gcc").exists() {
        "/usr/bin/gcc"
    } else if target_os == "linux" && std::path::Path::new("/usr/bin/cc").exists() {
        "/usr/bin/cc"
    } else {
        "cc"
    };

    let mut cmd = Command::new(system_cc);
    cmd.args([src, "-o", output.to_str().unwrap()])
        .args(&extra_flags)
        .args(["-O2", "-Wall", "-Wextra"]);

    if target_os == "linux" {
        // Clear Nix-injected environment for system glibc linkage
        cmd.env_clear();
        cmd.env("PATH", "/usr/bin:/bin");
        cmd.arg("-ldl");
    }

    let status = cmd.status();

    match status {
        Ok(s) if s.success() => {
            println!("cargo:rustc-env=BESOGNE_PRELOAD_LIB={}", output.display());
        }
        Ok(s) => {
            eprintln!("warning: besogne_preload.c compilation failed (exit {}), preload tracking disabled", s);
            println!("cargo:rustc-env=BESOGNE_PRELOAD_LIB=");
        }
        Err(e) => {
            eprintln!("warning: system cc not found ({e}), preload tracking disabled");
            println!("cargo:rustc-env=BESOGNE_PRELOAD_LIB=");
        }
    }

    // Keep old envtrack var for backward compat during transition
    println!("cargo:rustc-env=BESOGNE_ENVTRACK_LIB=");
}
