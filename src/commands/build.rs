use anyhow::{Context, Result};
use serde_json::Value;
use std::collections::HashMap;
use std::env;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};

use crate::cmake::{regenerate_root_cmake, rewrite_component_cmake};
use crate::models::TritonRoot;
use crate::templates::cmake_presets;
use crate::tools::ensure_ninja_dir;
use crate::util::read_json;

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
    project.join(format!("build/{}", cfg))
}

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

#[cfg(windows)]
fn vsdevcmd_path() -> Option<PathBuf> {
    let vswhere =
        PathBuf::from(r"C:\Program Files (x86)\Microsoft Visual Studio\Installer\vswhere.exe");
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
    if p.exists() {
        Some(p)
    } else {
        None
    }
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
    bat.push_str(&format!("call \"{}\" -arch=x64\r\n", vs));
    if let Some(p) = prepend_path {
        let pp = win_norm(p);
        bat.push_str(&format!("set PATH={};%PATH%\r\n", pp));
    }
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

pub fn handle_build(path: &str, config: &str, clean: bool, cleanf: bool) -> Result<()> {
    // Repo root
    let project = PathBuf::from(path)
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from(path));
    let components_dir = project.join("components");

    let cfg = normalize_config(config);
    let preset = preset_for(cfg);
    let build_dir = build_dir_for(&project, cfg);
    let build_root = project.join("build"); // <-- clean the whole build/ tree

    // --clean / --cleanf (operate on the entire build/ directory)
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
                        eprintln!("Clean aborted; continuing without deleting.");
                    }
                } else {
                    eprintln!("(no input) Clean aborted; continuing without deleting.");
                }
            }
        } else {
            eprintln!("Nothing to clean ({} does not exist).", build_root.display());
        }
    }

    // (Re)generate CMake files from triton.json every build
    let root: TritonRoot = read_json(project.join("triton.json"))?;
    regenerate_root_cmake(&root)?; // writes components/CMakeLists.txt
    for (name, comp) in &root.components {
        rewrite_component_cmake(name, &root, comp)?; // writes components/<name>/CMakeLists.txt
    }

    // Ensure components/CMakePresets.json exists (create if missing)
    let presets_path = components_dir.join("CMakePresets.json");
    if !presets_path.exists() {
        let text = cmake_presets(&root.app_name, &root.generator, &root.triplet);
        fs::write(&presets_path, text)
            .with_context(|| format!("writing {}", presets_path.display()))?;
    }

    // Load presets to figure out generator / env
    let (_v, map) = load_presets(&components_dir)?;
    let mut guard = Vec::new();
    let effective_gen = resolve_generator_for_preset(&map, preset, &mut guard)
        .or_else(|| resolve_generator_for_preset(&map, "default", &mut guard))
        .unwrap_or_else(|| "Ninja".to_string());

    // Portable Ninja if needed (next to components/)
    let mut ninja_abs_dir: Option<PathBuf> = None;
    if effective_gen.eq_ignore_ascii_case("ninja") {
        let dir = ensure_ninja_dir(&components_dir)?;
        ninja_abs_dir = Some(dir);
    }

    // Configure if not configured for this generator
    if !is_configured_for_generator(&build_dir, &effective_gen) {
        fs::create_dir_all(&build_dir)?;
        let mut configure_line = format!("cmake --preset {}", preset);
        if let Some(dir) = &ninja_abs_dir {
            let ninja_bin = if cfg!(windows) {
                dir.join("ninja.exe")
            } else {
                dir.join("ninja")
            };
            configure_line.push_str(&format!(" -DCMAKE_MAKE_PROGRAM=\"{}\"", ninja_bin.display()));
        }

        #[cfg(windows)]
        let using_ninja_on_windows = effective_gen.eq_ignore_ascii_case("ninja");
        #[cfg(not(windows))]
        let using_ninja_on_windows = false;

        #[cfg(windows)]
        if using_ninja_on_windows {
            // Ensure MSVC cl.exe is used with Ninja
            configure_line.push_str(" -DCMAKE_C_COMPILER=cl.exe -DCMAKE_CXX_COMPILER=cl.exe");
        }

        let status = if using_ninja_on_windows {
            let vs = vsdevcmd_path().ok_or_else(|| {
                anyhow::anyhow!(
                    "Visual Studio Build Tools not found. Install 'Desktop development with C++' \
                     (or run in a Developer Command Prompt)."
                )
            })?;
            write_batch_and_run(&components_dir, &vs, ninja_abs_dir.as_deref(), &[configure_line.as_str()])?
        } else {
            let mut cmd = Command::new("cmake");
            cmd.arg("--preset").arg(preset).current_dir(&components_dir);
            if let Some(dir) = &ninja_abs_dir {
                let existing = env::var_os("PATH").unwrap_or_default();
                let mut parts = env::split_paths(&existing).collect::<Vec<_>>();
                parts.insert(0, dir.clone());
                cmd.env("PATH", env::join_paths(parts).expect("join PATH"));
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
            cmd.status().context("failed to run cmake --preset (configure)")?
        };

        if !status.success() {
            anyhow::bail!("cmake configure failed for preset {}", preset);
        }
    }

    // Build
    let status = if cfg!(windows) && effective_gen.eq_ignore_ascii_case("ninja") {
        let vs = vsdevcmd_path().ok_or_else(|| {
            anyhow::anyhow!(
                "Visual Studio Build Tools not found. Install 'Desktop development with C++' \
                 (or run in a Developer Command Prompt)."
            )
        })?;
        write_batch_and_run(
            &components_dir,
            &vs,
            ninja_abs_dir.as_deref(),
            &[&format!("cmake --build --preset={}", preset)],
        )?
    } else {
        let mut b = Command::new("cmake");
        b.arg("--build")
            .arg(format!("--preset={}", preset))
            .current_dir(&components_dir);
        if let Some(dir) = &ninja_abs_dir {
            let existing = env::var_os("PATH").unwrap_or_default();
            let mut parts = env::split_paths(&existing).collect::<Vec<_>>();
            parts.insert(0, dir.clone());
            b.env("PATH", env::join_paths(parts).expect("join PATH"));
        }
        b.status().context("failed to run cmake --build")?
    };

    if !status.success() {
        anyhow::bail!("build failed for preset {}", preset);
    }

    eprintln!("Built at {}", build_dir.display());
    Ok(())
}
