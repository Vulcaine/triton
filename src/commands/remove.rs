use anyhow::Result;
use std::fs;
use std::path::Path;

use crate::cmake::{regenerate_root_cmake, rewrite_component_cmake};
use crate::models::{RootDep, TritonRoot};
use crate::util::{read_json, write_json_pretty_changed, write_text_if_changed};

pub fn handle_remove(pkg: &str, component_opt: Option<&str>, _features: Option<&str>, _host: bool) -> Result<()> {
    let mut root: TritonRoot = read_json("triton.json")?;

    if let Some(comp_name) = component_opt {
        {
            let comp = root
                .components
                .get_mut(comp_name)
                .ok_or_else(|| anyhow::anyhow!("No such component '{}'", comp_name))?;

            comp.link.retain(|e| {
                let (name, _) = e.normalize();
                name != pkg
            });
        }

        write_json_pretty_changed("triton.json", &root)?;
        let comp = root.components.get(comp_name).expect("component should exist");
        rewrite_component_cmake(comp_name, &root, comp)?;
        regenerate_root_cmake(&root)?;
        eprintln!("Unlinked '{}' from component '{}'.", pkg, comp_name);
        return Ok(());
    }

    // Remove dep from root.deps
    let mut removed_git_name: Option<String> = None;
    root.deps.retain(|d| match d {
        RootDep::Name(n) => n != pkg,
        RootDep::Git(g) => {
            let hit = g.name == pkg || g.repo == pkg;
            if hit {
                removed_git_name = Some(g.name.clone());
            }
            !hit
        }
    });

    // Unlink from all components
    for c in root.components.values_mut() {
        c.link.retain(|e| {
            let (name, _) = e.normalize();
            name != pkg && Some(name.as_str()) != removed_git_name.as_deref()
        });
    }

    // Sync vcpkg.json to remaining vcpkg deps
    let remaining: Vec<String> = root
        .deps
        .iter()
        .filter_map(|d| if let RootDep::Name(n) = d { Some(n.clone()) } else { None })
        .collect();

    let mani = serde_json::json!({
        "name": root.app_name,
        "version": "0.0.0",
        "dependencies": remaining
    });
    write_text_if_changed("vcpkg.json", &serde_json::to_string_pretty(&mani)?)?;
    write_json_pretty_changed("triton.json", &root)?;

    // Remove vendored dir if fully unused
    if let Some(n) = removed_git_name {
        let still_used = root.components.values().any(|c| {
            c.link.iter().any(|e| {
                let (name, _) = e.normalize();
                name == n
            })
        });
        if !still_used {
            let dir = format!("third_party/{n}");
            if Path::new(&dir).exists() {
                let _ = fs::remove_dir_all(&dir);
            }
        }
    }

    // Immutable borrows only here
    for (name, comp) in &root.components {
        rewrite_component_cmake(name, &root, comp)?;
    }
    regenerate_root_cmake(&root)?;
    eprintln!("Removed '{}' from project.", pkg);
    Ok(())
}
