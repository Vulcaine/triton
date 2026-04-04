use anyhow::Result;
use std::collections::HashSet;

use crate::models::{TritonRoot};

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

    // 3. Self-links (only if the name is NOT also a dep — if it's a dep,
    //    the component is linking to the vcpkg/git dep, not to itself)
    for (name, comp) in &root.components {
        for entry in &comp.link {
            let (link_name, _) = entry.normalize();
            if link_name == *name && !is_dep_case_insensitive(root, &link_name) {
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
    if let Some(cycle) = detect_cycles(root) {
        anyhow::bail!("Circular dependency detected: {}", cycle.join(" -> "));
    }

    Ok(())
}

/// Detect circular dependencies among components using DFS.
/// When a link target is both a component AND a dep, the dep takes priority
/// (CMake wires it via find_package, not component linking), so we skip it
/// in cycle detection.
pub fn detect_cycles(root: &TritonRoot) -> Option<Vec<String>> {
    let mut visited = HashSet::new();
    let mut in_stack = HashSet::new();
    let mut path = Vec::new();

    for name in root.components.keys() {
        if !visited.contains(name.as_str()) {
            if let Some(cycle) = dfs_cycle(name, root, &mut visited, &mut in_stack, &mut path)
            {
                return Some(cycle);
            }
        }
    }
    None
}

fn dfs_cycle(
    node: &str,
    root: &TritonRoot,
    visited: &mut HashSet<String>,
    in_stack: &mut HashSet<String>,
    path: &mut Vec<String>,
) -> Option<Vec<String>> {
    visited.insert(node.to_string());
    in_stack.insert(node.to_string());
    path.push(node.to_string());

    if let Some(comp) = root.components.get(node) {
        for entry in &comp.link {
            let (link_name, _) = entry.normalize();
            // Only follow pure component-to-component links.
            // If the link target is also a dep, skip it — CMake resolves
            // it via find_package, not as a component dependency.
            if !root.components.contains_key(&link_name) {
                continue;
            }
            if is_dep_case_insensitive(root, &link_name) {
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
                if let Some(cycle) = dfs_cycle(&link_name, root, visited, in_stack, path) {
                    return Some(cycle);
                }
            }
        }
    }

    path.pop();
    in_stack.remove(node);
    None
}

pub fn is_dep(root: &TritonRoot, name: &str) -> bool {
    root.deps.iter().any(|d| d.name() == name)
}

pub fn is_dep_case_insensitive(root: &TritonRoot, name: &str) -> bool {
    root.deps.iter().any(|d| d.name().eq_ignore_ascii_case(name))
}

pub fn has_link_to_name(comp: &crate::models::TritonComponent, want: &str) -> bool {
    comp.link.iter().any(|e| {
        let (n, _pkg) = e.normalize();
        n == want
    })
}
