use anyhow::{anyhow, bail, Context, Result};
use std::collections::HashMap;
use std::env;
use std::path::{Path};
use std::process::Command;

use crate::models::TritonRoot;
use crate::util;

#[cfg(windows)]
fn is_shelly(s: &str) -> bool {
    // cmd operators that require running through `cmd.exe /C`
    let ops = ["&&", "||", "|", ">", "<", "&", "(" , ")", "%"];
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
            if ch == '"' { out.push_str(r#"\""#); } else { out.push(ch); }
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

pub fn handle_script(tokens: &[String]) -> Result<()> {
    if tokens.is_empty() {
        bail!("No script name provided");
    }

    let script_name = &tokens[0];
    let args = &tokens[1..].iter().map(|s| s.as_str()).collect::<Vec<_>>();

    let cwd = env::current_dir()?;
    let scripts = load_scripts(&cwd)?;

    let raw = scripts
        .get(script_name)
        .ok_or_else(|| anyhow!("Unknown script: {}", script_name))?;
    let script: &str = raw.as_str();

    // Decide execution strategy
    #[cfg(windows)]
    {
        let mut use_cmd_shell = is_shelly(script);
        let cmdline_owned: Option<String>;

        // If it looks like a path (and especially for .cmd/.bat), normalize to absolute
        let mut prog = script.to_string();
        if looks_like_path(&prog) {
            prog = normalize_windows_path(&cwd, &prog);
            // .cmd / .bat must go through cmd.exe /C
            if prog.ends_with(".cmd") || prog.ends_with(".bat") {
                use_cmd_shell = true;
            }
        }

        if use_cmd_shell {
            // Build single /C string: "<prog> <args...>"
            let mut merged = prog.clone();
            for a in args {
                merged.push(' ');
                merged.push_str(&shell_escape(a));
            }
            cmdline_owned = Some(merged);

            let status = Command::new("cmd")
                .arg("/C")
                .arg(cmdline_owned.as_ref().unwrap())
                .current_dir(&cwd)
                .status()
                .with_context(|| format!("Failed to launch cmd.exe for script {}", script_name))?;

            if !status.success() {
                bail!("Script \"{}\" exited with exit code: {}", script_name, status.code().unwrap_or(-1));
            }
            return Ok(());
        } else {
            // Direct exec (e.g. .exe)
            let status = Command::new(&prog)
                .args(args)
                .current_dir(&cwd)
                .status()
                .with_context(|| format!("Failed to spawn {}", prog))?;
            if !status.success() {
                bail!("Script \"{}\" exited with exit code: {}", script_name, status.code().unwrap_or(-1));
            }
            return Ok(());
        }
    }

    #[cfg(not(windows))]
    {
        // Unix: shell line vs direct path
        if is_shelly(script) && !looks_like_path(script) {
            // plain shell line → sh -c "script args..."
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
                bail!("Script \"{}\" exited with exit code: {}", script_name, status.code().unwrap_or(-1));
            }
            return Ok(());
        }

        // Path-like: resolve ./ and run directly
        let prog = if let Some(rest) = script.strip_prefix("./") {
            cwd.join(rest)
        } else {
            cwd.join(script)
        };
        if !prog.exists() {
            // Fall back to PATH
            let status = Command::new(script)
                .args(args)
                .current_dir(&cwd)
                .status()
                .with_context(|| format!("Failed to spawn {}", script))?;
            if !status.success() {
                bail!("Script \"{}\" exited with exit code: {}", script_name, status.code().unwrap_or(-1));
            }
            return Ok(());
        }
        // ensure executable bit? (tests set it)
        let status = Command::new(&prog)
            .args(args)
            .current_dir(&cwd)
            .status()
            .with_context(|| format!("Failed to spawn {}", prog.display()))?;
        if !status.success() {
            bail!("Script \"{}\" exited with exit code: {}", script_name, status.code().unwrap_or(-1));
        }
        Ok(())
    }
}
