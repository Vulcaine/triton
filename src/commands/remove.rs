use anyhow::Result;
use std::fs;

use crate::cmake::{regenerate_root_cmake, rewrite_component_cmake};
use crate::models::{dep_eq, Dependency, DependencyDetail, TritonRoot, VcpkgManifest};
use crate::util::{read_json, write_json_pretty_changed};

pub fn handle_remove(pkg: &str, component: &str, features: Option<&str>, host: bool) -> Result<()> {
    // 1) Update component deps
    let mut root: TritonRoot = read_json("triton.json")?;
    if let Some(comp) = root.components.get_mut(component) {
        comp.deps.retain(|d| d != pkg);
        // persist component view
        let _ = write_json_pretty_changed(
            &format!("components/{component}/triton.json"),
            comp,
        )?;
    }
    let _ = write_json_pretty_changed("triton.json", &root)?;

    // 2) Update vcpkg manifest
    let mut mani: VcpkgManifest = read_json("vcpkg.json")?;
    let feats: Vec<String> = features
        .unwrap_or_default()
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    mani.dependencies = mani
        .dependencies
        .into_iter()
        .filter_map(|dep| {
            match dep {
                Dependency::Name(n) => {
                    if feats.is_empty() && !host && n == pkg {
                        // remove whole dep
                        None
                    } else {
                        Some(Dependency::Name(n))
                    }
                }
                Dependency::Detailed(mut d) => {
                    if d.name != pkg { return Some(Dependency::Detailed(d)); }
                    // respect host flag if it was set when adding
                    if host != d.host.unwrap_or(false) {
                        return Some(Dependency::Detailed(d));
                    }
                    if feats.is_empty() {
                        // no features specified -> remove entire entry
                        None
                    } else {
                        // remove only listed features
                        d.features.retain(|f| !feats.iter().any(|x| x == f));
                        if d.features.is_empty() {
                            // convert to plain name (default features) rather than dropping completely
                            // If user wanted to drop package entirely they should omit --features.
                            Some(Dependency::Name(d.name))
                        } else {
                            Some(Dependency::Detailed(d))
                        }
                    }
                }
            }
        })
        .collect();

    let _ = write_json_pretty_changed("vcpkg.json", &mani)?;

    // 3) Rewrite CMake for the component & root
    if let Some(comp) = root.components.get(component) {
        rewrite_component_cmake(component, comp)?;
    }
    regenerate_root_cmake(&root)?;
    eprintln!("Removed '{}'{} from component '{}'.",
        pkg,
        if feats.is_empty() { "" } else { &format!(" (features: {})", feats.join(",")) },
        component);
    Ok(())
}
