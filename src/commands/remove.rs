use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

use crate::cmake::{regenerate_root_cmake, rewrite_component_cmake};
use crate::models::{Dependency, DependencyDetail, TritonRoot, VcpkgManifest};
use crate::util::{read_json, write_json_pretty_changed};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct ThirdPartyDep {
    repo: String,
    name: String,
    #[serde(default)]
    target: Option<String>,
    #[serde(default)]
    branch: Option<String>,
}

fn third_party_list_path(component: &str) -> String {
    format!("components/{component}/third_party.json")
}

fn load_third_party(component: &str) -> Result<Vec<ThirdPartyDep>> {
    let p = third_party_list_path(component);
    let path = Path::new(&p);
    if path.exists() {
        Ok(read_json(path)?)
    } else {
        Ok(vec![])
    }
}

fn save_third_party(component: &str, list: &Vec<ThirdPartyDep>) -> Result<()> {
    let _ = write_json_pretty_changed(third_party_list_path(component), list)?;
    Ok(())
}

pub fn handle_remove(pkg: &str, component: &str, features: Option<&str>, host: bool) -> Result<()> {
    let mut root: TritonRoot = read_json("triton.json")?;

    // --- try third_party first (accept owner/repo or just folder name) ---
    {
        let mut tp = load_third_party(component)?;
        let before = tp.len();
        tp.retain(|t| !(t.repo.eq_ignore_ascii_case(pkg) || t.name.eq_ignore_ascii_case(pkg)));
        if tp.len() != before {
            // delete folder too
            for removed in load_third_party(component)?
                .into_iter()
                .filter(|t| t.repo.eq_ignore_ascii_case(pkg) || t.name.eq_ignore_ascii_case(pkg))
            {
                let dir = format!("third_party/{}", removed.name);
                if Path::new(&dir).exists() {
                    let _ = fs::remove_dir_all(&dir);
                }
            }
            save_third_party(component, &tp)?;
            // Rewrite CMake & root and return
            if let Some(comp) = root.components.get(component) {
                rewrite_component_cmake(component, comp)?;
            }
            regenerate_root_cmake(&root)?;
            eprintln!("Removed third_party '{}' from component '{}'.", pkg, component);
            return Ok(());
        }
    }

    // --- normal vcpkg path ---
    // Update component deps
    if let Some(comp) = root.components.get_mut(component) {
        comp.deps.retain(|d| d != pkg);
        write_json_pretty_changed(&format!("components/{component}/triton.json"), comp)?;
    }
    write_json_pretty_changed("triton.json", &root)?;

    // Update vcpkg manifest
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
                // if host flag mismatches, keep it
                if host != d.host.unwrap_or(false) {
                    return Some(Dependency::Detailed(d));
                }
                if feats_to_remove.is_empty() {
                    None
                } else {
                    d.features
                        .retain(|f| !feats_to_remove.iter().any(|x| x == f));
                    if d.features.is_empty() && d.host.is_none() && d.default_features.is_none() {
                        Some(Dependency::Name(d.name))
                    } else {
                        Some(Dependency::Detailed(d))
                    }
                }
            }
        })
        .collect();

    write_json_pretty_changed("vcpkg.json", &mani)?;

    // Rewrite CMake & root
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
