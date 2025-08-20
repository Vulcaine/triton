use anyhow::Result;
use serde_json::json;

use crate::cmake::{detect_vcpkg_triplet, regenerate_root_cmake, rewrite_component_cmake};
use crate::models::{DepSpec, TritonRoot};
use crate::templates::cmake_presets;
use crate::util::{read_json, write_text_if_changed};

pub fn handle_generate() -> Result<()> {
    eprintln!("Regenerating project CMake files...");
    let root: TritonRoot = read_json("triton.json")?;
    for (name, comp) in &root.components {
        rewrite_component_cmake(name, &root, comp)?;
    }
    regenerate_root_cmake(&root)?;

    // Always refresh CMakePresets.json for the host platform
    let trip = detect_vcpkg_triplet();
    eprintln!("detected triplet: {}", trip);

    write_text_if_changed(
        "components/CMakePresets.json",
        &cmake_presets(&root.app_name, &root.generator, &trip),
    )?;

    // --- regenerate vcpkg.json ---
    let mut deps: Vec<String> = Vec::new();
    for dep in &root.deps {
        match dep {
            DepSpec::Simple(s) => deps.push(s.clone()),
            DepSpec::Git(_) => { /* git deps not in vcpkg manifest */ }
            DepSpec::Detailed(d) => {
                let mut spec = d.name.clone();
                if !d.features.is_empty() {
                    spec.push(':');
                    spec.push_str(&d.features.join(","));
                }
                deps.push(spec);
            }
        }
    }

    let vcpkg_manifest = json!({
        "name": root.app_name,
        "version": "0.1.0",
        "dependencies": deps,
    });

    let text = serde_json::to_string_pretty(&vcpkg_manifest)?;
    write_text_if_changed("vcpkg.json", &text)?;

    eprintln!("Regenerated CMake files and vcpkg.json.");
    Ok(())
}
