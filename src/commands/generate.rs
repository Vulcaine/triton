use anyhow::Result;
use serde_json::json;

use crate::cmake::{
    dep_is_active, detect_vcpkg_triplet, effective_cmake_version, regenerate_root_cmake, rewrite_component_cmake
};
use crate::models::{DepSpec, TritonRoot};
use crate::templates::cmake_presets;
use crate::util::{read_json, validate_triton_root, write_text_if_changed};

pub fn handle_generate() -> Result<()> {
    eprintln!("Regenerating project CMake + vcpkg manifest...");
    let root: TritonRoot = read_json("triton.json")?;

    validate_triton_root(&root)?;

    let cmake_ver = effective_cmake_version();

    // Rewrite all component CMakeLists
    for (name, comp) in &root.components {
        rewrite_component_cmake(name, &root, comp, cmake_ver)?;
    }

    // Root CMake
    regenerate_root_cmake(&root)?;

    // --- regenerate CMakePresets.json ---
    let trip = detect_vcpkg_triplet();
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

    eprintln!("Regenerated CMake files and vcpkg.json.");
    Ok(())
}
