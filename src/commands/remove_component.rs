use anyhow::{bail, Result};
use std::fs;
use std::path::Path;

use crate::cmake::{effective_cmake_version, regenerate_root_cmake, rewrite_component_cmake};
use crate::models::TritonRoot;
use crate::util::{read_json, write_json_pretty_changed};

/// Remove a component entirely: delete it from triton.json, remove it from
/// other components' link lists, remove the on-disk directory, and regenerate CMake.
pub fn handle_remove_component(name: &str) -> Result<()> {
    let mut root: TritonRoot = read_json("triton.json")?;

    if !root.components.contains_key(name) {
        bail!("No such component '{}' in triton.json", name);
    }

    // Remove the component itself
    root.components.remove(name);

    // Remove references to this component from all other components' link lists
    for comp in root.components.values_mut() {
        comp.link.retain(|e| {
            let (link_name, _) = e.normalize();
            link_name != name
        });

        // Also remove from exports if present
        comp.exports.retain(|e| e != name);
    }

    // Save updated triton.json
    write_json_pretty_changed("triton.json", &root)?;

    // Remove the on-disk component directory
    let comp_dir = Path::new("components").join(name);
    if comp_dir.exists() {
        fs::remove_dir_all(&comp_dir)?;
        eprintln!("Removed directory: {}", comp_dir.display());
    }

    // Regenerate CMake for remaining components
    let cmake_ver = effective_cmake_version();
    for (comp_name, comp) in &root.components {
        rewrite_component_cmake(comp_name, &root, comp, cmake_ver)?;
    }
    regenerate_root_cmake(&root)?;

    eprintln!("Removed component '{}' from project.", name);
    Ok(())
}
