use anyhow::Result;

use crate::cmake::{effective_cmake_version, regenerate_root_cmake, rewrite_component_cmake};
use crate::models::TritonRoot;
use crate::util::{read_json, write_json_pretty_changed};

/// Unlink A from B: remove the dependency edge where B depends on A.
///   triton unlink A:B   — B no longer depends on A
///   triton unlink A      — remove A from ALL components' link lists
pub fn handle_unlink(from: &str, to: Option<&str>) -> Result<()> {
    let mut root: TritonRoot = read_json("triton.json")?;
    let cmake_ver = effective_cmake_version();

    if let Some(comp_name) = to {
        // Unlink from a specific component
        let comp = root
            .components
            .get_mut(comp_name)
            .ok_or_else(|| anyhow::anyhow!("No such component '{}'", comp_name))?;

        let before = comp.link.len();
        comp.link.retain(|e| {
            let (name, _) = e.normalize();
            !name.eq_ignore_ascii_case(from)
        });

        if comp.link.len() == before {
            eprintln!("Component '{}' does not link to '{}'.", comp_name, from);
            return Ok(());
        }

        write_json_pretty_changed("triton.json", &root)?;

        // Rewrite CMake for affected component
        if let Some(c) = root.components.get(comp_name) {
            rewrite_component_cmake(comp_name, &root, c, cmake_ver)?;
        }
        regenerate_root_cmake(&root)?;

        eprintln!("Unlinked '{}' from component '{}'.", from, comp_name);
    } else {
        // Unlink from ALL components
        let mut unlinked_from = Vec::new();

        for (comp_name, comp) in root.components.iter_mut() {
            let before = comp.link.len();
            comp.link.retain(|e| {
                let (name, _) = e.normalize();
                !name.eq_ignore_ascii_case(from)
            });
            if comp.link.len() < before {
                unlinked_from.push(comp_name.clone());
            }
        }

        if unlinked_from.is_empty() {
            eprintln!("'{}' is not linked to any component.", from);
            return Ok(());
        }

        write_json_pretty_changed("triton.json", &root)?;

        for (name, comp) in &root.components {
            rewrite_component_cmake(name, &root, comp, cmake_ver)?;
        }
        regenerate_root_cmake(&root)?;

        eprintln!(
            "Unlinked '{}' from: {}",
            from,
            unlinked_from.join(", ")
        );
    }

    Ok(())
}
