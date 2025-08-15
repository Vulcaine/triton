use anyhow::{Context, Result};
use std::path::{Path};
use std::process::Command;

use crate::commands::build::{
    build_dir_for, is_configured_for_generator, handle_build, load_presets, preset_for,
    resolve_generator_for_preset,
};

fn ctest_metadata_present(build_dir: &Path) -> bool {
    // Presence of CTest metadata indicates tests were discovered during configure/build.
    // CMake typically writes:
    // - CTestTestfile.cmake (top-level)
    // - Testing/TAG (once testing is enabled)
    build_dir.join("CTestTestfile.cmake").exists()
        || build_dir.join("Testing").join("TAG").exists()
}

pub fn handle_test(path: &str, config: &str) -> Result<()> {
    if std::env::var("TRITON_TEST_MODE").is_ok() {
        eprintln!("TRITON_TEST_MODE set — skipping actual build and tests");
        return Ok(());
    }

    let cfg = config.trim();
    let build_dir = build_dir_for(Path::new(path), cfg);

    let needs_build = !build_dir.exists()
        || !ctest_metadata_present(&build_dir)
        || {
            let preset = preset_for(cfg);
            let components_dir = Path::new(path).join("components");
            let (_v, map) = load_presets(&components_dir)?;
            let mut guard = Vec::new();
            let effective_gen = resolve_generator_for_preset(&map, preset, &mut guard)
                .or_else(|| resolve_generator_for_preset(&map, "default", &mut guard))
                .unwrap_or_else(|| "Ninja".to_string());
            !is_configured_for_generator(&build_dir, &effective_gen)
        };

    if needs_build {
        eprintln!(
            "No usable build with tests for config '{}', running `triton build` first…",
            cfg
        );
        handle_build(path, cfg, false, false)?;
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
