use anyhow::Result;
use std::fs;

use crate::cmake::{regenerate_root_cmake, rewrite_component_cmake};
use crate::models::{TritonComponent, TritonRoot};
use crate::util::{read_json, write_json_pretty_changed};

/// Link one component to another (adds `to` as a dependency of `from`)
/// Supports creating missing components (as libs).
pub fn handle_link(from: &str, to: &str) -> Result<()> {
    // Ensure folders exist so CMake has subdirs
    fs::create_dir_all(format!("components/{from}"))?;
    fs::create_dir_all(format!("components/{to}"))?;

    // Load root metadata
    let mut root: TritonRoot = read_json("triton.json")?;

    // Ensure components exist in metadata
    root.components.entry(from.into()).or_insert(TritonComponent {
        kind: "lib".into(),
        link: vec![],
    });
    root.components.entry(to.into()).or_insert(TritonComponent {
        kind: "lib".into(),
        link: vec![],
    });

    // Add link if missing
    {
        let c = root.components.get_mut(from).unwrap();
        if !c.link.iter().any(|x| x == to) {
            c.link.push(to.into());
        }
    }

    // Persist only root; no per-component triton.json anymore.
    write_json_pretty_changed("triton.json", &root)?;

    // Rewrite CMake for both components and root
    if let Some(cfrom) = root.components.get(from) {
        rewrite_component_cmake(from, &root, cfrom)?;
    }
    if let Some(cto) = root.components.get(to) {
        rewrite_component_cmake(to, &root, cto)?;
    }
    regenerate_root_cmake(&root)?;

    eprintln!("Linked component '{}' -> '{}'.", from, to);
    Ok(())
}
