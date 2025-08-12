use anyhow::Result;
use std::fs;
use std::path::Path;

use crate::cmake::{regenerate_root_cmake, rewrite_component_cmake};
use crate::models::{RootDep, TritonRoot};
use crate::util::{read_json, write_json_pretty_changed, write_text_if_changed};

pub fn handle_remove(pkg: &str, component_opt: Option<&str>, _features: Option<&str>, _host: bool) -> Result<()> {
    let mut root: TritonRoot = read_json("triton.json")?;

    // If component specified: only unlink from that component; keep deps intact.
    if let Some(comp_name) = component_opt {
        // mutate inside a narrow scope so the mutable borrow ends before we
        // pass &root immutably to the codegen helpers.
        let mut touched = false;
        {
            let comp = root.components.get_mut(comp_name)
                .ok_or_else(|| anyhow::anyhow!("No such component '{}'", comp_name))?;
            let before = comp.link.len();
            comp.link.retain(|l| l != pkg);
            touched = comp.link.len() != before;
        } // <- mutable borrow of root ends here

        if touched {
            write_json_pretty_changed("triton.json", &root)?;
            let comp_ref = root.components.get(comp_name).expect("exists after retain");
            rewrite_component_cmake(comp_name, &root, comp_ref)?;
            regenerate_root_cmake(&root)?;
            eprintln!("Unlinked '{}' from component '{}'.", pkg, comp_name);
        } else {
            eprintln!("Component '{}' had no link to '{}'. Nothing to do.", comp_name, pkg);
        }
        return Ok(());
    }

    // No component specified: remove from project deps and unlink from all components
    let mut removed_git_name: Option<String> = None;
    root.deps.retain(|d| match d {
        RootDep::Name(n) => n != pkg,
        RootDep::Git(g) => {
            let hit = g.name == pkg || g.repo == pkg;
            if (hit) { removed_git_name = Some(g.name.clone()); }
            !hit
        }
    });

    for (_, c) in root.components.iter_mut() {
        c.link.retain(|l| l != pkg && Some(l) != removed_git_name.as_ref());
    }

    // Update vcpkg.json to contain only remaining vcpkg "Name" deps
    let remaining_vcpkg: Vec<String> = root.deps.iter().filter_map(|d| {
        if let RootDep::Name(n) = d { Some(n.clone()) } else { None }
    }).collect();
    let mani = serde_json::json!({
        "name": root.app_name,
        "version": "0.0.0",
        "dependencies": remaining_vcpkg
    });
    write_text_if_changed("vcpkg.json", &serde_json::to_string_pretty(&mani)?)?;

    write_json_pretty_changed("triton.json", &root)?;

    // Delete vendored folder if not referenced by any component anymore
    if let Some(n) = removed_git_name {
        let still_used = root.components.values().any(|c| c.link.iter().any(|l| l == &n));
        if !still_used {
            let dir = format!("third_party/{n}");
            if Path::new(&dir).exists() {
                let _ = fs::remove_dir_all(&dir);
            }
        }
    }

    // Regenerate cmake for all components after a global dep change
    for (name, comp) in &root.components {
        rewrite_component_cmake(name, &root, comp)?;
    }
    regenerate_root_cmake(&root)?;
    eprintln!("Removed '{}' from project dependencies.", pkg);
    Ok(())
}
