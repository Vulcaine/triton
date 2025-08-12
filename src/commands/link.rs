// src/commands/link.rs
use anyhow::{bail, Context, Result};
use std::fs;
use std::path::Path;

use crate::cmake::{regenerate_root_cmake, rewrite_component_cmake};
use crate::models::{RootDep, TritonComponent, TritonRoot};
use crate::templates::component_cmakelists;
use crate::util::{read_json, write_json_pretty_changed, write_text_if_changed};

fn ensure_component_scaffold(name: &str) -> Result<()> {
    let base = format!("components/{name}");
    fs::create_dir_all(format!("{base}/src"))?;
    fs::create_dir_all(format!("{base}/include"))?;
    let cm_path = format!("{base}/CMakeLists.txt");
    if !Path::new(&cm_path).exists() {
        write_text_if_changed(&cm_path, &component_cmakelists())
            .with_context(|| format!("writing {}", cm_path))?;
    }
    Ok(())
}

/// Returns true if `name` is a project-level dependency (vcpkg or git).
fn is_dep_name(root: &TritonRoot, name: &str) -> bool {
    root.deps.iter().any(|d| match d {
        RootDep::Name(n) => n == name,
        RootDep::Git(g) => g.name == name || g.repo == name,
    })
}

/// Link one component to another (records **inbound** on the provider).
/// Example: `UI -> Engine` will store `"UI"` inside `Engine.link`.
fn link_component_to_component(root: &mut TritonRoot, consumer: &str, provider: &str) -> Result<()> {
    ensure_component_scaffold(consumer)?;
    ensure_component_scaffold(provider)?;

    root.components
        .entry(consumer.to_string())
        .or_insert(TritonComponent { kind: "lib".into(), link: vec![] });

    let prov = root
        .components
        .entry(provider.to_string())
        .or_insert(TritonComponent { kind: "lib".into(), link: vec![] });

    if !prov.link.iter().any(|x| x == consumer) {
        prov.link.push(consumer.to_string());
    }

    // Persist + regen
    write_json_pretty_changed("triton.json", root)?;
    if let Some(c) = root.components.get(consumer) {
        rewrite_component_cmake(consumer, root, c)?;
    }
    if let Some(p) = root.components.get(provider) {
        rewrite_component_cmake(provider, root, p)?;
    }
    regenerate_root_cmake(root)?;
    eprintln!("Linked component '{}' -> '{}' (recorded inbound on '{}').", consumer, provider, provider);
    Ok(())
}

/// Link a **dependency** to a component. Example: `lua:Scripting`
/// We record it on the **consumer** component (since deps aren’t components).
fn link_dep_to_component(root: &mut TritonRoot, dep: &str, consumer: &str) -> Result<()> {
    if !is_dep_name(root, dep) {
        bail!("Unknown dependency '{}'. Add it first with `triton add {}`.", dep, dep);
    }

    ensure_component_scaffold(consumer)?;
    let comp = root
        .components
        .entry(consumer.to_string())
        .or_insert(TritonComponent { kind: "lib".into(), link: vec![] });

    if !comp.link.iter().any(|x| x == dep) {
        comp.link.push(dep.to_string());
    }

    write_json_pretty_changed("triton.json", root)?;
    if let Some(c) = root.components.get(consumer) {
        rewrite_component_cmake(consumer, root, c)?;
    }
    regenerate_root_cmake(root)?;
    eprintln!("Linked dependency '{}' -> component '{}'.", dep, consumer);
    Ok(())
}

/// Public entry used by `main.rs` after parsing.
/// `left` and `right` are already split (supporting 'A B', 'A->B', or 'dep:Comp').
pub fn handle_link(left: &str, right: &str) -> Result<()> {
    let mut root: TritonRoot = read_json("triton.json")?;

    // If left is a known dep, treat as dep:component.
    if is_dep_name(&root, left) {
        return link_dep_to_component(&mut root, left, right);
    }

    // Otherwise, both are components (consumer -> provider),
    // and we record inbound on the provider.
    link_component_to_component(&mut root, left, right)
}
