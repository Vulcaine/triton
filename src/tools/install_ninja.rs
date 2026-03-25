use anyhow::{bail, Context, Result};
use std::env;
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt; // only on Unix
use std::path::{Path, PathBuf};
use std::process::Command;

/// Utility: join path pieces safely on PATH (process-local).
fn prepend_to_process_path(dir: &Path) {
    let existing = env::var_os("PATH").unwrap_or_default();
    let mut parts = env::split_paths(&existing).collect::<Vec<_>>();
    if !parts.iter().any(|p| p == dir) {
        parts.insert(0, dir.to_path_buf());
        if let Ok(joined) = env::join_paths(parts) {
            env::set_var("PATH", joined);
        }
    }
}

fn ninja_bin_in(dir: &Path) -> PathBuf {
    if cfg!(windows) { dir.join("ninja.exe") } else { dir.join("ninja") }
}
fn has_ninja(dir: &Path) -> bool { ninja_bin_in(dir).exists() }

/// Windows-only: strip the verbatim prefix (\\?\) which PowerShell cmdlets reject.
#[cfg(windows)]
fn win_normalize_path_for_ps(p: &Path) -> String {
    let s = p.as_os_str().to_string_lossy().to_string();
    let t = s.strip_prefix(r"\\?\").unwrap_or(&s).to_string();
    t
}
#[cfg(not(windows))]
fn win_normalize_path_for_ps(p: &Path) -> String {
    p.as_os_str().to_string_lossy().to_string()
}

/// Find an executable in PATH and return the directory containing it.
fn find_in_path(bin_name: &str) -> Option<PathBuf> {
    if let Ok(path_var) = env::var("PATH") {
        for dir in env::split_paths(&path_var) {
            let exe = if cfg!(windows) { dir.join(format!("{bin_name}.exe")) } else { dir.join(bin_name) };
            if exe.exists() { return Some(dir); }
        }
    }
    None
}

/// Known installation locations per platform (winget/choco/scoop etc).
fn known_ninja_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if cfg!(windows) {
        if let Ok(local) = env::var("LOCALAPPDATA") {
            dirs.push(Path::new(&local).join(r"Microsoft\WinGet\Links")); // Winget shims
        }
        dirs.push(Path::new(r"C:\ProgramData\chocolatey\bin").to_path_buf()); // Chocolatey
        if let Ok(home) = env::var("USERPROFILE") {
            dirs.push(Path::new(&home).join(r"scoop\shims"));                   // Scoop
            dirs.push(Path::new(&home).join(r"scoop\apps\ninja\current"));
        }
        if let Ok(program_files) = env::var("ProgramFiles") {
            dirs.push(Path::new(&program_files).join(r"Git\usr\bin")); // Git for Windows
        }
        dirs.push(Path::new(r"C:\Program Files\Ninja").to_path_buf()); // generic
    } else if cfg!(target_os = "macos") {
        dirs.push(Path::new("/opt/homebrew/bin").to_path_buf());
        dirs.push(Path::new("/usr/local/bin").to_path_buf());
    } else {
        dirs.push(Path::new("/usr/bin").to_path_buf());
        dirs.push(Path::new("/usr/local/bin").to_path_buf());
    }
    dirs
}

/// Best-effort install via package managers (non-fatal).
fn try_package_manager_install() {
    #[cfg(windows)]
    {
        if Command::new("winget").arg("--version").output().is_ok() {
            let _ = Command::new("winget").args(["install", "-e", "--id", "Ninja-build.Ninja"]).status();
        } else if Command::new("choco").arg("--version").output().is_ok() {
            let _ = Command::new("choco").args(["install", "-y", "ninja"]).status();
        } else if Command::new("scoop").arg("--version").output().is_ok() {
            let _ = Command::new("scoop").args(["install", "ninja"]).status();
        }
    }
    #[cfg(target_os = "macos")]
    {
        if Command::new("brew").arg("--version").output().is_ok() {
            let _ = Command::new("brew").args(["install", "ninja"]).status();
        }
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        if Command::new("apt-get").arg("--version").output().is_ok() {
            let _ = Command::new("bash").arg("-lc")
                .arg("sudo apt-get update && sudo apt-get install -y ninja-build").status();
        } else if Command::new("dnf").arg("--version").output().is_ok() {
            let _ = Command::new("sudo").args(["dnf", "install", "-y", "ninja-build"]).status();
        } else if Command::new("pacman").arg("-V").output().is_ok() {
            let _ = Command::new("sudo").args(["pacman", "-S", "--noconfirm", "ninja"]).status();
        } else if Command::new("zypper").arg("--version").output().is_ok() {
            let _ = Command::new("sudo").args(["zypper", "install", "-y", "ninja"]).status();
        }
    }
}

/// Download a portable ninja into `.triton/tools/ninja` and return an ABSOLUTE dir.
fn download_portable_ninja(project: &Path) -> Result<PathBuf> {
    let tools_dir = project.join(".triton").join("tools").join("ninja");
    fs::create_dir_all(&tools_dir)?;

    let ninja_path = if cfg!(windows) {
        // Use non-verbatim (no \\?\) absolute paths for PowerShell
        let tools_abs = tools_dir.canonicalize().unwrap_or_else(|_| tools_dir.clone());
        let zip_path = tools_abs.join("ninja-win.zip");
        let url = "https://github.com/ninja-build/ninja/releases/latest/download/ninja-win.zip";

        let zip_ps = win_normalize_path_for_ps(&zip_path).replace('\\', "\\\\");
        let dst_ps = win_normalize_path_for_ps(&tools_abs).replace('\\', "\\\\");

        // Try PowerShell first (ensure system PATH includes Windows dirs)
        let mut path = env::var("PATH").unwrap_or_default();
        let sys_root = env::var("SystemRoot").unwrap_or_else(|_| "C:\\WINDOWS".into());
        let ps_dir = format!("{}\\System32\\WindowsPowerShell\\v1.0", sys_root);
        if !path.contains(&ps_dir) {
            path = format!("{};{}\\System32;{}", path, sys_root, ps_dir);
        }

        let ps = format!(
            r#"
$ProgressPreference = 'SilentlyContinue';
Invoke-WebRequest -Uri '{url}' -OutFile '{zip}';
Expand-Archive -Path '{zip}' -DestinationPath '{dst}' -Force;
"#,
            url = url,
            zip = zip_ps,
            dst = dst_ps
        );

        let status = Command::new("powershell")
            .arg("-NoProfile")
            .arg("-Command")
            .arg(ps)
            .env("PATH", &path)
            .status()
            .context("failed to run PowerShell to fetch Ninja")?;
        if !status.success() {
            bail!("portable ninja download failed");
        }
        tools_abs.join("ninja.exe")
    } else if cfg!(target_os = "macos") {
        let tools_abs = tools_dir.canonicalize().unwrap_or_else(|_| tools_dir.clone());
        let zip_path = tools_abs.join("ninja-mac.zip");
        let url = "https://github.com/ninja-build/ninja/releases/latest/download/ninja-mac.zip";
        let status = Command::new("bash")
            .arg("-lc")
            .arg(format!(
                "curl -L '{}' -o '{}' && unzip -o '{}' -d '{}'",
                url,
                zip_path.display(),
                zip_path.display(),
                tools_abs.display()
            ))
            .status()
            .context("failed to curl/unzip ninja")?;
        if !status.success() {
            bail!("portable ninja download failed");
        }
        tools_abs.join("ninja")
    } else {
        let tools_abs = tools_dir.canonicalize().unwrap_or_else(|_| tools_dir.clone());
        let zip_path = tools_abs.join("ninja-linux.zip");
        let url = "https://github.com/ninja-build/ninja/releases/latest/download/ninja-linux.zip";
        let status = Command::new("bash")
            .arg("-lc")
            .arg(format!(
                "curl -L '{}' -o '{}' && unzip -o '{}' -d '{}'",
                url,
                zip_path.display(),
                zip_path.display(),
                tools_abs.display()
            ))
            .status()
            .context("failed to curl/unzip ninja")?;
        if !status.success() {
            bail!("portable ninja download failed");
        }
        tools_abs.join("ninja")
    };

    if !ninja_path.exists() {
        bail!("ninja binary not found after download");
    }

    // Ensure executable bit on Unix
    #[cfg(unix)]
    {
        let mut perms = fs::metadata(&ninja_path)?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&ninja_path, perms)?;
    }

    // Return ABSOLUTE dir
    let abs_dir = ninja_path.parent().unwrap().to_path_buf();
    Ok(abs_dir)
}

/// Ensure Ninja is available and return an **ABSOLUTE** directory we can prepend to PATH.
/// Strategy:
///   1) PATH
///   2) Known dirs (winget/choco/scoop/etc)
///   3) Package manager install (best effort), then rescan
///   4) Portable download into .triton/tools/ninja
/// Side effect: prepends the chosen dir to this process's PATH (so subcommands see it).
pub fn ensure_ninja_dir(project: &Path) -> Result<PathBuf> {
    // 1) PATH
    if let Some(dir) = find_in_path("ninja") {
        let abs = dir.canonicalize().unwrap_or(dir);
        prepend_to_process_path(&abs);
        eprintln!("Using Ninja at {}", ninja_bin_in(&abs).display());
        return Ok(abs);
    }

    // 2) Known dirs
    for dir in known_ninja_dirs() {
        if has_ninja(&dir) {
            let abs = dir.canonicalize().unwrap_or(dir);
            prepend_to_process_path(&abs);
            eprintln!("Using Ninja at {}", ninja_bin_in(&abs).display());
            return Ok(abs);
        }
    }

    // 3) Try to install via package manager (best effort)
    try_package_manager_install();

    // Rescan PATH and known dirs
    if let Some(dir) = find_in_path("ninja") {
        let abs = dir.canonicalize().unwrap_or(dir);
        prepend_to_process_path(&abs);
        eprintln!("Using Ninja at {}", ninja_bin_in(&abs).display());
        return Ok(abs);
    }
    for dir in known_ninja_dirs() {
        if has_ninja(&dir) {
            let abs = dir.canonicalize().unwrap_or(dir);
            prepend_to_process_path(&abs);
            eprintln!("Using Ninja at {}", ninja_bin_in(&abs).display());
            return Ok(abs);
        }
    }

    // 4) Portable fallback (ABSOLUTE + normalized for PS already handled)
    let abs = download_portable_ninja(project)?;
    prepend_to_process_path(&abs);
    eprintln!("Using Ninja (portable) at {}", ninja_bin_in(&abs).display());
    Ok(abs)
}
