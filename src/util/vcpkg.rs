use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

/// Scan a package's share directory for exported CMake target names.
/// Parses `*Targets.cmake` and `*-targets.cmake` files for `add_library(Ns::Target` patterns.
pub fn discover_cmake_targets(pkg_share_dir: &Path) -> Vec<String> {
    let mut targets = Vec::new();
    let entries = match fs::read_dir(pkg_share_dir) {
        Ok(e) => e,
        Err(_) => return targets,
    };
    for entry in entries.flatten() {
        let fname = entry.file_name().to_string_lossy().to_string();
        // Look for *Targets.cmake or *-targets.cmake (but not *-targets-debug/release.cmake)
        let is_targets_file = (fname.ends_with("Targets.cmake") || fname.ends_with("-targets.cmake"))
            && !fname.contains("-targets-debug")
            && !fname.contains("-targets-release")
            && !fname.contains("-targets-relwithdebinfo")
            && !fname.contains("-targets-minsizerel");
        if !is_targets_file { continue; }

        let content = match fs::read_to_string(entry.path()) {
            Ok(c) => c,
            Err(_) => continue,
        };

        // Parse lines like: add_library(Microsoft::DirectXTex SHARED IMPORTED)
        // or: add_library(SDL2::SDL2 STATIC IMPORTED)
        for line in content.lines() {
            let trimmed = line.trim();
            if let Some(rest) = trimmed.strip_prefix("add_library(") {
                if let Some(name_end) = rest.find(|c: char| c.is_whitespace() || c == ')') {
                    let target = &rest[..name_end];
                    if target.contains("::") && !targets.contains(&target.to_string()) {
                        targets.push(target.to_string());
                    }
                }
            }
        }
    }
    targets.sort();
    targets
}

/// Scan a vcpkg share directory and return all valid CMake config packages.
/// Returns vec of (package_name, path_to_config_cmake).
pub fn scan_vcpkg_share_for_configs(share_dir: &Path) -> Vec<(String, PathBuf)> {
    let mut results = Vec::new();
    let entries = match fs::read_dir(share_dir) {
        Ok(e) => e,
        Err(_) => return results,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let dir_name = match entry.file_name().into_string() {
            Ok(n) => n,
            Err(_) => continue,
        };
        // Check for <DirName>Config.cmake or <dirname>-config.cmake
        let config_path = path.join(format!("{}Config.cmake", dir_name));
        let config_path_lower = path.join(format!("{}-config.cmake", dir_name.to_lowercase()));
        if config_path.exists() {
            results.push((dir_name, config_path));
        } else if config_path_lower.exists() {
            results.push((dir_name, config_path_lower));
        }
    }
    results
}

/// Normalize a dep name for matching: lowercase, replace hyphens with underscores.
fn normalize_for_match(s: &str) -> String {
    s.to_ascii_lowercase().replace('-', "_")
}

/// Match a dep name against discovered CMake packages.
/// Returns matching packages ranked by relevance (best first).
pub fn match_dep_to_packages(
    dep_name: &str,
    packages: &[(String, PathBuf)],
) -> Vec<(String, PathBuf)> {
    let dep_norm = normalize_for_match(dep_name);

    let mut exact = Vec::new();
    let mut partial = Vec::new();

    for (pkg_name, path) in packages {
        let pkg_norm = normalize_for_match(pkg_name);

        if pkg_norm == dep_norm {
            // Exact match (case/hyphen-insensitive)
            exact.push((pkg_name.clone(), path.clone()));
        } else if pkg_norm.contains(&dep_norm) || dep_norm.contains(&pkg_norm) {
            // Substring match
            partial.push((pkg_name.clone(), path.clone()));
        }
    }

    // Exact matches first, then partial
    exact.extend(partial);
    exact
}

/// List directory names in a path. Returns empty set if path doesn't exist.
pub fn list_dir_names(dir: &Path) -> HashSet<String> {
    let mut names = HashSet::new();
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            if entry.path().is_dir() {
                if let Ok(name) = entry.file_name().into_string() {
                    names.insert(name);
                }
            }
        }
    }
    names
}
