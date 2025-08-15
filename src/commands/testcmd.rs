use anyhow::{Context, Result};
use std::path::Path;
use std::process::Command;

use crate::commands::build::{
    build_dir_for, is_configured_for_generator, handle_build, load_presets, preset_for,
    resolve_generator_for_preset,
};

/// Heuristic to detect whether CTest discovered any tests in this build dir.
fn ctest_metadata_present(build_dir: &Path) -> bool {
    build_dir.join("CTestTestfile.cmake").exists()
        || build_dir.join("Testing").join("TAG").exists()
}

pub fn handle_test(path: &str, config: &str) -> Result<()> {
    // In unit tests, we allow skipping the heavy work.
    if std::env::var("TRITON_TEST_MODE").is_ok() {
        eprintln!("TRITON_TEST_MODE set — skipping actual build and tests");
        return Ok(());
    }

    let cfg = config.trim();
    let build_dir = build_dir_for(Path::new(path), cfg);

    // Decide if we need to (re)build first.
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

    // Default: only run our project test added by the template: `add_test(NAME all_tests ...)`.
    // You can override with:
    //   TRITON_CTEST_FILTER="^my_tests$"   (regex)
    //   TRITON_CTEST_LABEL="triton"        (ctest label)
    let label = std::env::var("TRITON_CTEST_LABEL").ok();
    let filter = std::env::var("TRITON_CTEST_FILTER").unwrap_or_else(|_| String::from("^all_tests$"));

    let mut cmd = Command::new("ctest");
    cmd.current_dir(&build_dir)
        .arg("--output-on-failure");

    // On multi-config generators, direct ctest to the right config.
    // (Ignored on single-config generators like Ninja.)
    cmd.arg("-C").arg(match cfg.to_ascii_lowercase().as_str() {
        "release" | "rel" | "r" => "Release",
        _ => "Debug",
    });

    if let Some(lbl) = label {
        cmd.arg("-L").arg(lbl);
    } else {
        cmd.arg("-R").arg(filter);
    }

    let status = cmd.status().context("failed to run ctest")?;
    if !status.success() {
        anyhow::bail!("Some tests failed");
    }

    Ok(())
}
