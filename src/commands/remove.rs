use anyhow::Result;
use std::fs;
use std::path::Path;

use crate::cmake::{
    effective_cmake_version, regenerate_root_cmake, rewrite_component_cmake
};
use crate::models::{DepSpec, TritonRoot};
use crate::util::{read_json, write_json_pretty_changed, write_text_if_changed};

pub fn handle_remove(
    pkg: &str,
    component_opt: Option<&str>,
    _features: Option<&str>,
    _host: bool,
) -> Result<()> {
    let mut root: TritonRoot = read_json("triton.json")?;
    let cmake_ver = effective_cmake_version();

    // If a component is specified, only unlink from that component.
    if let Some(comp_name) = component_opt {
        let comp = root
            .components
            .get_mut(comp_name)
            .ok_or_else(|| anyhow::anyhow!("No such component '{}'", comp_name))?;

        comp.link.retain(|e| {
            let (name, _) = e.normalize();
            !name.eq_ignore_ascii_case(pkg)
        });

        write_json_pretty_changed("triton.json", &root)?;

        for (name, comp) in &root.components {
            rewrite_component_cmake(name, &root, comp, cmake_ver)?;
        }
        regenerate_root_cmake(&root)?; 

        eprintln!("Unlinked '{}' from component '{}'.", pkg, comp_name);
        return Ok(());
    }

    // Global remove
    root.deps.retain(|d| match d {
        DepSpec::Simple(n) => !n.eq_ignore_ascii_case(pkg),
        DepSpec::Git(g) => !(g.name.eq_ignore_ascii_case(pkg) || g.repo.eq_ignore_ascii_case(pkg)),
        DepSpec::Detailed(d) => !d.name.eq_ignore_ascii_case(pkg),
    });

    for c in root.components.values_mut() {
        c.link.retain(|e| {
            let (name, _) = e.normalize();
            !name.eq_ignore_ascii_case(pkg)
        });
    }

    // Sync vcpkg.json with remaining simple deps
    let remaining: Vec<String> = root
        .deps
        .iter()
        .filter_map(|d| match d {
            DepSpec::Simple(n) => Some(n.clone()),
            DepSpec::Detailed(d) if d.os.is_empty() && d.triplet.is_empty() => Some(d.name.clone()),
            _ => None,
        })
        .collect();

    let mani = serde_json::json!({
        "name": root.app_name,
        "version": "0.0.0",
        "dependencies": remaining
    });
    write_text_if_changed("vcpkg.json", &serde_json::to_string_pretty(&mani)?)?;
    write_json_pretty_changed("triton.json", &root)?;

    // remove vendored dir if unused
    let dir = format!("third_party/{pkg}");
    if Path::new(&dir).exists() {
        let still_used = root.deps.iter().any(|d| match d {
            DepSpec::Git(g) => g.name == pkg,
            _ => false,
        });
        if !still_used {
            let _ = fs::remove_dir_all(&dir);
        }
    }

    for (name, comp) in &root.components {
        rewrite_component_cmake(name, &root, comp, cmake_ver)?;
    }
    regenerate_root_cmake(&root)?; // ✅ fixed here too
    eprintln!("Removed '{}' from project.", pkg);
    Ok(())
}
