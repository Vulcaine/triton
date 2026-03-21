use anyhow::{anyhow, bail, Context, Result};
use std::collections::HashMap;
use std::env;
use std::path::Path;
use std::process::Command;

use crate::models::TritonRoot;
use crate::util;

#[cfg(windows)]
fn is_shelly(s: &str) -> bool {
    // cmd operators that require running through `cmd.exe /C`
    let ops = ["&&", "||", "|", ">", "<", "&", "(", ")", "%"];
    ops.iter().any(|op| s.contains(op))
}

#[cfg(not(windows))]
fn is_shelly(s: &str) -> bool {
    // sh operators that require `sh -c`
    let ops = ["&&", "||", "|", ";", ">", "<", "$(", "`"];
    ops.iter().any(|op| s.contains(op))
}

#[cfg(windows)]
fn shell_escape(arg: &str) -> String {
    // minimal quoting for cmd.exe
    if arg.is_empty() || arg.contains(char::is_whitespace) || arg.contains('"') {
        let mut out = String::from("\"");
        for ch in arg.chars() {
            if ch == '"' {
                out.push_str(r#"\""#);
            } else {
                out.push(ch);
            }
        }
        out.push('"');
        out
    } else {
        arg.to_string()
    }
}

#[cfg(not(windows))]
fn shell_escape(arg: &str) -> String {
    // minimal POSIX quoting
    if arg.is_empty() || arg.contains(|c: char| c.is_whitespace() || "'\"\\$`".contains(c)) {
        format!("'{}'", arg.replace('\'', r#"'\''"#))
    } else {
        arg.to_string()
    }
}

fn load_scripts(root_dir: &Path) -> Result<HashMap<String, String>> {
    let root: TritonRoot = util::read_json(root_dir.join("triton.json"))?;
    Ok(root.scripts)
}

fn looks_like_path(s: &str) -> bool {
    s.contains('/') || s.contains('\\') || s.ends_with(".exe") || s.ends_with(".cmd") || s.ends_with(".bat") || s.ends_with(".ps1")
}

/// Split a script string into (program, arguments).
/// The first whitespace-delimited token is the program; the rest are arguments.
/// This handles simple commands like `dotnet build path/to/project`.
fn split_command(script: &str) -> (&str, Vec<&str>) {
    let trimmed = script.trim();
    let mut parts = trimmed.split_whitespace();
    let program = parts.next().unwrap_or(trimmed);
    let script_args: Vec<&str> = parts.collect();
    (program, script_args)
}

#[cfg(windows)]
fn normalize_windows_path(root_dir: &Path, s: &str) -> String {
    // Handle leading ./ or .\ and normalize to absolute path
    let fixed = if let Some(rest) = s.strip_prefix("./") {
        root_dir.join(rest.replace('/', r"\")).to_string_lossy().to_string()
    } else if let Some(rest) = s.strip_prefix(".\\") {
        root_dir.join(rest).to_string_lossy().to_string()
    } else {
        s.replace('/', r"\")
    };
    fixed
}

#[cfg(windows)]
fn path_join_has_exe(dir: &str, exe: &str) -> Option<String> {
    use std::path::PathBuf;
    let mut p = PathBuf::from(dir);
    p.push(exe);
    if p.is_file() {
        return Some(p.to_string_lossy().to_string());
    }
    None
}

#[cfg(windows)]
fn find_in_path_excluding_system32(name: &str) -> Option<String> {
    // Search PATH for name (e.g., "bash.exe"), but skip the WSL shim in System32.
    let wanted = name.to_ascii_lowercase();
    if let Some(path) = env::var_os("PATH") {
        for dir in env::split_paths(&path) {
            let mut cand = dir.clone();
            cand.push(&wanted);
            let s = cand.to_string_lossy().to_string();
            if cand.is_file() {
                // Skip the legacy WSL shim
                let s_lower = s.to_ascii_lowercase();
                if s_lower.contains("\\windows\\system32\\bash.exe") {
                    continue;
                }
                return Some(s);
            }
        }
    }
    None
}

#[cfg(windows)]
fn find_shell_interpreter_on_windows(name: &str) -> Option<String> {
    // 1) Respect explicit override
    if let Ok(custom) = env::var("TRITON_BASH") {
        let p = custom.trim_matches('"').trim();
        if !p.is_empty() && Path::new(p).is_file() {
            return Some(p.to_string());
        }
    }

    let exe = format!("{name}.exe");

    // 2) PATH search, but skip System32\bash.exe (WSL shim)
    if let Some(p) = find_in_path_excluding_system32(&exe) {
        return Some(p);
    }

    // 3) Well-known installs
    let mut candidates: Vec<String> = Vec::new();

    // ProgramFiles variants
    for pf in ["ProgramFiles", "ProgramFiles(x86)"] {
        if let Ok(root) = env::var(pf) {
            // Git for Windows
            candidates.extend(
                [
                    "\\Git\\bin\\bash.exe",
                    "\\Git\\usr\\bin\\bash.exe",
                    "\\Git\\bin\\sh.exe",
                    "\\Git\\usr\\bin\\sh.exe",
                ]
                .iter()
                .filter_map(|suf| path_join_has_exe(&root, suf)),
            );
        }
    }

    // MSYS2 / Cygwin common defaults
    for root in [
        "C:\\msys64\\usr\\bin",
        "C:\\msys32\\usr\\bin",
        "C:\\cygwin64\\bin",
        "C:\\cygwin\\bin",
    ] {
        if let Some(p) = path_join_has_exe(root, &exe) {
            candidates.push(p);
        }
    }

    // If they asked for "sh", also accept bash.exe from the same locations.
    if name.eq_ignore_ascii_case("sh") {
        for root in [
            "C:\\msys64\\usr\\bin",
            "C:\\msys32\\usr\\bin",
            "C:\\cygwin64\\bin",
            "C:\\cygwin\\bin",
        ] {
            if let Some(p) = path_join_has_exe(root, "bash.exe") {
                candidates.push(p);
            }
        }
        for pf in ["ProgramFiles", "ProgramFiles(x86)"] {
            if let Ok(root) = env::var(pf) {
                if let Some(p) = path_join_has_exe(&root, "\\Git\\usr\\bin\\bash.exe") {
                    candidates.push(p);
                }
                if let Some(p) = path_join_has_exe(&root, "\\Git\\bin\\bash.exe") {
                    candidates.push(p);
                }
            }
        }
    }

    candidates.into_iter().find(|p| Path::new(p).is_file())
}

pub fn handle_script(tokens: &[String]) -> Result<()> {
    if tokens.is_empty() {
        bail!("No script name provided");
    }

    // Detect leading "bash …" or "sh …" and return (interpreter, rest)
    fn detect_shell_invocation<'a>(s: &'a str) -> Option<(&'static str, &'a str)> {
        let s = s.trim_start();
        let bytes = s.as_bytes();

        if s.len() >= 4 && s[..4].eq_ignore_ascii_case("bash") {
            let mut i = 4;
            while i < s.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            return Some(("bash", &s[i..]));
        }

        if s.len() >= 2 && s[..2].eq_ignore_ascii_case("sh") {
            let mut i = 2;
            while i < s.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            return Some(("sh", &s[i..]));
        }

        None
    }

    let script_name = &tokens[0];
    let args = &tokens[1..].iter().map(|s| s.as_str()).collect::<Vec<_>>();

    let cwd = env::current_dir()?;
    let scripts = load_scripts(&cwd)?;

    let raw = scripts
        .get(script_name)
        .ok_or_else(|| anyhow!("Unknown script: {}", script_name))?;
    let script: &str = raw.as_str();

    #[cfg(windows)]
    {
        // Special-case "bash …" / "sh …" to avoid hitting the WSL shim.
        if let Some((interp, rest_raw)) = detect_shell_invocation(script) {
            // Resolve an actual MinGW/MSYS/Cygwin/Git Bash interpreter.
            let resolved = find_shell_interpreter_on_windows(interp).ok_or_else(|| {
                anyhow!(
                    "Could not find a usable '{}' on Windows.\n\
                     Install Git for Windows or MSYS2 and ensure their bash/sh is on PATH,\n\
                     or set TRITON_BASH to the full path of bash.exe.",
                    interp
                )
            })?;

            // Normalize path for POSIX shell:
            //   - convert backslashes to forward slashes
            //   - ensure relative paths start with "./"
            let mut rest = rest_raw.trim();
            if let Some(r) = rest.strip_prefix(".\\") {
                rest = r;
            }
            let mut arg0 = rest.replace('\\', "/");
            if !(arg0.starts_with("./") || arg0.starts_with('/')) {
                arg0 = format!("./{}", arg0);
            }

            let status = Command::new(&resolved)
                .arg(&arg0)
                .args(args)
                .current_dir(&cwd)
                .status()
                .with_context(|| format!("Failed to spawn {} {}", resolved, &arg0))?;

            if !status.success() {
                bail!(
                    "Script \"{}\" exited with exit code: {}",
                    script_name,
                    status.code().unwrap_or(-1)
                );
            }
            return Ok(());
        }

        let use_cmd_shell = is_shelly(script);

        if use_cmd_shell {
            // Build: "<script> <args...>" for cmd /C
            let mut merged = script.to_string();
            for a in args {
                merged.push(' ');
                merged.push_str(&shell_escape(a));
            }

            let status = Command::new("cmd")
                .arg("/C")
                .arg(&merged)
                .current_dir(&cwd)
                .status()
                .with_context(|| format!("Failed to launch cmd.exe for script {}", script_name))?;

            if !status.success() {
                bail!(
                    "Script \"{}\" exited with exit code: {}",
                    script_name,
                    status.code().unwrap_or(-1)
                );
            }
            return Ok(());
        } else {
            // Split the script into program + its own arguments.
            // e.g. "dotnet build path/to/project" → program="dotnet", script_args=["build", "path/to/project"]
            let (program, script_args) = split_command(script);

            // If the program looks like a path, normalize it; .cmd/.bat go through cmd.exe
            if looks_like_path(program) {
                let normalized = normalize_windows_path(&cwd, program);
                if normalized.ends_with(".cmd") || normalized.ends_with(".bat") {
                    // .cmd/.bat must go through cmd.exe
                    let mut merged = normalized;
                    for a in &script_args {
                        merged.push(' ');
                        merged.push_str(&shell_escape(a));
                    }
                    for a in args {
                        merged.push(' ');
                        merged.push_str(&shell_escape(a));
                    }
                    let status = Command::new("cmd")
                        .arg("/C")
                        .arg(&merged)
                        .current_dir(&cwd)
                        .status()
                        .with_context(|| format!("Failed to launch cmd.exe for script {}", script_name))?;
                    if !status.success() {
                        bail!(
                            "Script \"{}\" exited with exit code: {}",
                            script_name,
                            status.code().unwrap_or(-1)
                        );
                    }
                    return Ok(());
                }

                let status = Command::new(&normalized)
                    .args(&script_args)
                    .args(args)
                    .current_dir(&cwd)
                    .status()
                    .with_context(|| format!("Failed to spawn {}", normalized))?;
                if !status.success() {
                    bail!(
                        "Script \"{}\" exited with exit code: {}",
                        script_name,
                        status.code().unwrap_or(-1)
                    );
                }
                return Ok(());
            }

            let status = Command::new(program)
                .args(&script_args)
                .args(args)
                .current_dir(&cwd)
                .status()
                .with_context(|| format!("Failed to spawn {}", program))?;
            if !status.success() {
                bail!(
                    "Script \"{}\" exited with exit code: {}",
                    script_name,
                    status.code().unwrap_or(-1)
                );
            }
            return Ok(());
        }
    }

    #[cfg(not(windows))]
    {
        if let Some((interp, rest)) = detect_shell_invocation(script) {
            let status = Command::new(interp)
                .arg(rest)
                .args(args)
                .current_dir(&cwd)
                .status()
                .with_context(|| format!("Failed to spawn {} {}", interp, rest))?;
            if !status.success() {
                bail!(
                    "Script \"{}\" exited with exit code: {}",
                    script_name,
                    status.code().unwrap_or(-1)
                );
            }
            return Ok(());
        }

        // Shell line with operators → sh -c
        if is_shelly(script) && !looks_like_path(script) {
            let mut merged = script.to_string();
            for a in args {
                merged.push(' ');
                merged.push_str(&shell_escape(a));
            }
            let status = Command::new("sh")
                .arg("-c")
                .arg(merged)
                .current_dir(&cwd)
                .status()
                .with_context(|| format!("Failed to launch sh for script {}", script_name))?;
            if !status.success() {
                bail!(
                    "Script \"{}\" exited with exit code: {}",
                    script_name,
                    status.code().unwrap_or(-1)
                );
            }
            return Ok(());
        }

        // Split the script into program + its own arguments.
        // e.g. "dotnet build path/to/project" → program="dotnet", script_args=["build", "path/to/project"]
        let (program, script_args) = split_command(script);

        // Path-like: resolve ./ and run directly, else fall back to PATH
        let prog = if let Some(rest) = program.strip_prefix("./") {
            cwd.join(rest)
        } else {
            cwd.join(program)
        };

        if prog.exists() {
            let status = Command::new(&prog)
                .args(&script_args)
                .args(args)
                .current_dir(&cwd)
                .status()
                .with_context(|| format!("Failed to spawn {}", prog.display()))?;
            if !status.success() {
                bail!(
                    "Script \"{}\" exited with exit code: {}",
                    script_name,
                    status.code().unwrap_or(-1)
                );
            }
            return Ok(());
        } else {
            let status = Command::new(program)
                .args(&script_args)
                .args(args)
                .current_dir(&cwd)
                .status()
                .with_context(|| format!("Failed to spawn {}", program))?;
            if !status.success() {
                bail!(
                    "Script \"{}\" exited with exit code: {}",
                    script_name,
                    status.code().unwrap_or(-1)
                );
            }
            return Ok(());
        }
    }
}
