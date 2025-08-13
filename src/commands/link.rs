use anyhow::Result;

use crate::cmake::{regenerate_root_cmake, rewrite_component_cmake};
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

    // Guard against ambiguous usage: both sides are deps (nothing to link in components)
    if from_is_dep && to_is_dep {
        anyhow::bail!(
            "Both '{}' and '{}' are listed as deps; nothing to link as components.\n\
             Use `triton add {0}->{1}` to link a dep to a component, or pick a component name.",
            from, to
        );
    }

    // Helper to create component entry in root.components with default kind if missing
    let mut ensure_component_entry = |name: &str| {
        if !root.components.contains_key(name) {
            root.components.insert(
                name.to_string(),
                TritonComponent {
                    kind: "lib".into(),
                    link: vec![],
                    defines: vec![]
                },
            );
        }
    };

    // Track which components’ CMake we need to rewrite
    let mut touched_components: Vec<String> = Vec::new();

    if from_is_dep {
        // Case: link DEP 'from' into COMPONENT 'to'
        if to_is_dep {
            // already guarded above, but double safety
            anyhow::bail!(
                "Cannot link dep '{}' into dep '{}'. '{}' must be a component.",
                from, to, to
            );
        }

        // Ensure component 'to' scaffold & entry
        ensure_component_scaffold(to)?;
        ensure_component_entry(to);

        // Add link to 'to' if not present
        {
            let to_comp = root.components.get_mut(to).expect("component to exists");
            if !has_link_to_name(to_comp, from) {
                to_comp.link.push(LinkEntry::Name(from.into()));
                touched_components.push(to.to_string());
            }
        }
    } else {
        // Case: link COMPONENT 'from' to COMPONENT or DEP 'to'
        if !to_is_dep {
            // to is a component; ensure both scaffolds & entries
            ensure_component_scaffold(from)?;
            ensure_component_scaffold(to)?;
            ensure_component_entry(from);
            ensure_component_entry(to);
        } else {
            // to is a dep; only from is a component to scaffold/ensure
            ensure_component_scaffold(from)?;
            ensure_component_entry(from);
        }

        // Add link 'from' -> 'to' if missing
        {
            let from_comp = root.components.get_mut(from).expect("component from exists");
            if !has_link_to_name(from_comp, to) {
                from_comp.link.push(LinkEntry::Name(to.into()));
                touched_components.push(from.to_string());
            }
        }
    }

    // Persist triton.json
    write_json_pretty_changed("triton.json", &root)?;

    // Rewrite CMake for touched components
    for name in touched_components.iter() {
        if let Some(c) = root.components.get(name) {
            rewrite_component_cmake(name, &root, c)?;
        }
    }

    // Regenerate the root (helpers + subdirs)
    regenerate_root_cmake(&root)?;

    // Message
    if from_is_dep {
        eprintln!("Linked dep '{}' -> component '{}'.", from, to);
    } else if to_is_dep {
        eprintln!("Linked component '{}' -> dep '{}'.", from, to);
    } else {
        eprintln!("Linked component '{}' -> component '{}'.", from, to);
    }

    Ok(())
}
