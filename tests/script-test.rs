use std::env;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

fn unique_temp_dir() -> PathBuf {
    let mut p = env::temp_dir();
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    p.push(format!("triton-script-test-{}", nanos));
    fs::create_dir_all(&p).unwrap();
    p
}

fn write_file(path: &Path, contents: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    let mut f = fs::File::create(path).unwrap();
    f.write_all(contents.as_bytes()).unwrap();
}

#[cfg(windows)]
fn make_script(dir: &Path, name: &str) -> PathBuf {
    let mut p = dir.to_path_buf();
    p.push(name);
    // simple .cmd that prints all args
    write_file(
        &p,
        "@echo off\r\n\
         echo OK %*\r\n",
    );
    p
}

#[cfg(unix)]
fn make_script(dir: &Path, name: &str) -> PathBuf {
    let mut p = dir.to_path_buf();
    p.push(name);
    write_file(
        &p,
        "#!/bin/sh\n\
         echo OK \"$@\"\n",
    );
    let mut perm = fs::metadata(&p).unwrap().permissions();
    perm.set_mode(0o755);
    fs::set_permissions(&p, perm).unwrap();
    p
}

fn escape_json_path(p: &Path) -> String {
    let s = p.to_string_lossy().to_string();
    // escape backslashes for JSON on Windows
    s.replace('\\', "\\\\")
}

fn write_triton_json_with_scripts(dir: &Path, scripts_block: &str) {
    // minimal viable config + user scripts
    let json = format!(
        r#"{{
  "app_name": "demo",
  "triplet": "x64-windows",
  "generator": "Ninja",
  "cxx_std": "20",
  "deps": [],
  "components": {{}},
  "scripts": {{
    {scripts}
  }}
}}"#,
        scripts = scripts_block
    );
    write_file(&dir.join("triton.json"), &json);
}

fn run_triton_in(dir: &Path, args: &[&str]) -> Output {
    // Path to compiled bin provided by Cargo for integration tests
    let exe = env!("CARGO_BIN_EXE_triton");
    Command::new(exe)
        .current_dir(dir)
        .args(args)
        .output()
        .expect("failed to spawn triton")
}

#[test]
fn script_executes_direct_and_passes_args() {
    let td = unique_temp_dir();

    // create a tiny script file
    #[cfg(windows)]
    let script_rel = "echo_args.cmd";
    #[cfg(unix)]
    let script_rel = "echo_args.sh";

    let script_path = make_script(&td, script_rel);

    // absolute path script
    let abs = escape_json_path(&script_path);
    write_triton_json_with_scripts(&td, &format!(r#""say": "{}""#, abs));

    let out = run_triton_in(&td, &["say", "hello", "world"]);
    assert!(
        out.status.success(),
        "script failed: status={:?}, stdout={:?}, stderr={:?}",
        out.status.code(),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("OK hello world"),
        "unexpected stdout: {}",
        stdout
    );
}

#[test]
fn script_with_shell_operators_runs_via_shell() {
    let td = unique_temp_dir();

    #[cfg(windows)]
    let shell_line = r#"echo LEFT && echo RIGHT"#;
    #[cfg(unix)]
    let shell_line = r#"echo LEFT && echo RIGHT"#;

    write_triton_json_with_scripts(&td, &format!(r#""both": "{}""#, shell_line));

    let out = run_triton_in(&td, &["both"]);
    assert!(
        out.status.success(),
        "shell script failed: status={:?}, stdout={:?}, stderr={:?}",
        out.status.code(),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("LEFT"), "stdout missing LEFT: {}", stdout);
    assert!(stdout.contains("RIGHT"), "stdout missing RIGHT: {}", stdout);
}

#[test]
fn windows_dot_slash_path_is_normalized() {
    // Only meaningful on Windows
    #[cfg(windows)]
    {
        let td = unique_temp_dir();
        let script_path = make_script(&td, "script.cmd");

        // write script using POSIX-style ./ with forward slashes
        let rel = "./script.cmd";
        write_triton_json_with_scripts(&td, &format!(r#""dot": "{}""#, rel));

        let out = run_triton_in(&td, &["dot", "A", "B"]);
        assert!(
            out.status.success(),
            "dot-slash script failed: code={:?}, stdout={:?}, stderr={:?}",
            out.status.code(),
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            stdout.contains("OK A B"),
            "unexpected stdout for dot path: {} (script at {:?})",
            stdout,
            script_path
        );
    }
}

#[test]
fn unknown_script_is_reported() {
    let td = unique_temp_dir();
    // valid file with no scripts entry
    write_triton_json_with_scripts(&td, "");

    let out = run_triton_in(&td, &["nope"]);
    assert!(
        !out.status.success(),
        "expected failure for unknown script"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.to_ascii_lowercase().contains("unknown script") || stderr.to_ascii_lowercase().contains("no script"),
        "stderr should mention unknown script, got: {}",
        stderr
    );
}
