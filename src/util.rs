use anyhow::{ Context, Result};
use serde::{de::DeserializeOwned, Serialize};
use std::collections::{HashSet, BTreeMap};
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;

use crate::models::{TritonComponent, TritonRoot};

#[derive(Debug, Clone, Copy)]
pub enum Change {
    Created,
    Modified,
    Unchanged,
}

pub fn read_to_string_opt<P: AsRef<Path>>(p: P) -> Option<String> {
    fs::read_to_string(p.as_ref()).ok()
}

pub fn write_text_if_changed<P: AsRef<Path>>(p: P, content: &str) -> Result<Change> {
    let p = p.as_ref();
    if !p.exists() {
        if let Some(parent) = p.parent() { fs::create_dir_all(parent)?; }
        fs::write(p, content)?;
        return Ok(Change::Created);
    }
    let existing = fs::read_to_string(p)?;
    if existing == content {
        Ok(Change::Unchanged)
    } else {
        fs::write(p, content)?;
        Ok(Change::Modified)
    }
}

pub fn write_json_pretty_changed<P: AsRef<Path>, T: ?Sized + Serialize>(p: P, value: &T) -> Result<Change> {
    let s = serde_json::to_string_pretty(value)?;
    write_text_if_changed(p, &s)
}

pub fn read_json<P: AsRef<Path>, T: DeserializeOwned>(p: P) -> Result<T> {
    let s = fs::read_to_string(p.as_ref())
        .with_context(|| format!("reading {}", p.as_ref().display()))?;
    Ok(serde_json::from_str(&s)?)
}

pub fn run(exe: impl AsRef<Path>, args: &[&str], cwd: impl AsRef<Path>) -> Result<()> {
    let status = Command::new(exe.as_ref())
        .current_dir(cwd)
        .args(args)
        .status()
        .map_err(|e| anyhow::anyhow!("failed to spawn {}: {e}", exe.as_ref().display()))?;
    if !status.success() {
        return Err(anyhow::anyhow!("command exited with {}", status));
    }
    Ok(())
}

/// Convert paths to a form that plays nicely with CMake and Windows shells.
/// - Strip leading verbatim prefix (`\\?\` or `//?/`) if present (CMake 4.1+ often uses this).
/// - On Windows, return backslashes `\`.
/// - On non-Windows, return forward slashes `/`.
pub fn normalize_path<P: AsRef<Path>>(p: P) -> String {
    let mut s = p.as_ref().to_string_lossy().into_owned();

    // Strip Windows verbatim prefixes if present
    if s.starts_with(r"\\?\") {
        // remove the leading \\?\
        s = s.replacen(r"\\?\", "", 1);
    } else if s.starts_with("//?/") {
        // remove the leading //?/
        s = s.replacen("//?/", "", 1);
    }

    // Normalize separators per-platform
    if cfg!(windows) {
        // Use backslashes on Windows
        s = s.replace('/', r"\");
    } else {
        // Use forward slashes elsewhere
        s = s.replace('\\', "/");
    }

    s
}

pub fn ensure_component_scaffold(name: &str) -> anyhow::Result<()> {
    use std::fs;
    use std::io::Write;
    use std::path::Path;

    // components/<name>/
    let base = Path::new("components").join(name);
    fs::create_dir_all(&base)?;

    // components/<name>/src/<name> and components/<name>/include/<name>
    let src_dir = base.join("src").join(name);
    let inc_dir = base.join("include").join(name);
    fs::create_dir_all(&src_dir)?;
    fs::create_dir_all(&inc_dir)?;

    // Minimal placeholder header so includes like <Name/Name.hpp> resolve.
    let header_path = inc_dir.join(format!("{name}.hpp"));
    if !header_path.exists() {
        let mut f = fs::File::create(&header_path)?;
        writeln!(f, "#pragma once")?;
        writeln!(f, "// {} public headers live under this folder.", name)?;
    }

    // Minimal placeholder source (no main()).
    let source_path = src_dir.join(format!("{name}.cpp"));
    if !source_path.exists() {
        let mut f = fs::File::create(&source_path)?;
        writeln!(f, "#include <{0}/{0}.hpp>", name)?;
        writeln!(f, "// Implementation files for {} live here.", name)?;
    }

    Ok(())
}


pub fn is_dep(root: &TritonRoot, name: &str) -> bool {
    root.deps.iter().any(|d| d.name() == name)
}

pub fn is_dep_case_insensitive(root: &TritonRoot, name: &str) -> bool {
    root.deps.iter().any(|d| d.name().eq_ignore_ascii_case(name))
}

pub fn has_link_to_name(comp: &TritonComponent, want: &str) -> bool {
    comp.link.iter().any(|e| {
        let (n, _pkg) = e.normalize();
        n == want
    })
}

pub fn cmake_quote(val: &str) -> String {
    let s = val.trim().replace('"', "\\\"");
    format!("\"{}\"", s)
}

pub fn infer_cmake_type(val: &str) -> &'static str {
    match val.to_ascii_uppercase().as_str() {
        "ON" | "OFF" | "TRUE" | "FALSE" | "YES" | "NO" => "BOOL",
        _ => "STRING",
    }
}

pub fn split_kv(raw: &str) -> (String, String) {
    if let Some(idx) = raw.find('=') {
        let (k, v) = raw.split_at(idx);
        let key = k.trim().to_string();
        let mut val = v[1..].trim().to_string();
        if val.starts_with('"') && val.ends_with('"') && val.len() >= 2 {
            val = val[1..val.len() - 1].to_string();
        }
        (key, if val.is_empty() { "ON".into() } else { val })
    } else {
        (raw.trim().to_string(), "ON".to_string())
    }
}

// ===========================================================================
// vcpkg share-dir scanning (used by find-target and auto-detect)
// ===========================================================================

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

// ===========================================================================
// Validation
// ===========================================================================

/// Validate a TritonRoot for common errors. Returns Ok(()) if valid,
/// or an error describing the first problem found.
pub fn validate_triton_root(root: &TritonRoot) -> Result<()> {
    // 1. Empty app_name
    if root.app_name.trim().is_empty() {
        anyhow::bail!("app_name cannot be empty.");
    }

    // 2. Invalid kind
    for (name, comp) in &root.components {
        if comp.kind != "exe" && comp.kind != "lib" {
            anyhow::bail!(
                "Component '{}' has invalid kind '{}'. Must be 'exe' or 'lib'.",
                name, comp.kind
            );
        }
    }

    // 3. Self-links
    for (name, comp) in &root.components {
        for entry in &comp.link {
            let (link_name, _) = entry.normalize();
            if link_name == *name {
                anyhow::bail!("Component '{}' cannot link to itself.", name);
            }
        }
    }

    // 4. Unknown link targets (case-insensitive for deps since vcpkg/CMake are case-insensitive)
    for (comp_name, comp) in &root.components {
        for entry in &comp.link {
            let (link_name, _) = entry.normalize();
            if link_name.is_empty() {
                continue;
            }
            let in_deps = is_dep_case_insensitive(root, &link_name);
            let in_components = root.components.contains_key(&link_name);
            if !in_deps && !in_components {
                anyhow::bail!(
                    "Component '{}' links to '{}' which is not a known dep or component.",
                    comp_name, link_name
                );
            }
        }
    }

    // 5. Circular component dependencies
    if let Some(cycle) = detect_cycles(&root.components) {
        anyhow::bail!("Circular dependency detected: {}", cycle.join(" -> "));
    }

    Ok(())
}

/// Detect circular dependencies among components using DFS.
/// Returns Some(cycle_path) if a cycle is found, None otherwise.
pub fn detect_cycles(components: &BTreeMap<String, TritonComponent>) -> Option<Vec<String>> {
    let mut visited = HashSet::new();
    let mut in_stack = HashSet::new();
    let mut path = Vec::new();

    for name in components.keys() {
        if !visited.contains(name.as_str()) {
            if let Some(cycle) = dfs_cycle(name, components, &mut visited, &mut in_stack, &mut path)
            {
                return Some(cycle);
            }
        }
    }
    None
}

fn dfs_cycle(
    node: &str,
    components: &BTreeMap<String, TritonComponent>,
    visited: &mut HashSet<String>,
    in_stack: &mut HashSet<String>,
    path: &mut Vec<String>,
) -> Option<Vec<String>> {
    visited.insert(node.to_string());
    in_stack.insert(node.to_string());
    path.push(node.to_string());

    if let Some(comp) = components.get(node) {
        for entry in &comp.link {
            let (link_name, _) = entry.normalize();
            // Only follow component-to-component links
            if !components.contains_key(&link_name) {
                continue;
            }
            if in_stack.contains(&link_name) {
                // Found a cycle — build the cycle path
                let mut cycle = vec![];
                let start_idx = path.iter().position(|n| n == &link_name).unwrap_or(0);
                cycle.extend_from_slice(&path[start_idx..]);
                cycle.push(link_name);
                return Some(cycle);
            }
            if !visited.contains(&link_name) {
                if let Some(cycle) = dfs_cycle(&link_name, components, visited, in_stack, path) {
                    return Some(cycle);
                }
            }
        }
    }

    path.pop();
    in_stack.remove(node);
    None
}
