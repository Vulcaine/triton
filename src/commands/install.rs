use anyhow::{Context, Result};
use std::path::Path;
use std::process::Command;

use crate::models::TritonRoot;

/// Install all vcpkg deps valid for current host.
pub fn handle_install(root: &TritonRoot, project: &Path) -> Result<()> {
    // ensure vcpkg.json is up-to-date
    crate::commands::handle_generate()?;  

    let triplet = &root.triplet;
    eprintln!("Running vcpkg install with manifest mode...");

    let status = Command::new("vcpkg")
        .arg("install")
        .arg(format!("--triplet={}", triplet))
        .current_dir(project)
        .status()
        .context("failed to run vcpkg install")?;

    if !status.success() {
        anyhow::bail!("vcpkg install failed");
    }

    Ok(())
}
