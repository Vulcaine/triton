// cmake_install.rs

use anyhow::{anyhow, Context, Result};
use std::io::{self, Write};
use std::process::Command;
use crate::{cmake::effective_cmake_version, handle_generate};

/// Install/upgrade CMake. If `version_override` is provided, we **prompt** the user and then
/// attempt to install **that specific version** (where the package manager supports pinning).
/// If not provided, we ensure CMake is >= the version from `effective_cmake_version()`.
pub fn handle_cmake_install(version_override: Option<String>) -> Result<()> {
    // Resolve requested version string (if any) or the minimal required one.
    let requested = version_override
        .as_ref()
        .map(|s| s.trim().trim_start_matches(">=").trim().to_string())
        .filter(|s| !s.is_empty());

    // Determine the minimal acceptable version for the "ensure >= ..." flow.
    let minimal_required = {
        let t = effective_cmake_version();
        format!("{}.{}.{}", t.0, t.1, t.2)
    };

    // When a specific version is requested, we **do not** early out even if current is newer.
    // We will prompt the user first and, if downgrading, try to uninstall the existing one.
    if let Some(req) = &requested {
        let current = cmake_version()?;
        eprintln!("Requested CMake version: {}", req);
        let mut is_downgrade = false;

        match current.as_deref() {
            Some(cur) => {
                if version_gt(cur, req) {
                    // Downgrade warning (yellow)
                    is_downgrade = true;
                    eprintln!("\x1b[33mWarning:\x1b[0m You're requesting a downgrade from CMake {} to {}.", cur, req);
                } else if version_lt(cur, req) {
                    eprintln!("You are about to upgrade CMake from {} to {}.", cur, req);
                } else {
                    eprintln!("Requested version equals the currently installed version ({}).", cur);
                }
            }
            None => {
                eprintln!("CMake is not currently installed. It will be installed as {}.", req);
            }
        }

        // Extra notice for package managers that typically don't pin.
        if current_os() == Os::Mac {
            eprintln!("Note: Homebrew usually installs the latest CMake and may not pin exactly {}.", req);
        }

        if !prompt_yes_no("Proceed with installing the requested CMake version? [y/N] ") {
            return Err(anyhow!("User aborted CMake installation."));
        }

        // If this is a downgrade, proactively try to uninstall whatever CMake we can find via package managers.
        if is_downgrade {
            eprintln!("Attempting to uninstall existing CMake before installing {}...", req);
            try_uninstall_all(); // best-effort; errors are logged and ignored
        }

        // Attempt installation for the requested version (pin where supported).
        try_install_flow(Some(req.as_str()), &minimal_required)?;

        // Validate the result: if a specific version was requested, check for equality (best effort).
        if let Some(ver) = cmake_version()? {
            if version_eq(&ver, req) || version_ge(&ver, req) {
                eprintln!("✔ CMake {} is installed and ready.", ver);
                handle_generate()?;
                return Ok(());
            } else {
                return Err(anyhow!(
                    "After installation, cmake is {} which does not satisfy requested {}",
                    ver,
                    req
                ));
            }
        } else {
            return Err(anyhow!("Installation ran but cmake still not found on PATH."));
        }
    } else {
        // No specific version requested: ensure >= minimal_required
        eprintln!("Checking for cmake (>= {})...", minimal_required);

        if let Some(ver) = cmake_version()? {
            if version_ge(&ver, &minimal_required) {
                eprintln!("✔ Found cmake {}", ver);
                handle_generate()?;
                return Ok(());
            } else {
                eprintln!("• cmake {} is too old (need {}+), attempting upgrade...", ver, minimal_required);
            }
        } else {
            eprintln!("• cmake not found in PATH, attempting installation...");
        }

        try_install_flow(None, &minimal_required)?;

        if let Some(ver) = cmake_version()? {
            if version_ge(&ver, &minimal_required) {
                eprintln!("✔ cmake {} is installed and ready.", ver);
                handle_generate()?;
                return Ok(());
            } else {
                eprintln!("• Installer ran, but cmake is still {} (< {}).", ver, minimal_required);
            }
        } else {
            eprintln!("• Installer ran, but cmake still not in PATH.");
        }

        eprintln!();
        eprintln!("Sorry, we couldn't install/upgrade CMake automatically.");
        eprintln!("Please install CMake {}+ manually and ensure it's on your PATH:", minimal_required);
        eprintln!("  - Windows: https://cmake.org/download/ (choose the x64 Installer) or use winget/choco");
        eprintln!("  - macOS:  brew install cmake  (or grab the installer from cmake.org)");
        eprintln!("  - Linux:  use your distro’s package manager (apt/dnf/yum/pacman/zypper/apk) or snap");
        return Err(anyhow!("Unable to install CMake automatically"));
    }
}

/// Attempt all platform-appropriate installers. If `pin_version` is Some, we try to install that
/// exact version where supported (winget/choco/scoop/macports). Otherwise we install "cmake" latest.
///
/// On failure, returns a **summary** of all attempts instead of only the last error.
fn try_install_flow(pin_version: Option<&str>, minimal_required: &str) -> Result<()> {
    struct Attempt {
        name: &'static str,
        run: Box<dyn Fn() -> Result<()> + Send + Sync>,
    }

    let os = current_os();
    let mut attempts: Vec<Attempt> = Vec::new();

    match os {
        Os::Windows => {
            {
                let v = pin_version.map(|s| s.to_string());
                attempts.push(Attempt {
                    name: "winget",
                    run: Box::new(move || install_with_winget(v.as_deref())),
                });
            }
            {
                let v = pin_version.map(|s| s.to_string());
                attempts.push(Attempt {
                    name: "choco",
                    run: Box::new(move || install_with_choco(v.as_deref())),
                });
            }
            {
                let v = pin_version.map(|s| s.to_string());
                attempts.push(Attempt {
                    name: "scoop",
                    run: Box::new(move || install_with_scoop(v.as_deref())),
                });
            }
        }
        Os::Mac => {
            {
                let v = pin_version.map(|s| s.to_string());
                attempts.push(Attempt {
                    name: "brew",
                    run: Box::new(move || install_with_brew(v.as_deref())),
                });
            }
            {
                let v = pin_version.map(|s| s.to_string());
                attempts.push(Attempt {
                    name: "macports",
                    run: Box::new(move || install_with_macports(v.as_deref())),
                });
            }
        }
        Os::Linux => {
            attempts.push(Attempt { name: "apt", run: Box::new(|| install_with_apt()) });
            attempts.push(Attempt { name: "dnf", run: Box::new(|| install_with_dnf()) });
            attempts.push(Attempt { name: "yum", run: Box::new(|| install_with_yum()) });
            attempts.push(Attempt { name: "pacman", run: Box::new(|| install_with_pacman()) });
            attempts.push(Attempt { name: "zypper", run: Box::new(|| install_with_zypper()) });
            attempts.push(Attempt { name: "apk", run: Box::new(|| install_with_apk()) });
            attempts.push(Attempt { name: "snap", run: Box::new(|| install_with_snap()) });
        }
        Os::Unknown => {
            {
                let v = pin_version.map(|s| s.to_string());
                attempts.push(Attempt {
                    name: "brew",
                    run: Box::new(move || install_with_brew(v.as_deref())),
                });
            }
            attempts.push(Attempt { name: "apt", run: Box::new(|| install_with_apt()) });
            attempts.push(Attempt { name: "dnf", run: Box::new(|| install_with_dnf()) });
            attempts.push(Attempt { name: "yum", run: Box::new(|| install_with_yum()) });
            attempts.push(Attempt { name: "pacman", run: Box::new(|| install_with_pacman()) });
            attempts.push(Attempt { name: "zypper", run: Box::new(|| install_with_zypper()) });
            attempts.push(Attempt { name: "apk", run: Box::new(|| install_with_apk()) });
            attempts.push(Attempt { name: "snap", run: Box::new(|| install_with_snap()) });
        }
    }

    let total = attempts.len();
    let mut error_log: Vec<String> = Vec::new();

    for (idx, attempt) in attempts.into_iter().enumerate() {
        match (attempt.run)() {
            Ok(()) => {
                if let Some(ver) = cmake_version()? {
                    if let Some(req) = pin_version {
                        // Prefer exact match; accept newer if pin not possible but still meets minimal_required
                        if version_eq(&ver, req) || version_ge(&ver, minimal_required) {
                            return Ok(());
                        } else {
                            let msg = format!("{}: installer ran, but cmake is {} (not requested {}).", attempt.name, ver, req);
                            eprintln!("• {msg} Continuing attempts...");
                            error_log.push(msg);
                        }
                    } else if version_ge(&ver, minimal_required) {
                        return Ok(());
                    } else {
                        let msg = format!("{}: installer ran, but cmake is {} (< {}).", attempt.name, ver, minimal_required);
                        eprintln!("• {msg} Continuing attempts...");
                        error_log.push(msg);
                    }
                } else {
                    let msg = format!("{}: installer ran, but cmake not found on PATH.", attempt.name);
                    eprintln!("• {msg} Continuing attempts...");
                    error_log.push(msg);
                }
            }
            Err(e) => {
                let msg = format!("{}: {}", attempt.name, e);
                error_log.push(msg);
                if idx != total.saturating_sub(1) {
                    eprintln!("• Attempt failed; trying another installer...");
                }
            }
        }
    }

    // Summarize instead of returning only the last error.
    let mut summary = String::new();
    summary.push_str("Unable to install the requested CMake.\n");
    if let Some(req) = pin_version {
        summary.push_str(&format!("Requested version: {}\n", req));
    }
    summary.push_str("Tried multiple installers; all attempts failed. Summary:\n");
    for line in error_log {
        summary.push_str("  - ");
        summary.push_str(&line);
        summary.push('\n');
    }
    Err(anyhow!(summary.trim_end().to_string()))
}

/// Best-effort removal of an existing CMake via common package managers on the current platform.
/// All errors are **logged and ignored** so we can keep going with the fresh install.
fn try_uninstall_all() {
    match current_os() {
        Os::Windows => {
            if let Err(e) = uninstall_with_winget() {
                eprintln!("• winget uninstall failed: {e}");
            }
            if let Err(e) = uninstall_with_choco() {
                eprintln!("• choco uninstall failed: {e}");
            }
            if let Err(e) = uninstall_with_scoop() {
                eprintln!("• scoop uninstall failed: {e}");
            }
        }
        Os::Mac => {
            if let Err(e) = uninstall_with_brew() {
                eprintln!("• brew uninstall failed: {e}");
            }
            if let Err(e) = uninstall_with_macports() {
                eprintln!("• MacPorts uninstall failed: {e}");
            }
        }
        Os::Linux | Os::Unknown => {
            if let Err(e) = uninstall_with_apt() {
                eprintln!("• apt remove failed: {e}");
            }
            if let Err(e) = uninstall_with_dnf() {
                eprintln!("• dnf remove failed: {e}");
            }
            if let Err(e) = uninstall_with_yum() {
                eprintln!("• yum remove failed: {e}");
            }
            if let Err(e) = uninstall_with_pacman() {
                eprintln!("• pacman remove failed: {e}");
            }
            if let Err(e) = uninstall_with_zypper() {
                eprintln!("• zypper remove failed: {e}");
            }
            if let Err(e) = uninstall_with_apk() {
                eprintln!("• apk del failed: {e}");
            }
            if let Err(e) = uninstall_with_snap() {
                eprintln!("• snap remove failed: {e}");
            }
        }
    }
}

/* ------------------------- Prompt helpers ------------------------- */

fn prompt_yes_no(prompt: &str) -> bool {
    eprint!("{prompt}");
    let _ = io::stdout().flush();
    let _ = io::stderr().flush();

    let mut input = String::new();
    if io::stdin().read_line(&mut input).is_ok() {
        let resp = input.trim().to_lowercase();
        resp == "y" || resp == "yes"
    } else {
        false
    }
}

/* ------------------------- Version helpers ------------------------- */

fn cmake_version() -> Result<Option<String>> {
    let out = Command::new("cmake").arg("--version").output();

    match out {
        Ok(o) if o.status.success() => {
            let txt = String::from_utf8_lossy(&o.stdout);
            if let Some(first) = txt.lines().next() {
                let ver = first.trim().strip_prefix("cmake version ").unwrap_or("").trim();
                if !ver.is_empty() {
                    return Ok(Some(normalize_version_string(ver)));
                }
            }
            Ok(None)
        }
        _ => Ok(None),
    }
}

fn normalize_version_string(ver: &str) -> String {
    // Keep only numeric dot-separated prefix, e.g., "3.30.5-rc1" -> "3.30.5"
    let mut out = String::new();
    for ch in ver.chars() {
        if ch.is_ascii_digit() || ch == '.' {
            out.push(ch);
        } else {
            break;
        }
    }
    if out.is_empty() { ver.to_string() } else { out }
}

fn parse_version(v: &str) -> Vec<u64> {
    v.split('.').map(|x| x.parse::<u64>().unwrap_or(0)).collect()
}

fn version_ge(a: &str, b: &str) -> bool {
    let (av, bv) = (parse_version(a), parse_version(b));
    let n = av.len().max(bv.len());
    for i in 0..n {
        let ai = *av.get(i).unwrap_or(&0);
        let bi = *bv.get(i).unwrap_or(&0);
        if ai > bi { return true; }
        if ai < bi { return false; }
    }
    true
}

fn version_gt(a: &str, b: &str) -> bool {
    version_ge(a, b) && !version_eq(a, b)
}

fn version_lt(a: &str, b: &str) -> bool {
    !version_ge(a, b)
}

fn version_eq(a: &str, b: &str) -> bool {
    parse_version(a) == parse_version(b)
}

/* ---------------------------- OS detection ---------------------------- */

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Os {
    Windows,
    Mac,
    Linux,
    Unknown,
}

fn current_os() -> Os {
    if cfg!(target_os = "windows") {
        Os::Windows
    } else if cfg!(target_os = "macos") {
        Os::Mac
    } else if cfg!(target_os = "linux") {
        Os::Linux
    } else {
        Os::Unknown
    }
}

/* ------------------------- Command utilities ------------------------- */

fn cmd_exists(bin: &str) -> bool {
    if current_os() == Os::Windows {
        Command::new("where")
            .arg(bin)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    } else {
        Command::new("which")
            .arg(bin)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }
}

fn run(cmd: &str, args: &[&str]) -> Result<()> {
    eprintln!("$ {} {}", cmd, args.join(" "));
    let status = Command::new(cmd)
        .args(args)
        .status()
        .with_context(|| format!("failed to spawn '{}'", cmd))?;
    if !status.success() {
        return Err(anyhow!("command '{}' exited with {}", cmd, status));
    }
    Ok(())
}

fn run_sudo(cmd: &str, args: &[&str]) -> Result<()> {
    if current_os() == Os::Windows {
        return run(cmd, args);
    }
    if !cmd_exists("sudo") {
        return run(cmd, args);
    }
    eprintln!("$ sudo {} {}", cmd, args.join(" "));
    let status = Command::new("sudo")
        .arg(cmd)
        .args(args)
        .status()
        .with_context(|| format!("failed to spawn 'sudo {}'", cmd))?;
    if !status.success() {
        return Err(anyhow!("'sudo {}' exited with {}", cmd, status));
    }
    Ok(())
}

/* ------------------------ Installers: Windows ------------------------ */

fn install_with_winget(ver: Option<&str>) -> Result<()> {
    if !cmd_exists("winget") {
        return Err(anyhow!("winget not found"));
    }
    eprintln!("Attempting install via winget (Kitware.CMake)...");
    let mut args = vec!["install", "--id", "Kitware.CMake", "-e", "--source", "winget"];
    if let Some(v) = ver {
        args.push("--version");
        args.push(v);
    }
    run("winget", &args)?;
    Ok(())
}

fn install_with_choco(ver: Option<&str>) -> Result<()> {
    if !cmd_exists("choco") {
        return Err(anyhow!("choco not found"));
    }
    eprintln!("Attempting install via Chocolatey (cmake)...");
    let mut args = vec![
        "install", "cmake", "--yes", "--installargs", "ADD_CMAKE_TO_PATH=System",
    ];
    if let Some(v) = ver {
        args.push("--version");
        args.push(v);
    }
    run("choco", &args)?;
    Ok(())
}

fn install_with_scoop(ver: Option<&str>) -> Result<()> {
    if !cmd_exists("scoop") {
        return Err(anyhow!("scoop not found"));
    }
    eprintln!("Attempting install via Scoop (cmake)...");
    if let Some(v) = ver {
        let pkg = format!("cmake@{}", v);
        run("scoop", &["install", &pkg])?;
    } else {
        run("scoop", &["install", "cmake"])?;
    }
    Ok(())
}

/* ------------------------- Installers: macOS ------------------------- */

fn install_with_brew(ver: Option<&str>) -> Result<()> {
    if !cmd_exists("brew") {
        return Err(anyhow!("Homebrew not found"));
    }
    eprintln!("Attempting install via Homebrew (cmake)...");
    if ver.is_some() {
        eprintln!("  (Note: Homebrew generally doesn't support pinning arbitrary CMake versions; installing latest available)");
    }
    run("brew", &["update"])?;
    run("brew", &["install", "cmake"])?;
    Ok(())
}

fn install_with_macports(ver: Option<&str>) -> Result<()> {
    if !cmd_exists("port") {
        return Err(anyhow!("MacPorts not found"));
    }
    eprintln!("Attempting install via MacPorts (cmake)...");
    run_sudo("port", &["selfupdate"])?;
    if let Some(v) = ver {
        // Attempt to pin: `port install cmake @<version>`
        let args: Vec<String> = vec!["install".into(), "cmake".into(), format!("@{}", v)];
        let args_str: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        run_sudo("port", &args_str)?;
    } else {
        run_sudo("port", &["install", "cmake"])?;
    }
    Ok(())
}

/* -------------------------- Installers: Linux ------------------------- */

fn install_with_apt() -> Result<()> {
    if !cmd_exists("apt-get") && !cmd_exists("apt") {
        return Err(anyhow!("apt not found"));
    }
    eprintln!("Attempting install via APT (cmake)...");
    if cmd_exists("apt-get") {
        run_sudo("apt-get", &["update"])?;
        run_sudo("apt-get", &["install", "-y", "cmake"])?;
    } else {
        run_sudo("apt", &["update"])?;
        run_sudo("apt", &["install", "-y", "cmake"])?;
    }
    Ok(())
}

fn install_with_dnf() -> Result<()> {
    if !cmd_exists("dnf") {
        return Err(anyhow!("dnf not found"));
    }
    eprintln!("Attempting install via DNF (cmake)...");
    run_sudo("dnf", &["install", "-y", "cmake"])?;
    Ok(())
}

fn install_with_yum() -> Result<()> {
    if !cmd_exists("yum") {
        return Err(anyhow!("yum not found"));
    }
    eprintln!("Attempting install via YUM (cmake)...");
    run_sudo("yum", &["install", "-y", "cmake"])?;
    Ok(())
}

fn install_with_pacman() -> Result<()> {
    if !cmd_exists("pacman") {
        return Err(anyhow!("pacman not found"));
    }
    eprintln!("Attempting install via Pacman (cmake)...");
    run_sudo("pacman", &["-Sy", "--noconfirm", "cmake"])?;
    Ok(())
}

fn install_with_zypper() -> Result<()> {
    if !cmd_exists("zypper") {
        return Err(anyhow!("zypper not found"));
    }
    eprintln!("Attempting install via Zypper (cmake)...");
    run_sudo("zypper", &["install", "-y", "cmake"])?;
    Ok(())
}

fn install_with_apk() -> Result<()> {
    if !cmd_exists("apk") {
        return Err(anyhow!("apk not found"));
    }
    eprintln!("Attempting install via Alpine APK (cmake)...");
    run_sudo("apk", &["add", "cmake"])?;
    Ok(())
}

fn install_with_snap() -> Result<()> {
    if !cmd_exists("snap") {
        return Err(anyhow!("snap not found"));
    }
    eprintln!("Attempting install via Snap (cmake)...");
    run_sudo("snap", &["install", "cmake", "--classic"])?;
    Ok(())
}

/* ------------------------ Uninstallers ------------------------ */

fn uninstall_with_winget() -> Result<()> {
    if !cmd_exists("winget") {
        return Err(anyhow!("winget not found"));
    }
    eprintln!("Attempting uninstall via winget (Kitware.CMake)...");
    run("winget", &["uninstall", "--id", "Kitware.CMake", "-e"])?;
    Ok(())
}

fn uninstall_with_choco() -> Result<()> {
    if !cmd_exists("choco") {
        return Err(anyhow!("choco not found"));
    }
    eprintln!("Attempting uninstall via Chocolatey (cmake)...");
    run("choco", &["uninstall", "cmake", "-y"])?;
    Ok(())
}

fn uninstall_with_scoop() -> Result<()> {
    if !cmd_exists("scoop") {
        return Err(anyhow!("scoop not found"));
    }
    eprintln!("Attempting uninstall via Scoop (cmake)...");
    run("scoop", &["uninstall", "cmake"])?;
    Ok(())
}

fn uninstall_with_brew() -> Result<()> {
    if !cmd_exists("brew") {
        return Err(anyhow!("Homebrew not found"));
    }
    eprintln!("Attempting uninstall via Homebrew (cmake)...");
    run("brew", &["uninstall", "cmake"])?;
    Ok(())
}

fn uninstall_with_macports() -> Result<()> {
    if !cmd_exists("port") {
        return Err(anyhow!("MacPorts not found"));
    }
    eprintln!("Attempting uninstall via MacPorts (cmake)...");
    run_sudo("port", &["uninstall", "cmake"])?;
    Ok(())
}

fn uninstall_with_apt() -> Result<()> {
    if !cmd_exists("apt-get") && !cmd_exists("apt") {
        return Err(anyhow!("apt not found"));
    }
    eprintln!("Attempting remove via APT (cmake)...");
    if cmd_exists("apt-get") {
        run_sudo("apt-get", &["remove", "-y", "cmake"])?;
    } else {
        run_sudo("apt", &["remove", "-y", "cmake"])?;
    }
    Ok(())
}

fn uninstall_with_dnf() -> Result<()> {
    if !cmd_exists("dnf") {
        return Err(anyhow!("dnf not found"));
    }
    eprintln!("Attempting remove via DNF (cmake)...");
    run_sudo("dnf", &["remove", "-y", "cmake"])?;
    Ok(())
}

fn uninstall_with_yum() -> Result<()> {
    if !cmd_exists("yum") {
        return Err(anyhow!("yum not found"));
    }
    eprintln!("Attempting remove via YUM (cmake)...");
    run_sudo("yum", &["remove", "-y", "cmake"])?;
    Ok(())
}

fn uninstall_with_pacman() -> Result<()> {
    if !cmd_exists("pacman") {
        return Err(anyhow!("pacman not found"));
    }
    eprintln!("Attempting remove via Pacman (cmake)...");
    run_sudo("pacman", &["-R", "--noconfirm", "cmake"])?;
    Ok(())
}

fn uninstall_with_zypper() -> Result<()> {
    if !cmd_exists("zypper") {
        return Err(anyhow!("zypper not found"));
    }
    eprintln!("Attempting remove via Zypper (cmake)...");
    run_sudo("zypper", &["remove", "-y", "cmake"])?;
    Ok(())
}

fn uninstall_with_apk() -> Result<()> {
    if !cmd_exists("apk") {
        return Err(anyhow!("apk not found"));
    }
    eprintln!("Attempting delete via Alpine APK (cmake)...");
    run_sudo("apk", &["del", "cmake"])?;
    Ok(())
}

fn uninstall_with_snap() -> Result<()> {
    if !cmd_exists("snap") {
        return Err(anyhow!("snap not found"));
    }
    eprintln!("Attempting remove via Snap (cmake)...");
    run_sudo("snap", &["remove", "cmake"])?;
    Ok(())
}
