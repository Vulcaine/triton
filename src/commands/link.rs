use anyhow::Result;
use std::fs;

use crate::cmake::{regenerate_root_cmake, rewrite_component_cmake};
use crate::models::{TritonComponent, TritonRoot};
use crate::util::{read_json, write_json_pretty_changed};

/// Link one component to another (adds `to` as a dependency of `from`)
pub fn handle_link(from: &str, to: &str) -> Result<()> {
    // Ensure folders exist so CMake has subdirs
    fs::create_dir_all(format!("components/{from}"))?;
    fs::create_dir_all(format!("components/{to}"))?;

    // Load root metadata
    let mut root: TritonRoot = read_json("triton.json")?;

    // Ensure components exist in metadata (new schema: { kind, link })
    root.components
        .entry(from.to_string())
        .or_insert(TritonComponent {
            kind: "lib".into(),
            link: vec![],
        });

    root.components
        .entry(to.to_string())
        .or_insert(TritonComponent {
            kind: "lib".into(),
            link: vec![],
        });

    // Add link if missing
    {
        let comp_from = root.components.get_mut(from).unwrap();
        if !comp_from.link.iter().any(|x| x == to) {
            comp_from.link.push(to.to_string());
        }
    }

    // Persist only root (per-component json removed in new layout)
    write_json_pretty_changed("triton.json", &root)?;

    // Rewrite CMake for the 'from' component and regenerate root
    let comp_from = root.components.get(from).unwrap();
    rewrite_component_cmake(from, &root, comp_from)?;
    regenerate_root_cmake(&root)?;

    eprintln!("Linked component '{}' -> '{}'.", from, to);
    Ok(())
}
