use anyhow::Result;

use crate::cmake::{regenerate_root_cmake, rewrite_component_cmake};
use crate::models::{Dependency, DependencyDetail, TritonRoot, VcpkgManifest};
use crate::util::{read_json, write_json_pretty_changed};

pub fn handle_remove(pkg: &str, component: &str, features: Option<&str>, host: bool) -> Result<()> {
    // 1) Update component deps
    let mut root: TritonRoot = read_json("triton.json")?;
    if let Some(comp) = root.components.get_mut(component) {
        comp.deps.retain(|d| d != pkg);
        write_json_pretty_changed(&format!("components/{component}/triton.json"), comp)?;
    }
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
            Dependency::Detailed(mut d) => {
                if d.name != pkg {
                    return Some(Dependency::Detailed(d));
                }
                // if host flag mismatches, leave it
                if host != d.host.unwrap_or(false) {
                    return Some(Dependency::Detailed(d));
                }
                if feats_to_remove.is_empty() {
                    // remove whole dep entry
                    None
                } else {
                    // remove only listed features
                    d.features
                        .retain(|f| !feats_to_remove.iter().any(|x| x == f));
                    if d.features.is_empty() && d.host.is_none() && d.default_features.is_none() {
                        // convert to plain name
                        Some(Dependency::Name(d.name))
                    } else {
                        Some(Dependency::Detailed(d))
                    }
                }
            }
        })
        .collect();

    write_json_pretty_changed("vcpkg.json", &mani)?;

    // 3) Rewrite CMake for the component & root
    if let Some(comp) = root.components.get(component) {
        rewrite_component_cmake(component, comp)?;
    }
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
