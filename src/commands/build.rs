use anyhow::{Context, Result};
use serde_json::Value;
use std::collections::HashMap;
use std::env;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};

use crate::cmake::{detect_vcpkg_triplet, effective_cmake_version, regenerate_root_cmake, rewrite_component_cmake};
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

/// Windows-only helpers to load the MSVC env via VsDevCmd so Ninja+MSVC works from a plain shell.
#[cfg(windows)]
fn vsdevcmd_path() -> Option<PathBuf> {
    let vswhere = PathBuf::from(r"C:\Program Files (x86)\Microsoft Visual Studio\Installer\vswhere.exe");
    if !vswhere.exists() {
        return None;
    }
    let out = Command::new(&vswhere)
        .args([
            "-latest",
            "-products",
            "*",
            "-requires",
            "Microsoft.VisualStudio.Component.VC.Tools.x86.x64",
            "-property",
            "installationPath",
        ])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() {
        return None;
    }
    let mut p = PathBuf::from(s);
    p.push(r"Common7\Tools\VsDevCmd.bat");
    if p.exists() { Some(p) } else { None }
}

#[cfg(windows)]
fn win_norm(p: &Path) -> String {
    let mut s = p.to_string_lossy().to_string();
    if let Some(rest) = s.strip_prefix(r"\\?\") {
        s = rest.to_string();
    }
    s.replace('/', r"\")
}

#[cfg(windows)]
fn write_batch_and_run(
    cwd: &Path,
    vsdevcmd: &Path,
    prepend_path: Option<&Path>,
    commands: &[&str],
) -> Result<ExitStatus> {
    use std::fs::File;
    use std::io::Write;

    let tmp_dir = cwd.join(".triton").join("tmp");
    fs::create_dir_all(&tmp_dir)?;
    let bat_path = tmp_dir.join("run-msvc-env.cmd");

    let vs = win_norm(vsdevcmd);
    let mut bat = String::new();
    bat.push_str("@echo off\r\n");
    bat.push_str("setlocal\r\n");
    // Ensure Windows system dirs are in PATH (git-bash may strip them)
    bat.push_str("set PATH=%SystemRoot%\\System32;%SystemRoot%;%SystemRoot%\\System32\\Wbem;%SystemRoot%\\System32\\WindowsPowerShell\\v1.0;%PATH%\r\n");
    bat.push_str(&format!("call \"{}\" -arch=x64\r\n", vs));
    if let Some(p) = prepend_path {
        let pp = win_norm(p);
        bat.push_str(&format!("set PATH={};%PATH%\r\n", pp));
    }
    // Avoid inheriting random compilers
    bat.push_str("set CC=\r\nset CXX=\r\n");

    for c in commands {
        bat.push_str(c);
        bat.push_str("\r\n");
        bat.push_str("if errorlevel 1 exit /b %errorlevel%\r\n");
    }
    bat.push_str("endlocal\r\n");

    let mut f = File::create(&bat_path)?;
    f.write_all(bat.as_bytes())?;

    let status = Command::new("cmd")
        .arg("/C")
        .arg(win_norm(&bat_path))
        .current_dir(cwd)
        .status()
        .context("failed to run batch in MSVC env")?;
    Ok(status)
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
        let text = cmake_presets(&root.app_name, &root.generator, &detect_vcpkg_triplet(), cmake_ver);
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

        // Common pieces
        let toolchain_arg = format!(
            "-DCMAKE_TOOLCHAIN_FILE=\"{}\"",
            normalize_path(vcpkg_toolchain)
        );

        #[cfg(windows)]
        let using_ninja_on_windows = effective_gen.eq_ignore_ascii_case("ninja");
        #[cfg(not(windows))]
        let using_ninja_on_windows = false;

        if using_ninja_on_windows {
            // Build the commandline we want to run inside the VS dev environment
            let mut configure_line = format!("cmake --preset {}", preset);
            configure_line.push(' ');
            configure_line.push_str(&toolchain_arg);

            // Prefer MSVC `cl` when using the x64-windows triplet
            configure_line.push_str(" -DCMAKE_C_COMPILER=cl.exe -DCMAKE_CXX_COMPILER=cl.exe");

            // Add portable Ninja path and point CMake to it
            let mut prepend: Option<PathBuf> = None;
            if let Some(dir) = &ninja_abs_dir {
                prepend = Some(dir.clone());
                let ninja_bin = dir.join("ninja.exe");
                configure_line.push_str(&format!(" -DCMAKE_MAKE_PROGRAM=\"{}\"", win_norm(&ninja_bin)));
            }

            let vs = vsdevcmd_path().ok_or_else(|| {
                anyhow::anyhow!(
                    "Visual Studio Build Tools not found. Install 'Desktop development with C++' \
                     (or run in a Developer Command Prompt)."
                )
            })?;
            let status = write_batch_and_run(
                &components_dir,
                &vs,
                prepend.as_deref(),
                &[configure_line.as_str()],
            )?;
            if !status.success() {
                anyhow::bail!("cmake configure failed for preset {}", preset);
            }
        } else {
            // Non-Windows, or non-Ninja on Windows: run directly
            let mut cmd = Command::new("cmake");
            cmd.arg("--preset").arg(preset).current_dir(&components_dir);
            cmd.arg(toolchain_arg);
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
            // avoid leaking CC/CXX from environment
            cmd.env_remove("CC");
            cmd.env_remove("CXX");

            let status = cmd.status().context("cmake configure failed")?;
            if !status.success() {
                anyhow::bail!("cmake configure failed for preset {}", preset);
            }
        }
    }

    // --- Build ---
    #[cfg(windows)]
    let using_ninja_on_windows = effective_gen.eq_ignore_ascii_case("ninja");
    #[cfg(not(windows))]
    let using_ninja_on_windows = false;

    if using_ninja_on_windows {
        let mut prepend: Option<PathBuf> = None;
        if let Some(dir) = &ninja_abs_dir {
            prepend = Some(dir.clone());
        }
        let vs = vsdevcmd_path().ok_or_else(|| {
            anyhow::anyhow!(
                "Visual Studio Build Tools not found. Install 'Desktop development with C++' \
                 (or run in a Developer Command Prompt)."
            )
        })?;
        let build_line = format!("cmake --build --preset={}", preset);
        let status = write_batch_and_run(&components_dir, &vs, prepend.as_deref(), &[&build_line])?;
        if !status.success() {
            anyhow::bail!("build failed for preset {}", preset);
        }
    } else {
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
    }

    eprintln!("Built at {}", build_dir.display());
    Ok(())
}
