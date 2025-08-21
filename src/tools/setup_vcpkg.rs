use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Ensure vcpkg is cloned locally, and bootstrap if missing.
/// Returns (toolchain file path, path to vcpkg executable).
pub fn ensure_vcpkg(project: &Path) -> Result<(String, PathBuf)> {
    let vcpkg_dir = project.join("vcpkg");
    if !vcpkg_dir.exists() {
        eprintln!("Cloning vcpkg...");
        Command::new("git")
            .args(["clone", "https://github.com/microsoft/vcpkg.git", "vcpkg"])
            .current_dir(project)
            .status()
            .context("failed to clone vcpkg")?;
    }

    // bootstrap
    #[cfg(windows)]
    let bootstrap = vcpkg_dir.join("bootstrap-vcpkg.bat");
    #[cfg(not(windows))]
    let bootstrap = vcpkg_dir.join("bootstrap-vcpkg.sh");

    let vcpkg_exe = if cfg!(windows) {
        vcpkg_dir.join("vcpkg.exe")
    } else {
        vcpkg_dir.join("vcpkg")
    };

    if !vcpkg_exe.exists() {
        eprintln!("Bootstrapping vcpkg...");
        let status = if cfg!(windows) {
            Command::new("cmd")
                .args(["/C", bootstrap.to_str().unwrap()])
                .current_dir(&vcpkg_dir)
                .status()
        } else {
            Command::new("bash")
                .arg(bootstrap.to_str().unwrap())
                .current_dir(&vcpkg_dir)
                .status()
        }
        .context("failed to bootstrap vcpkg")?;
        if !status.success() {
            anyhow::bail!("vcpkg bootstrap failed");
        }
    }

    let toolchain = vcpkg_dir
        .join("scripts")
        .join("buildsystems")
        .join("vcpkg.cmake")
        .display()
        .to_string();

    Ok((toolchain, vcpkg_exe))
}
