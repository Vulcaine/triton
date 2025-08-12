// src/commands/link.rs
use anyhow::{Context, Result};
use std::fs;
use std::path::Path;

use crate::cmake::{regenerate_root_cmake, rewrite_component_cmake};
use crate::models::{TritonComponent, TritonRoot};
use crate::templates::component_cmakelists;
use crate::util::{read_json, write_json_pretty_changed, write_text_if_changed};

/// Link one component to another (adds `to` as a dependency of `from`).
/// Creates missing components and their CMakeLists.txt.
pub fn handle_link(from: &str, to: &str) -> Result<()> {
    // Ensure folders + template CMakeLists exist for both components
    for name in [from, to] {
        let base = format!("components/{name}");
        fs::create_dir_all(format!("{base}/src"))?;
        fs::create_dir_all(format!("{base}/include"))?;
        let cm_path = format!("{base}/CMakeLists.txt");
        if !Path::new(&cm_path).exists() {
            write_text_if_changed(&cm_path, &component_cmakelists())
                .with_context(|| format!("writing {}", cm_path))?;
        }
    }

    // Load and update root metadata
    let mut root: TritonRoot = read_json("triton.json")?;
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
            c.link.push(to.to_string());
        }
    }

    // Persist + regenerate cmake
    write_json_pretty_changed("triton.json", &root)?;
    let cfrom = root.components.get(from).unwrap();
    rewrite_component_cmake(from, &root, cfrom)?;
    let cto = root.components.get(to).unwrap();
    rewrite_component_cmake(to, &root, cto)?;
    regenerate_root_cmake(&root)?;

    eprintln!("Linked component '{}' : '{}'.", from, to);
    Ok(())
}
