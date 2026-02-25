use anyhow::Result;

use crate::cmake::{dep_is_active, detect_vcpkg_triplet, effective_cmake_version, regenerate_root_cmake, rewrite_component_cmake};
use crate::models::{LinkEntry, TritonComponent, TritonRoot};
use crate::util::{
    ensure_component_scaffold, has_link_to_name, is_dep, read_json, write_json_pretty_changed,
};

/// Link one component to another (adds dependency edge).
/// Behavior:
///   triton link A:B
///   - If A is a dep (in root.deps): link dep A -> component B (create B scaffold if needed).
///   - If A is not a dep: link component A -> component B (create scaffolds if needed).
pub fn handle_link(from: &str, to: &str) -> Result<()> {
    // Load current project state
    let mut root: TritonRoot = read_json("triton.json")?;

    let from_is_dep = is_dep(&root, from);
    let to_is_dep = is_dep(&root, to);

    // RHS ('to') must be a component
    if to_is_dep {
        anyhow::bail!(
            "Right-hand side '{}' is a dep. `triton link A:B` means 'B depends on A'. \
             The right-hand side must be a component.",
            to
        );
    }

    // Helper to ensure a component entry exists (default "lib") + scaffold
    let mut ensure_component_entry = |name: &str| {
        if !root.components.contains_key(name) {
            root.components.insert(
                name.to_string(),
                TritonComponent {
                    kind: "lib".into(),
                    link: vec![],
                    defines: vec![],
                    exports: vec![],
                    resources: vec![],
                    link_options: Default::default(),
                    vendor_libs: vec![],
                    assets: vec![],
                },
            );
        }
        ensure_component_scaffold(name)
    };

    // 'to' must be a component (create if missing)
    ensure_component_entry(to)?;

    // 'from' can be a dep or component
    if !from_is_dep {
        ensure_component_entry(from)?;
    }

    // --- validate dep applicability ---
    if from_is_dep {
        let host_os = std::env::consts::OS;
        let triplet = detect_vcpkg_triplet();
        let active = root
            .deps
            .iter()
            .any(|d| dep_is_active(d, from, host_os, &triplet));

        if !active {
            eprintln!(
                "Warning: dep '{}' is not active for this platform/triplet. Skipping link.",
                from
            );
            return Ok(()); // skip adding
        }
    }

    // Add: B (to) depends on A (from)
    {
        let to_comp = root.components.get_mut(to).expect("component 'to' exists");
        if !has_link_to_name(to_comp, from) {
            to_comp.link.push(LinkEntry::Name(from.into()));
        }
    }

    // Persist triton.json
    write_json_pretty_changed("triton.json", &root)?;

    let cmake_ver = effective_cmake_version();
    // Rewrite CMake for 'to' (and 'from' if new component)
    if let Some(c) = root.components.get(to) {
        rewrite_component_cmake(to, &root, c, cmake_ver)?;
    }
    if !from_is_dep {
        if let Some(c) = root.components.get(from) {
            rewrite_component_cmake(from, &root, c, cmake_ver)?;
        }
    }

    // Regenerate the root
    regenerate_root_cmake(&root)?;

    if from_is_dep {
        eprintln!("Linked component '{}' to depend on dep '{}'.", to, from);
    } else {
        eprintln!("Linked component '{}' to depend on component '{}'.", to, from);
    }

    Ok(())
}
