use anyhow::Result;
use std::fs;
use std::path::Path;

use crate::cmake::{regenerate_root_cmake, rewrite_component_cmake};
use crate::models::{RootDep, TritonRoot};
use crate::util::{read_json, write_json_pretty_changed, write_text_if_changed};

pub fn handle_remove(pkg: &str, component_opt: Option<&str>, features: Option<&str>, host: bool) -> Result<()> {
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

    // Remove from all components link lists
    for (_, c) in root.components.iter_mut() {
        c.link.retain(|l| l != pkg && Some(l) != removed_git_name.as_ref());
    }

    // Update vcpkg.json to be the set of all remaining RootDep::Name
    let remaining_vcpkg: Vec<String> = root.deps.iter().filter_map(|d| {
        if let RootDep::Name(n) = d { Some(n.clone()) } else { None }
    }).collect();
    let mut mani: serde_json::Value = serde_json::json!({ "name": root.app_name, "version":"0.0.0", "dependencies": remaining_vcpkg });
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
    for (name, comp) in &root.components {
        rewrite_component_cmake(name, &root, comp)?;
    }
    regenerate_root_cmake(&root)?;
    eprintln!("Removed '{}' from project.", pkg);
    Ok(())
}
