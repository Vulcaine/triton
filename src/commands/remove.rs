// src/commands/remove.rs
use anyhow::Result;
use std::fs;
use std::path::Path;

use crate::cmake::{regenerate_root_cmake, rewrite_component_cmake};
use crate::models::{RootDep, TritonRoot};
use crate::util::{read_json, write_json_pretty_changed, write_text_if_changed};

pub fn handle_remove(pkg: &str, component_opt: Option<&str>, _features: Option<&str>, _host: bool) -> Result<()> {
    let mut root: TritonRoot = read_json("triton.json")?;

    // Remove from root.deps
    let mut removed_git_name: Option<String> = None;
    root.deps.retain(|d| {
        match d {
            RootDep::Name(n) => n != pkg,
            RootDep::Git(g) => {
                let hit = g.name == pkg || g.repo == pkg;
                if hit { removed_git_name = Some(g.name.clone()); }
                !hit
            }
        }
    });

    // Remove from components link lists
    if let Some(comp_name) = component_opt {
        if let Some(c) = root.components.get_mut(comp_name) {
            c.link.retain(|l| l != pkg && Some(l) != removed_git_name.as_ref());
        }
    } else {
        for (_, c) in root.components.iter_mut() {
            c.link.retain(|l| l != pkg && Some(l) != removed_git_name.as_ref());
        }
    }

    // Update vcpkg.json (only name-based deps stay there)
    let remaining_vcpkg: Vec<String> = root.deps.iter().filter_map(|d| {
        if let RootDep::Name(n) = d { Some(n.clone()) } else { None }
    }).collect();
    let mani = serde_json::json!({ "name": root.app_name, "version":"0.0.0", "dependencies": remaining_vcpkg });
    write_text_if_changed("vcpkg.json", &serde_json::to_string_pretty(&mani)?)?;

    write_json_pretty_changed("triton.json", &root)?;

    // Delete vendored folder if nobody references it anymore
    if let Some(n) = removed_git_name {
        let still_used = root.components.values().any(|c| c.link.iter().any(|l| l == &n));
        if !still_used {
            let dir = format!("third_party/{n}");
            if Path::new(&dir).exists() {
                let _ = fs::remove_dir_all(&dir);
            }
        }
    }

    // Regenerate cmake for all components (safest after global change)
    regenerate_root_cmake(&root)?;
    for (name, comp) in &root.components {
        rewrite_component_cmake(name, &root, comp)?;
    }
    eprintln!("Removed '{}' from project.", pkg);
    Ok(())
}
