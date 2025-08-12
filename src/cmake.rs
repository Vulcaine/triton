use anyhow::{Context, Result};
use regex::Regex;
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

use crate::models::{TritonComponent, TritonRoot};
use crate::util::read_json;

/// Replace the `# ## triton:components ...` block at repo root.
pub fn regenerate_root_cmake(root: &TritonRoot) -> Result<()> {
    // discover existing component dirs (on disk)
    let mut existing: Vec<String> = Vec::new();
    if Path::new("components").exists() {
        for entry in WalkDir::new("components").min_depth(1).max_depth(1) {
            let e = entry?;
            if e.file_type().is_dir() {
                existing.push(e.file_name().to_string_lossy().into_owned());
            }
        }
    }
    existing.sort();

    let mut cmake = fs::read_to_string("CMakeLists.txt")?;
    let mut lines = String::new();
    for c in &existing {
        lines.push_str(&format!("add_subdirectory(components/{})\n", c));
    }
    if lines.is_empty() {
        lines = "# (no components)".into();
    }

    let re = Regex::new(r"(?s)# ## triton:components begin.*?# ## triton:components end").unwrap();
    let replacement = format!("# ## triton:components begin\n{}\n# ## triton:components end", lines);
    if re.is_match(&cmake) {
        cmake = re.replace(&cmake, replacement.as_str()).to_string();
    } else {
        cmake.push_str("\n");
        cmake.push_str(&replacement);
        cmake.push_str("\n");
    }
    fs::write("CMakeLists.txt", cmake)?;
    Ok(())
}

pub fn rewrite_component_cmake(name: &str, comp: &TritonComponent) -> Result<()> {
    let p = format!("components/{name}/CMakeLists.txt");
    let mut cmake = fs::read_to_string(&p).with_context(|| format!("reading {}", p))?;

    // Load root to know triplet and vcpkg installed prefix
    let root: TritonRoot = read_json("triton.json")?;
    let vcpkg_inst = Path::new("vcpkg_installed").join(&root.triplet);
    let share_dir = vcpkg_inst.join("share");
    let lib_dir = vcpkg_inst.join("lib");

    let mut lines: Vec<String> = Vec::new();

    // 1) Inter-component deps
    for c in &comp.comps {
        lines.push(format!("target_link_libraries(${{_comp_name}} PRIVATE {})", c));
    }

    // 2) vcpkg package deps
    for pkg in &comp.deps {
        let pkg_lc = pkg.to_string();
        let config_exists = has_config_package(&share_dir, pkg);
        if config_exists {
            lines.push(format!("find_package({} CONFIG REQUIRED)", pkg_lc));
            lines.push(format!(
                "target_link_libraries(${{_comp_name}} PRIVATE {}::{} )",
                pkg_lc, pkg_lc
            ));
            continue;
        }
        if let Some(usage) = read_usage_hint(&share_dir, pkg) {
            lines.push(format!("# vcpkg usage for {}:", pkg));
            for ln in usage.lines() {
                lines.push(format!("# {}", ln));
            }
            // Fall through to best-effort include/lib
        }

        // Fallback: include vcpkg installed include + guess libs from lib/
        lines.push(
            r#"target_include_directories(${_comp_name} PRIVATE "${VCPKG_INSTALLED_DIR}/${VCPKG_TARGET_TRIPLET}/include")"#
                .into(),
        );

        let guessed = guess_libs(&lib_dir, pkg);
        if guessed.is_empty() {
            // ESCAPE CMake braces for Rust's format!
            lines.push(format!(
                "# TODO(triton): could not find config for '{}'; add concrete libs from ${{{{VCPKG_INSTALLED_DIR}}}}/${{{{VCPKG_TARGET_TRIPLET}}}}/lib if needed",
                pkg
            ));
        } else {
            // link absolute paths (robust)
            let joined = guessed
                .iter()
                .map(|p| cmake_path(p))
                .collect::<Vec<_>>()
                .join(" ");
            lines.push(format!(
                "target_link_libraries(${{_comp_name}} PRIVATE {})",
                joined
            ));
        }
    }

    let block = if lines.is_empty() {
        "# (none)".to_string()
    } else {
        lines.join("\n")
    };

    let re = Regex::new(r"(?s)# ## triton:deps begin.*?# ## triton:deps end").unwrap();
    let replacement = format!("# ## triton:deps begin\n{}\n# ## triton:deps end", block);
    if re.is_match(&cmake) {
        cmake = re.replace(&cmake, replacement.as_str()).to_string();
    } else {
        cmake.push_str("\n");
        cmake.push_str(&replacement);
        cmake.push_str("\n");
    }
    fs::write(&p, cmake)?;
    Ok(())
}

fn has_config_package(share_dir: &Path, pkg: &str) -> bool {
    let d = share_dir.join(pkg);
    if !d.exists() {
        return false;
    }
    for e in WalkDir::new(&d).max_depth(2).into_iter().filter_map(|e| e.ok()) {
        let p = e.path();
        if p.is_file() {
            if let Some(name) = p.file_name().and_then(|s| s.to_str()) {
                if name.ends_with("Config.cmake") {
                    return true;
                }
            }
        }
    }
    false
}

fn read_usage_hint(share_dir: &Path, pkg: &str) -> Option<String> {
    let p = share_dir.join(pkg).join("usage");
    fs::read_to_string(p).ok()
}

fn guess_libs(lib_dir: &Path, pkg: &str) -> Vec<PathBuf> {
    if !lib_dir.exists() {
        return vec![];
    }
    let mut out = Vec::new();
    let needle = pkg.to_ascii_lowercase();
    for e in WalkDir::new(lib_dir)
        .min_depth(1)
        .max_depth(1)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let p = e.path();
        if !p.is_file() {
            continue;
        }
        if let Some(ext) = p.extension().and_then(|s| s.to_str()) {
            let ext_lc = ext.to_ascii_lowercase();
            if ext_lc == "lib" || ext_lc == "a" {
                if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
                    let stem_lc = stem.to_ascii_lowercase();
                    if stem_lc.contains(&needle) || stem_lc.starts_with(&format!("lib{}", needle)) {
                        out.push(p.to_path_buf());
                    }
                }
            }
        }
    }
    out
}

fn cmake_path(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}
