use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::cmake::detect_vcpkg_triplet;
use crate::models::TritonRoot;

/// Install all vcpkg deps valid for current host, using the project-local vcpkg binary.
pub fn handle_install(_root: &TritonRoot, project: &Path, vcpkg_exe: &PathBuf) -> Result<()> {
    // ensure vcpkg.json is up-to-date
    crate::commands::handle_generate()?;  

    eprintln!("Running vcpkg install with manifest mode...");

    // Use the project-local vcpkg binary (bootstrapped by ensure_vcpkg).
    #[cfg(windows)]
    let vcpkg_bin = project.join("vcpkg").join("vcpkg.exe");
    #[cfg(not(windows))]
    let vcpkg_bin = project.join("vcpkg").join("vcpkg");

    let status = Command::new(&vcpkg_bin)
        .arg("install")
        .arg(format!("--triplet={}", detect_vcpkg_triplet()))
        .current_dir(project)
        .status()
        .context("failed to run vcpkg install")?;

    if !status.success() {
        anyhow::bail!("vcpkg install failed");
    }

    Ok(())
}
