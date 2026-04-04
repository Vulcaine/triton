use anyhow::{Context, Result};
use serde_json::Value;
use std::collections::HashMap;
use std::env;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::ffi::OsString;
use std::process::{Command, ExitStatus};
use walkdir::WalkDir;

use crate::cmake::{
    arch_label_for_triplet, detect_vcpkg_triplet, detect_vcpkg_triplet_for_arch,
    effective_cmake_version, regenerate_root_cmake,
    rewrite_component_cmake,
};
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
    let triplet = detect_vcpkg_triplet();
    build_dir_for_triplet(project, cfg, &triplet)
}

pub fn build_dir_for_triplet(project: &Path, cfg: &str, triplet: &str) -> PathBuf {
    project.join("build").join(arch_label_for_triplet(triplet)).join(cfg)
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

fn cache_has_nonempty_value(build_dir: &Path, key: &str) -> bool {
    let cache = build_dir.join("CMakeCache.txt");
    let Ok(text) = fs::read_to_string(cache) else {
        return false;
    };

    let prefix = format!("{key}:");
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix(&prefix) {
            if let Some((_, value)) = rest.split_once('=') {
                return !value.trim().is_empty();
            }
        }
    }
    false
}

fn build_tree_has_valid_compiler_id(build_dir: &Path) -> bool {
    cache_has_nonempty_value(build_dir, "CMAKE_C_COMPILER_ID")
        || cache_has_nonempty_value(build_dir, "CMAKE_CXX_COMPILER_ID")
}

fn clear_configure_state(build_dir: &Path) -> Result<()> {
    let cache = build_dir.join("CMakeCache.txt");
    if cache.exists() {
        fs::remove_file(&cache)
            .with_context(|| format!("removing {}", cache.display()))?;
    }

    let cmake_files = build_dir.join("CMakeFiles");
    if cmake_files.exists() {
        fs::remove_dir_all(&cmake_files)
            .with_context(|| format!("removing {}", cmake_files.display()))?;
    }
    Ok(())
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
fn msvc_cl_path(vsdevcmd: &Path, target_arch: &str) -> Option<PathBuf> {
    let install_root = vsdevcmd.parent()?.parent()?.parent()?.to_path_buf();
    let msvc_root = install_root.join(r"VC\Tools\MSVC");
    if !msvc_root.is_dir() {
        return None;
    }

    let relative = match target_arch {
        "x86" => r"bin\Hostx64\x86\cl.exe",
        "arm64" => r"bin\Hostx64\arm64\cl.exe",
        _ => r"bin\Hostx64\x64\cl.exe",
    };

    let mut versions: Vec<PathBuf> = fs::read_dir(&msvc_root)
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    versions.sort();
    versions.reverse();

    for version in versions {
        let candidate = version.join(relative);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
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
    target_arch: &str,
    prepend_path: Option<&str>,
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
    bat.push_str(&format!("call \"{}\" -arch={}\r\n", vs, target_arch));
    if let Some(p) = prepend_path {
        bat.push_str(&format!("set PATH={};%PATH%\r\n", p));
    }
    // Avoid inheriting random compilers
    bat.push_str("set CC=\r\nset CXX=\r\n");
    bat.push_str("set VCPKG_FORCE_SYSTEM_BINARIES=1\r\n");

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

// === Extracted helpers ===

fn handle_clean(build_root: &Path, clean: bool, cleanf: bool) -> Result<()> {
    if clean || cleanf {
        if build_root.exists() {
            if cleanf {
                eprintln!("Force cleaning: {}", build_root.display());
                fs::remove_dir_all(build_root)
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
                        fs::remove_dir_all(build_root)
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
    Ok(())
}

fn find_cmake_in_vcpkg(project: &Path) -> Option<PathBuf> {
    let tools_dir = project.join("vcpkg").join("downloads").join("tools");
    if !tools_dir.is_dir() {
        return None;
    }

    let want_name = if cfg!(windows) { "cmake.exe" } else { "cmake" };
    let mut matches: Vec<PathBuf> = WalkDir::new(&tools_dir)
        .follow_links(true)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .filter(|e| e.file_name().to_string_lossy().eq_ignore_ascii_case(want_name))
        .map(|e| e.path().to_path_buf())
        .collect();

    matches.sort();
    matches.pop()
}

fn resolve_cmake_executable(project: &Path) -> PathBuf {
    find_cmake_in_vcpkg(project).unwrap_or_else(|| PathBuf::from("cmake"))
}

fn find_tool_dir(project: &Path, tool_name: &str) -> Option<PathBuf> {
    let tools_dir = project.join("vcpkg").join("downloads").join("tools");
    if !tools_dir.is_dir() {
        return None;
    }

    WalkDir::new(&tools_dir)
        .follow_links(true)
        .into_iter()
        .filter_map(|e| e.ok())
        .find(|e| e.file_type().is_file() && e.file_name().to_string_lossy().eq_ignore_ascii_case(tool_name))
        .and_then(|e| e.path().parent().map(|p| p.to_path_buf()))
}

fn build_process_path(project: &Path, ninja_abs_dir: Option<&Path>) -> Option<OsString> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Some(dir) = ninja_abs_dir {
        dirs.push(dir.to_path_buf());
    }
    if cfg!(windows) {
        if let Some(dir) = find_tool_dir(project, "cmake.exe") {
            dirs.push(dir);
        }
        if let Some(dir) = find_tool_dir(project, "7z.exe") {
            dirs.push(dir);
        }
    } else {
        if let Some(dir) = find_tool_dir(project, "cmake") {
            dirs.push(dir);
        }
        if let Some(dir) = find_tool_dir(project, "7z") {
            dirs.push(dir);
        }
    }

    if dirs.is_empty() {
        return None;
    }

    let existing = env::var_os("PATH").unwrap_or_default();
    let mut parts = env::split_paths(&existing).collect::<Vec<_>>();
    for dir in dirs.into_iter().rev() {
        if !parts.iter().any(|p| p == &dir) {
            parts.insert(0, dir);
        }
    }
    env::join_paths(parts).ok()
}

fn run_cmake_configure(
    cmake_exe: &Path,
    project: &Path,
    components_dir: &Path,
    preset: &str,
    toolchain_path: &Path,
    ninja_abs_dir: Option<&Path>,
    using_ninja_on_windows: bool,
    skip_manifest_install: bool,
    target_arch: &str,
) -> Result<()> {
    let toolchain_arg = format!(
        "-DCMAKE_TOOLCHAIN_FILE=\"{}\"",
        normalize_path(toolchain_path.to_path_buf())
    );

    if using_ninja_on_windows {
        #[cfg(windows)]
        {
            let mut configure_line = format!("\"{}\" --preset {}", win_norm(cmake_exe), preset);
            configure_line.push(' ');
            configure_line.push_str(&toolchain_arg);

            // Prefer MSVC `cl` when using the x64-windows triplet
            if skip_manifest_install {
                configure_line.push_str(" -DVCPKG_MANIFEST_INSTALL=OFF");
            }

            // Add portable Ninja path and point CMake to it
            let process_path = build_process_path(project, ninja_abs_dir);
            if let Some(dir) = ninja_abs_dir {
                let ninja_bin = dir.join("ninja.exe");
                configure_line.push_str(&format!(" -DCMAKE_MAKE_PROGRAM=\"{}\"", win_norm(&ninja_bin)));
            }

            let vs = vsdevcmd_path().ok_or_else(|| {
                anyhow::anyhow!(
                    "Visual Studio Build Tools not found. Install 'Desktop development with C++' \
                     (or run in a Developer Command Prompt)."
                )
            })?;
            let cl = msvc_cl_path(&vs, target_arch).ok_or_else(|| anyhow::anyhow!("could not locate VS 2022 cl.exe"))?;
            configure_line.push_str(&format!(" -DCMAKE_C_COMPILER=\"{}\" -DCMAKE_CXX_COMPILER=\"{}\"", win_norm(&cl), win_norm(&cl)));
            let status = write_batch_and_run(
                components_dir,
                &vs,
                target_arch,
                process_path.as_ref().and_then(|p| p.to_str()),
                &[configure_line.as_str()],
            )?;
            if !status.success() {
                anyhow::bail!("cmake configure failed for preset {}", preset);
            }
        }
    } else {
        // Non-Windows, or non-Ninja on Windows: run directly
        let mut cmd = Command::new(cmake_exe);
        cmd.arg("--preset").arg(preset).current_dir(components_dir);
        cmd.arg(toolchain_arg);
        if skip_manifest_install {
            cmd.arg("-DVCPKG_MANIFEST_INSTALL=OFF");
        }
        if let Some(path) = build_process_path(project, ninja_abs_dir) {
            cmd.env("PATH", path);
        }
        if let Some(dir) = ninja_abs_dir {
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
        cmd.env("VCPKG_FORCE_SYSTEM_BINARIES", "1");

        let status = cmd.status().context("cmake configure failed")?;
        if !status.success() {
            anyhow::bail!("cmake configure failed for preset {}", preset);
        }
    }
    Ok(())
}

fn run_cmake_build(
    cmake_exe: &Path,
    project: &Path,
    components_dir: &Path,
    preset: &str,
    target: Option<&str>,
    ninja_abs_dir: Option<&Path>,
    using_ninja_on_windows: bool,
    target_arch: &str,
) -> Result<()> {
    if using_ninja_on_windows {
        #[cfg(windows)]
        {
            let process_path = build_process_path(project, ninja_abs_dir);
            let vs = vsdevcmd_path().ok_or_else(|| {
                anyhow::anyhow!(
                    "Visual Studio Build Tools not found. Install 'Desktop development with C++' \
                     (or run in a Developer Command Prompt)."
                )
            })?;
            let mut build_line = format!("\"{}\" --build --preset={}", win_norm(cmake_exe), preset);
            if let Some(target) = target {
                build_line.push_str(" --target ");
                build_line.push_str(target);
            }
            let build_line_str = build_line.as_str();
            let status = write_batch_and_run(
                components_dir,
                &vs,
                target_arch,
                process_path.as_ref().and_then(|p| p.to_str()),
                &[build_line_str],
            )?;
            if !status.success() {
                anyhow::bail!("build failed for preset {}", preset);
            }
        }
    } else {
        let mut b = Command::new(cmake_exe);
        b.arg("--build")
            .arg(format!("--preset={}", preset))
            .current_dir(components_dir);
        if let Some(target) = target {
            b.arg("--target").arg(target);
        }
        if let Some(path) = build_process_path(project, ninja_abs_dir) {
            b.env("PATH", path);
        }
        b.env("VCPKG_FORCE_SYSTEM_BINARIES", "1");
        let status = b.status().context("cmake build failed")?;
        if !status.success() {
            anyhow::bail!("build failed for preset {}", preset);
        }
    }
    Ok(())
}

fn has_installed_vcpkg_state(project: &Path, triplet: &str) -> bool {
    project
        .join("vcpkg_installed")
        .join(triplet)
        .join("share")
        .is_dir()
}

fn build_tree_has_target(build_dir: &Path, target: &str) -> bool {
    let ninja = build_dir.join("build.ninja");
    if !ninja.is_file() {
        return false;
    }

    std::fs::read_to_string(&ninja)
        .map(|text| text.contains(target))
        .unwrap_or(false)
}

fn run_pre_build_scripts(root: &TritonRoot, project: &Path) -> Result<()> {
    if root.scripts.contains_key("pre_build") {
        eprintln!("Running pre_build script...");
        let _prev = env::current_dir();
        env::set_current_dir(project)?;
        crate::commands::handle_script(&["pre_build".to_string()])?;
        if let Ok(prev) = _prev {
            env::set_current_dir(prev)?;
        }
    }
    Ok(())
}

// === Main build entrypoint ===
pub fn handle_build(path: &str, component: Option<&str>, config: &str, clean: bool, cleanf: bool) -> Result<()> {
    handle_build_with_arch(path, component, None, config, clean, cleanf)
}

pub fn handle_build_with_arch(path: &str, component: Option<&str>, arch: Option<&str>, config: &str, clean: bool, cleanf: bool) -> Result<()> {
    let project = PathBuf::from(path)
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from(path));
    let components_dir = project.join("components");

    // Resolve triplet once from arch (or use host default)
    let triplet = match arch {
        Some(a) => detect_vcpkg_triplet_for_arch(a)?,
        None => detect_vcpkg_triplet(),
    };
    let target_arch = arch_label_for_triplet(&triplet);

    let cfg = normalize_config(config);
    let preset = preset_for(cfg);
    let build_dir = build_dir_for_triplet(&project, cfg, &triplet);
    let build_root = project.join("build");
    let cmake_exe = resolve_cmake_executable(&project);

    handle_clean(&build_root, clean, cleanf)?;

    let (vcpkg_toolchain, vcpkg_exe) = ensure_vcpkg(&project)?;
    let root: TritonRoot = read_json(project.join("triton.json"))?;
    let should_install = component.is_none() || !has_installed_vcpkg_state(&project, &triplet);
    let reuse_installed_vcpkg = !should_install;
    if should_install {
        handle_install(&root, &project, &vcpkg_exe, &triplet)?;
    } else {
        eprintln!("Using existing vcpkg install for component build '{}'.", component.unwrap());
    }

    if let Some(component_name) = component {
        let component_exists = root.components.contains_key(component_name)
            && components_dir.join(component_name).is_dir();
        if !component_exists {
            anyhow::bail!("component '{}' was not found in triton.json/components", component_name);
        }
    }

    let cmake_ver = effective_cmake_version();
    let existing: Vec<String> = root
        .components
        .keys()
        .filter(|name| components_dir.join(*name).is_dir())
        .cloned()
        .collect();

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

    let presets_path = components_dir.join("CMakePresets.json");
    if !presets_path.exists() {
        let text = cmake_presets(&root.app_name, &root.generator, &triplet, cmake_ver);
        fs::write(&presets_path, text)?;
    }

    let (_v, map) = load_presets(&components_dir)?;
    let mut guard = Vec::new();
    let effective_gen = resolve_generator_for_preset(&map, preset, &mut guard)
        .or_else(|| resolve_generator_for_preset(&map, "default", &mut guard))
        .unwrap_or_else(|| "Ninja".to_string());

    let mut ninja_abs_dir: Option<PathBuf> = None;
    if effective_gen.eq_ignore_ascii_case("ninja") {
        ninja_abs_dir = Some(ensure_ninja_dir(&components_dir)?);
    }

    #[cfg(windows)]
    let using_ninja_on_windows = effective_gen.eq_ignore_ascii_case("ninja");
    #[cfg(not(windows))]
    let using_ninja_on_windows = false;

    let malformed_cache = build_dir.join("CMakeCache.txt").exists() && !build_tree_has_valid_compiler_id(&build_dir);
    let configure_needed = !is_configured_for_generator(&build_dir, &effective_gen)
        || component.is_some_and(|target| !build_tree_has_target(&build_dir, target))
        || malformed_cache;
    if configure_needed {
        if malformed_cache {
            eprintln!("Detected incomplete CMake cache in {}. Reconfiguring from a clean cache.", build_dir.display());
            clear_configure_state(&build_dir)?;
        }
        fs::create_dir_all(&build_dir)?;
        run_cmake_configure(
            &cmake_exe,
            &project,
            &components_dir,
            preset,
            Path::new(&vcpkg_toolchain),
            ninja_abs_dir.as_deref(),
            using_ninja_on_windows,
            reuse_installed_vcpkg,
            target_arch,
        )?;
    }

    if component.is_none() {
        run_pre_build_scripts(&root, &project)?;
    }

    run_cmake_build(
        &cmake_exe,
        &project,
        &components_dir,
        preset,
        component,
        ninja_abs_dir.as_deref(),
        using_ninja_on_windows,
        target_arch,
    )?;

    eprintln!("Built at {}", build_dir.display());
    Ok(())
}







