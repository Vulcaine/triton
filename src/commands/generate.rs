use anyhow::Result;
use serde_json::json;

use crate::cmake::{
    dep_is_active, detect_vcpkg_triplet, effective_cmake_version, regenerate_root_cmake, rewrite_component_cmake
};
use crate::models::{DepDetailed, DepSpec, TritonRoot};
use crate::templates::cmake_presets;
use crate::util::{read_json, validate_triton_root, write_json_pretty_changed, write_text_if_changed};

pub fn handle_generate() -> Result<()> {
    let trip = detect_vcpkg_triplet();
    handle_generate_for_triplet(&trip)
}

pub fn handle_generate_for_triplet(trip: &str) -> Result<()> {
    eprintln!("Regenerating project CMake + vcpkg manifest...");
    let mut root: TritonRoot = read_json("triton.json")?;

    // Fix malformed deps (e.g. "pkg[feature]" strings â†’ Detailed)
    let mut fixed = fix_malformed_deps(&mut root);
    // Fix malformed link entries (e.g. "pkg[feature]" â†’ "pkg")
    fixed |= fix_malformed_links(&mut root);
    // Dedup deps by name (keep last occurrence, which is usually the most complete)
    let deduped = dedup_deps(&mut root);
    let needs_save = fixed || deduped;

    validate_triton_root(&root)?;

    let cmake_ver = effective_cmake_version();

    // Rewrite all component CMakeLists
    for (name, comp) in &root.components {
        rewrite_component_cmake(name, &root, comp, cmake_ver, None)?;
    }

    // Root CMake
    regenerate_root_cmake(&root)?;

    // --- regenerate CMakePresets.json ---
    eprintln!("detected triplet: {}", trip);

    write_text_if_changed(
        "components/CMakePresets.json",
        &cmake_presets(&root.app_name, &root.generator, &trip, cmake_ver),
    )?;

    // --- regenerate vcpkg.json ---
    let host_os = std::env::consts::OS;
    let mut deps: Vec<serde_json::Value> = Vec::new();

    for dep in &root.deps {
        match dep {
            DepSpec::Simple(s) => {
                if dep_is_active(dep, s, host_os, &trip) {
                    deps.push(json!(s));
                }
            }
            DepSpec::Git(_) => {}
            DepSpec::Detailed(d) => {
                if dep_is_active(dep, &d.name, host_os, &trip) {
                    if !d.features.is_empty() {
                        // vcpkg manifest mode requires object format for features
                        deps.push(json!({
                            "name": d.name,
                            "features": d.features,
                        }));
                    } else {
                        deps.push(json!(d.name));
                    }
                }
            }
        }
    }

    let vcpkg_name = root.app_name.to_lowercase().replace('_', "-");
    let vcpkg_manifest = json!({
        "name": vcpkg_name,
        "version": "0.1.0",
        "dependencies": deps,
    });

    let text = serde_json::to_string_pretty(&vcpkg_manifest)?;
    write_text_if_changed("vcpkg.json", &text)?;

    // Only write triton.json if we fixed something
    if needs_save {
        write_json_pretty_changed("triton.json", &root)?;
    }

    eprintln!("Regenerated CMake files and vcpkg.json.");
    Ok(())
}

/// Remove duplicate deps by name, keeping the most detailed version.
/// A Detailed dep wins over Simple; later entries win for same-type ties.
/// Fix malformed link entries: strip bracket notation from LinkEntry::Name values.
/// e.g. "directxtex[dx12]" â†’ "directxtex"
fn fix_malformed_links(root: &mut TritonRoot) -> bool {
    use crate::models::LinkEntry;
    let mut fixed = false;
    for comp in root.components.values_mut() {
        for link in &mut comp.link {
            if let LinkEntry::Name(s) = link {
                if let Some(bracket_start) = s.find('[') {
                    let clean = s[..bracket_start].trim().to_string();
                    eprintln!("Fixed malformed link: \"{}\" â†’ \"{}\"", s, clean);
                    *s = clean;
                    fixed = true;
                }
            }
        }
        // Dedup links by name after cleanup
        let mut seen = std::collections::HashSet::new();
        comp.link.retain(|e| {
            let (name, _) = e.normalize();
            seen.insert(name)
        });
    }
    fixed
}

/// Fix malformed deps like "pkg[feature1,feature2]" strings â†’ DepDetailed.
/// Returns true if any were fixed.
fn fix_malformed_deps(root: &mut TritonRoot) -> bool {
    let mut fixed = false;
    for i in 0..root.deps.len() {
        if let DepSpec::Simple(s) = &root.deps[i] {
            if let Some(bracket_start) = s.find('[') {
                let name = s[..bracket_start].trim().to_string();
                let features_str = s[bracket_start + 1..].trim_end_matches(']');
                let features: Vec<String> = features_str
                    .split(',')
                    .map(|f| f.trim().to_string())
                    .filter(|f| !f.is_empty())
                    .collect();
                eprintln!("Fixed malformed dep: \"{}\" â†’ {{ name: \"{}\", features: {:?} }}", s, name, features);
                root.deps[i] = DepSpec::Detailed(DepDetailed {
                    name,
                    features,
                    ..Default::default()
                });
                fixed = true;
            }
        }
    }
    fixed
}

/// Returns true if any duplicates were removed.
/// When merging duplicates, keeps the most complete version:
/// Detailed with more fields wins over Detailed with fewer; Detailed wins over Simple.
fn dedup_deps(root: &mut TritonRoot) -> bool {
    let mut best: std::collections::HashMap<String, DepSpec> = std::collections::HashMap::new();
    let mut order: Vec<String> = Vec::new();

    for dep in &root.deps {
        let name = dep.name().to_string();
        let existing = best.get(&name);
        let should_replace = match (existing, dep) {
            (None, _) => true,
            // Detailed always beats Simple
            (Some(DepSpec::Simple(_)), DepSpec::Detailed(_)) => true,
            // Among Detailed, merge features and prefer one with package set
            (Some(DepSpec::Detailed(old)), DepSpec::Detailed(new)) => {
                let mut merged = old.clone();
                for f in &new.features {
                    if !merged.features.contains(f) {
                        merged.features.push(f.clone());
                    }
                }
                if merged.package.is_none() && new.package.is_some() {
                    merged.package = new.package.clone();
                }
                best.insert(name.clone(), DepSpec::Detailed(merged));
                false // already inserted via merge
            }
            // Don't downgrade Detailed to Simple
            (Some(DepSpec::Detailed(_)), DepSpec::Simple(_)) => false,
            // Git deps: keep latest
            (Some(_), DepSpec::Git(_)) => true,
            _ => false,
        };
        if should_replace {
            if !order.contains(&name) {
                order.push(name.clone());
            }
            best.insert(name, dep.clone());
        } else if !order.contains(&name) {
            order.push(name);
        }
    }

    let deduped: Vec<DepSpec> = order.into_iter()
        .filter_map(|name| best.remove(&name))
        .collect();

    if deduped.len() < root.deps.len() {
        let removed = root.deps.len() - deduped.len();
        eprintln!("Removed {} duplicate dep(s) from triton.json", removed);
        root.deps = deduped;
        true
    } else {
        root.deps = deduped;
        false
    }
}


