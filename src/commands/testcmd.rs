// commands/testcmd.rs
use anyhow::{Context, Result};
use std::path::Path;
use std::process::Command;

pub fn handle_test(path: &str, config: &str) -> Result<()> {
    let build_dir = Path::new(path).join("build").join(config);
    if !build_dir.exists() {
        anyhow::bail!(
            "Build directory {} does not exist. Did you run `triton build`?",
            build_dir.display()
        );
    }

    let status = Command::new("ctest")
        .current_dir(&build_dir)
        .arg("--output-on-failure")
        .status()
        .context("failed to run ctest")?;

    if !status.success() {
        anyhow::bail!("Some tests failed");
    }

    Ok(())
}
