use anyhow::{Context, Result};
use std::path::Path;
use std::process::Command;

/// Ensure vcpkg is cloned locally, and bootstrap if missing.
/// Returns path to the vcpkg toolchain file.
pub fn ensure_vcpkg(project: &Path) -> Result<String> {
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

    if !vcpkg_dir.join("vcpkg").exists() && !vcpkg_dir.join("vcpkg.exe").exists() {
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

    Ok(vcpkg_dir.join("scripts").join("buildsystems").join("vcpkg.cmake").display().to_string())
}
