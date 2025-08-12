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

    // Ensure components exist in metadata
    root.components.entry(from.into()).or_insert(TritonComponent {
        kind: "lib".into(),
        deps: vec![],
        comps: vec![],
    });
    root.components.entry(to.into()).or_insert(TritonComponent {
        kind: "lib".into(),
        deps: vec![],
        comps: vec![],
    });

    // Add link if missing
    {
        let c = root.components.get_mut(from).unwrap();
        if !c.comps.iter().any(|x| x == to) {
            c.comps.push(to.into());
        }
    }

    // Persist
    let _ = write_json_pretty_changed("triton.json", &root)?;
    let _ = write_json_pretty_changed(
        &format!("components/{from}/triton.json"),
        root.components.get(from).unwrap(),
    )?;

    // Rewrite CMake
    rewrite_component_cmake(from, root.components.get(from).unwrap())?;
    regenerate_root_cmake(&root)?;

    eprintln!("Linked component '{}' -> '{}'.", from, to);
    Ok(())
}
