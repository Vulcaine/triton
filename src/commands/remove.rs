use anyhow::Result;
use std::fs;
use std::path::Path;

use crate::cmake::{regenerate_root_cmake, rewrite_component_cmake};
use crate::models::{Dependency, TritonRoot, VcpkgManifest};
use crate::util::{read_json, write_json_pretty_changed};

pub fn handle_remove(pkg: &str, component: &str, features: Option<&str>, host: bool) -> Result<()> {
    // Load root + locate component
    let mut root: TritonRoot = read_json("triton.json")?;
    if !root.components.contains_key(component) {
        eprintln!("Component '{}' not found; nothing to do.", component);
        return Ok(());
    }

    // --- Vendored (Git) removal path ---------------------------------------------------------
    // If looks like GitHub "owner/name" OR matches a vendored name, remove from git list
    let tail = pkg.split('/').last().unwrap_or(pkg).to_string();
    let vendored_removed = {
        let comp = root.components.get_mut(component).unwrap(); // mutable borrow scoped to this block
        let before_len = comp.git.len();
        comp.git.retain(|g| !(g.repo == pkg || g.name == tail));
        if comp.git.len() != before_len {
            // Persist component file while we still have &mut comp
            write_json_pretty_changed(&format!("components/{component}/triton.json"), comp)?;
            true
        } else {
            false
        }
    }; // <— mutable borrow of root.components ends here

    if vendored_removed {
        // Now we can borrow `root` immutably again
        write_json_pretty_changed("triton.json", &root)?;

        // If no other component uses this vendored folder, delete it
        let still_used_elsewhere = root.components.iter().any(|(n, c)| {
            if n == component {
                return false;
            }
            c.git.iter().any(|g| g.name == tail || g.repo == pkg)
        });
        if !still_used_elsewhere {
            let dir = format!("third_party/{}", tail);
            if Path::new(&dir).exists() {
                let _ = fs::remove_dir_all(&dir);
            }
        }

        // Rewrite CMake
        let comp_now = root.components.get(component).unwrap();
        rewrite_component_cmake(component, comp_now)?;
        regenerate_root_cmake(&root)?;
        eprintln!("Removed vendored '{}' from component '{}'.", pkg, component);
        return Ok(());
    }

    // --- vcpkg removal path ------------------------------------------------------------------
    // 1) Update component deps
    {
        let comp = root.components.get_mut(component).unwrap();
        comp.deps.retain(|d| d != pkg);
        write_json_pretty_changed(&format!("components/{component}/triton.json"), comp)?;
    } // drop mutable borrow
    write_json_pretty_changed("triton.json", &root)?;

    // 2) Update vcpkg manifest
    let mut mani: VcpkgManifest = read_json("vcpkg.json")?;
    let feats_to_remove: Vec<String> = features
        .unwrap_or_default()
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    mani.dependencies = mani
        .dependencies
        .into_iter()
        .filter_map(|dep| match dep {
            Dependency::Name(n) => {
                if feats_to_remove.is_empty() && !host && n == pkg {
                    None // drop whole dependency
                } else {
                    Some(Dependency::Name(n))
                }
            }
            // Detailed dep: possibly remove specific features or entire entry
            crate::models::Dependency::Detailed(mut d) => {
                if d.name != pkg {
                    return Some(crate::models::Dependency::Detailed(d));
                }
                if host != d.host.unwrap_or(false) {
                    return Some(crate::models::Dependency::Detailed(d));
                }
                if feats_to_remove.is_empty() {
                    None
                } else {
                    d.features.retain(|f| !feats_to_remove.iter().any(|x| x == f));
                    if d.features.is_empty() && d.host.is_none() && d.default_features.is_none() {
                        Some(Dependency::Name(d.name))
                    } else {
                        Some(crate::models::Dependency::Detailed(d))
                    }
                }
            }
        })
        .collect();

    write_json_pretty_changed("vcpkg.json", &mani)?;

    // 3) Rewrite CMake + root
    let comp_now = root.components.get(component).unwrap();
    rewrite_component_cmake(component, comp_now)?;
    regenerate_root_cmake(&root)?;
    eprintln!(
        "Removed '{}'{} from component '{}'.",
        pkg,
        if feats_to_remove.is_empty() {
            "".to_string()
        } else {
            format!(" (features: {})", feats_to_remove.join(","))
        },
        component
    );
    Ok(())
}
