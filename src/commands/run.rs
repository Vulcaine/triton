use anyhow::{Context, Result};
use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};
use walkdir::WalkDir;

use crate::cmake::detect_vcpkg_triplet_for_arch;
use crate::models::TritonRoot;
use crate::util::read_json;

use super::build::handle_build;

fn normalize_config(cfg: &str) -> &'static str {
    match cfg.to_ascii_lowercase().as_str() {
        "release" | "rel" => "release",
        _ => "debug",
    }
}

fn exe_name_for(component: &str) -> String {
    if cfg!(windows) { format!("{component}.exe") } else { component.to_string() }
}

fn component_triplet(project: &Path, component: Option<&str>) -> Result<String> {
    let root: TritonRoot = read_json(project.join("triton.json"))?;
    let arch = if let Some(name) = component {
        root.components
            .get(name)
            .and_then(|c| c.arch.as_deref())
            .map(|a| crate::cmake::normalize_target_arch(Some(a)))
            .transpose()?
            .unwrap_or(crate::cmake::host_default_arch())
    } else {
        crate::cmake::host_default_arch()
    };
    detect_vcpkg_triplet_for_arch(arch)
}

fn build_dir_for(project: &Path, cfg: &str, component: Option<&str>) -> Result<PathBuf> {
    let triplet = component_triplet(project, component)?;
    Ok(super::build::build_dir_for_triplet(project, cfg, &triplet))
}

fn runtime_search_dirs(project: &Path, cfg: &str, exe_dir: &Path, component: Option<&str>) -> Result<Vec<PathBuf>> {
    let triplet = component_triplet(project, component)?;
    let build_dir = build_dir_for(project, cfg, component)?;
    let mut dirs = vec![exe_dir.to_path_buf()];
    if cfg.eq_ignore_ascii_case("debug") {
        dirs.push(build_dir.join("vcpkg_installed").join(&triplet).join("debug").join("bin"));
    }
    dirs.push(build_dir.join("vcpkg_installed").join(&triplet).join("bin"));
    Ok(dirs)
}

fn prepend_existing_path_dirs(dirs: &[PathBuf]) -> Result<OsString> {
    let existing = env::var_os("PATH").unwrap_or_default();
    let mut parts = env::split_paths(&existing).collect::<Vec<_>>();
    for dir in dirs.iter().rev() {
        if dir.is_dir() && !parts.iter().any(|p| p == dir) {
            parts.insert(0, dir.clone());
        }
    }
    env::join_paths(parts).context("failed to compose PATH for triton run")
}

fn cmake_cache_exists(project: &Path, cfg: &str, component: Option<&str>) -> bool {
    build_dir_for(project, cfg, component).map(|d| d.join("CMakeCache.txt").exists()).unwrap_or(false)
}

fn newest_mtime_in_dirs(dirs: &[&Path], exts: &[&str]) -> SystemTime {
    let mut newest = UNIX_EPOCH;
    for d in dirs {
        if !d.exists() { continue; }
        for entry in WalkDir::new(d).into_iter().filter_map(|e| e.ok()) {
            let p = entry.path();
            if p.is_file() {
                if let Some(ext) = p.extension().and_then(|s| s.to_str()) {
                    if exts.iter().any(|e| e.eq_ignore_ascii_case(ext)) {
                        if let Ok(meta) = fs::metadata(p) {
                            if let Ok(mt) = meta.modified() {
                                if mt > newest { newest = mt; }
                            }
                        }
                    }
                }
            }
        }
    }
    newest
}

fn newest_project_input_mtime(project: &Path) -> SystemTime {
    // consider source/headers + key config files that trigger rebuild/regenerate
    let src_exts = ["c","cc","cxx","cpp","hpp","hh","h","hxx","inl","ixx"];

    let comp_dir = project.join("components");
    let include_dir = project.join("include");
    let src_dir = project.join("src");

    let dirs: [&Path; 3] = [
        comp_dir.as_path(),
        include_dir.as_path(),
        src_dir.as_path(),
    ];
    let newest_sources = newest_mtime_in_dirs(&dirs, &src_exts);

    let important_files = [
        project.join("components/CMakeLists.txt"),
        project.join("components/CMakePresets.json"),
        project.join("triton.json"),
        project.join("vcpkg.json"),
    ];
    let mut newest = newest_sources;
    for f in &important_files {
        if let Ok(meta) = fs::metadata(f) {
            if let Ok(mt) = meta.modified() {
                if mt > newest { newest = mt; }
            }
        }
    }
    newest
}

fn find_executable(project: &Path, cfg: &str, component: Option<&str>) -> Result<PathBuf> {
    let build_dir = build_dir_for(project, cfg, component)?;
    let want = if let Some(c) = component {
        exe_name_for(c)
    } else {
        let root: TritonRoot = read_json(project.join("triton.json"))?;
        if let Some((name, _comp)) = root.components.iter().find(|(_, c)| c.kind == "exe") {
            exe_name_for(name)
        } else {
            exe_name_for("app")
        }
    };

    for entry in WalkDir::new(&build_dir).follow_links(true).into_iter().filter_map(|e| e.ok()) {
        let p = entry.path();
        if p.is_file() {
            if let Some(fname) = p.file_name().and_then(|s| s.to_str()) {
                if fname == want {
                    return Ok(p.to_path_buf());
                }
            }
        }
    }
    anyhow::bail!("executable '{}' not found under {}", want, build_dir.display());
}

pub fn handle_run(path: &str, component: Option<&str>, config: &str, args: &[String]) -> Result<()> {
    // If `path` matches a script name in triton.json, delegate to the script runner.
    if let Ok(cwd) = std::env::current_dir() {
        if let Ok(root) = read_json::<_, TritonRoot>(cwd.join("triton.json")) {
            if root.scripts.contains_key(path) {
                let mut tokens = vec![path.to_string()];
                tokens.extend_from_slice(args);
                return super::script::handle_script(&tokens);
            }
        }
    }

    let project = PathBuf::from(path).canonicalize().unwrap_or_else(|_| PathBuf::from(path));
    let cfg = normalize_config(config);

    // Fast path: if exe exists and is newer than inputs, and cache exists -> skip build
    let need_build = {
        let cache_ok = cmake_cache_exists(&project, cfg, component);
        let exe_path_guess = find_executable(&project, cfg, component).ok();
        match (cache_ok, exe_path_guess) {
            (true, Some(exe)) => {
                let exe_mt = fs::metadata(&exe).and_then(|m| m.modified()).unwrap_or(UNIX_EPOCH);
                let newest_in = newest_project_input_mtime(&project);
                newest_in > exe_mt
            }
            _ => true,
        }
    };

    if need_build {
        handle_build(&project.display().to_string(), component, cfg, None, false, false)?;
    }

    // Run (re-find exe in case we just built it)
    let exe_path = find_executable(&project, cfg, component)?;
    eprintln!("Running {} â€¦", exe_path.display());
    let exe_dir = exe_path.parent().unwrap_or(&project);
    let run_path = prepend_existing_path_dirs(&runtime_search_dirs(&project, cfg, exe_dir, component)?)?;
    let status = Command::new(&exe_path)
        .args(args)
        .current_dir(exe_dir)
        .env("PATH", run_path)
        .status()
        .with_context(|| format!("failed to launch {}", exe_path.display()))?;
    if !status.success() {
        anyhow::bail!("program exited with status {:?}", status.code());
    }
    Ok(())
}




