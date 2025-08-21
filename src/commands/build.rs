use anyhow::{Context, Result};
use serde_json::Value;
use std::collections::HashMap;
use std::env;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::cmake::{effective_cmake_version, regenerate_root_cmake, rewrite_component_cmake};
use crate::models::TritonRoot;
use crate::templates::cmake_presets;
use crate::tools::{ensure_ninja_dir, ensure_vcpkg};
use crate::util::{normalize_path, read_json};
use crate::commands::handle_install;

//
// === Helpers ===
//
pub fn normalize_config(cfg: &str) -> &'static str {
    match cfg.trim().to_ascii_lowercase().as_str() {
        "release" | "rel" | "r" => "release",
        "debug" | "dbg" | "d" => "debug",
        _ => "debug",
    }
}

pub fn preset_for(cfg: &str) -> &'static str {
    match normalize_config(cfg) {
        "release" => "release",
        _ => "debug",
    }
}

pub fn build_dir_for(project: &Path, cfg: &str) -> PathBuf {
    project.join("build").join(cfg)
}

/// Detect if CMake has already configured this build dir for the right generator.
pub fn is_configured_for_generator(build_dir: &Path, generator: &str) -> bool {
    let cache = build_dir.join("CMakeCache.txt");
    if !cache.exists() {
        return false;
    }
    let g = generator.to_ascii_lowercase();
    if g.contains("ninja") {
        return build_dir.join("build.ninja").exists();
    } else if g.contains("unix makefiles") {
        return build_dir.join("Makefile").exists();
    }
    true
}

pub fn load_presets(presets_dir: &Path) -> Result<(Value, HashMap<String, Value>)> {
    let mut s = String::new();
    File::open(presets_dir.join("CMakePresets.json"))?.read_to_string(&mut s)?;
    let v: Value = serde_json::from_str(&s)?;
    let mut map = HashMap::new();
    if let Some(arr) = v.get("configurePresets").and_then(|x| x.as_array()) {
        for p in arr {
            if let Some(name) = p.get("name").and_then(|n| n.as_str()) {
                map.insert(name.to_string(), p.clone());
            }
        }
    }
    Ok((v, map))
}

pub fn resolve_generator_for_preset(
    m: &HashMap<String, Value>,
    start: &str,
    guard: &mut Vec<String>,
) -> Option<String> {
    if guard.len() > 32 {
        return None;
    }
    guard.push(start.to_string());
    let p = m.get(start)?;
    if let Some(gen) = p.get("generator").and_then(|g| g.as_str()) {
        return Some(gen.to_string());
    }
    if let Some(inh) = p.get("inherits") {
        if let Some(s) = inh.as_str() {
            if !guard.contains(&s.to_string()) {
                if let Some(g) = resolve_generator_for_preset(m, s, guard) {
                    return Some(g);
                }
            }
        } else if let Some(arr) = inh.as_array() {
            for item in arr {
                if let Some(s) = item.as_str() {
                    if !guard.contains(&s.to_string()) {
                        if let Some(g) = resolve_generator_for_preset(m, s, guard) {
                            return Some(g);
                        }
                    }
                }
            }
        }
    }
    None
}
// === Main build entrypoint ===
pub fn handle_build(path: &str, config: &str, clean: bool, cleanf: bool) -> Result<()> {
    let project = PathBuf::from(path)
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from(path));
    let components_dir = project.join("components");

    let cfg = normalize_config(config);
    let preset = preset_for(cfg);
    let build_dir = build_dir_for(&project, cfg);
    let build_root = project.join("build");

    // --- CLEAN ---
    if clean || cleanf {
        if build_root.exists() {
            if cleanf {
                eprintln!("Force cleaning: {}", build_root.display());
                fs::remove_dir_all(&build_root)
                    .with_context(|| format!("removing {}", build_root.display()))?;
            } else {
                eprintln!("About to remove the build directory (ALL CONFIGS):");
                eprintln!("  {}", build_root.display());
                eprintln!("Proceed? [y/N]  (Ctrl+C to abort)");
                eprint!("> ");
                io::stdout().flush().ok();

                let mut line = String::new();
                if io::stdin().read_line(&mut line).is_ok() {
                    let ans = line.trim().to_ascii_lowercase();
                    if ans == "y" || ans == "yes" {
                        fs::remove_dir_all(&build_root)
                            .with_context(|| format!("removing {}", build_root.display()))?;
                        eprintln!("Removed {}", build_root.display());
                    } else {
                        eprintln!("Clean aborted; continuing.");
                    }
                } else {
                    eprintln!("(no input) Clean aborted; continuing.");
                }
            }
        }
    }

    // --- ensure vcpkg (returns toolchain file + exe path) ---
    let (vcpkg_toolchain, vcpkg_exe) = ensure_vcpkg(&project)?;

    // Load project model
    let root: TritonRoot = read_json(project.join("triton.json"))?;

    // --- Install packages ---
    handle_install(&root, &project, &vcpkg_exe)?;

    // Determine effective CMake version tuple (system if >= MIN, else MIN)
    let cmake_ver = effective_cmake_version();

    // Filter only existing components
    let existing: Vec<String> = root
        .components
        .keys()
        .filter(|name| components_dir.join(*name).is_dir())
        .cloned()
        .collect();

    // Filtered view for regeneration
    let mut root_filtered = root.clone();
    root_filtered
        .components
        .retain(|name, _| existing.iter().any(|n| n == name));

    regenerate_root_cmake(&root_filtered)?;
    for name in existing {
        if let Some(comp) = root.components.get(&name) {
            rewrite_component_cmake(&name, &root, comp, cmake_ver)?;
        }
    }

    // Ensure/refresh CMakePresets.json (write if missing)
    let presets_path = components_dir.join("CMakePresets.json");
    if !presets_path.exists() {
        let text = cmake_presets(&root.app_name, &root.generator, &root.triplet, cmake_ver);
        fs::write(&presets_path, text)?;
    }

    // Load presets
    let (_v, map) = load_presets(&components_dir)?;
    let mut guard = Vec::new();
    let effective_gen = resolve_generator_for_preset(&map, preset, &mut guard)
        .or_else(|| resolve_generator_for_preset(&map, "default", &mut guard))
        .unwrap_or_else(|| "Ninja".to_string());

    // Ninja bootstrap if needed
    let mut ninja_abs_dir: Option<PathBuf> = None;
    if effective_gen.eq_ignore_ascii_case("ninja") {
        ninja_abs_dir = Some(ensure_ninja_dir(&components_dir)?);
    }

    // --- Configure ---
    if !is_configured_for_generator(&build_dir, &effective_gen) {
        fs::create_dir_all(&build_dir)?;
        let mut cmd = Command::new("cmake");
        cmd.arg("--preset").arg(preset).current_dir(&components_dir);
        cmd.arg(format!("-DCMAKE_TOOLCHAIN_FILE={}", normalize_path(vcpkg_toolchain)));
        if let Some(dir) = &ninja_abs_dir {
            let existing = env::var_os("PATH").unwrap_or_default();
            let mut parts = env::split_paths(&existing).collect::<Vec<_>>();
            parts.insert(0, dir.clone());
            cmd.env("PATH", env::join_paths(parts).unwrap());
            let ninja_bin = if cfg!(windows) {
                dir.join("ninja.exe")
            } else {
                dir.join("ninja")
            };
            cmd.arg(format!("-DCMAKE_MAKE_PROGRAM={}", ninja_bin.display()));
        }
        cmd.env_remove("CC");
        cmd.env_remove("CXX");
        let status = cmd.status().context("cmake configure failed")?;

        if !status.success() {
            anyhow::bail!("cmake configure failed for preset {}", preset);
        }
    }

    // --- Build ---
    let mut b = Command::new("cmake");
    b.arg("--build")
        .arg(format!("--preset={}", preset))
        .current_dir(&components_dir);
    if let Some(dir) = &ninja_abs_dir {
        let existing = env::var_os("PATH").unwrap_or_default();
        let mut parts = env::split_paths(&existing).collect::<Vec<_>>();
        parts.insert(0, dir.clone());
        b.env("PATH", env::join_paths(parts).unwrap());
    }
    let status = b.status().context("cmake build failed")?;
    if !status.success() {
        anyhow::bail!("build failed for preset {}", preset);
    }

    eprintln!("Built at {}", build_dir.display());
    Ok(())
}

