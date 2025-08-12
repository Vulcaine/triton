use anyhow::Result;
use std::fs;

use crate::cmake::{regenerate_root_cmake, rewrite_component_cmake};
use crate::models::{
    dep_eq, Dependency, DependencyDetail, TritonComponent, TritonRoot, VcpkgManifest,
};
use crate::util::{
    read_json, run, vcpkg_exe_path, write_json_pretty_changed, write_text_if_changed,
};
use crate::templates::component_cmakelists;

pub fn handle_add(pkg: &str, component: &str, features: Option<&str>, host: bool) -> Result<()> {
    // Load root metadata
    let mut root: TritonRoot = read_json("triton.json")?;
    if !root.components.contains_key(component) {
        // scaffold new component
        fs::create_dir_all(format!("components/{component}/src"))?;
        fs::create_dir_all(format!("components/{component}/include"))?;
        let _ = write_text_if_changed(
            &format!("components/{component}/CMakeLists.txt"),
            &component_cmakelists(),
        )?;
        root.components.insert(
            component.into(),
            TritonComponent { kind: "lib".into(), deps: vec![] },
        );
    }

    // Update component deps (triton.json)
    {
        let comp = root.components.get_mut(component).unwrap();
        if !comp.deps.iter().any(|d| d == pkg) {
            comp.deps.push(pkg.to_string());
        }
    }
    let _ = write_json_pretty_changed("triton.json", &root)?;
    let _ = write_json_pretty_changed(
        &format!("components/{component}/triton.json"),
        root.components.get(component).unwrap(),
    )?;

    // Update vcpkg.json (manifest)
    let mut mani: VcpkgManifest = read_json("vcpkg.json")?;
    let feats: Vec<String> = features
        .unwrap_or_default()
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    // Create the dependency record (host flag lives here)
    let dep = if feats.is_empty() && !host {
        Dependency::Name(pkg.into())
    } else {
        Dependency::Detailed(DependencyDetail {
            name: pkg.into(),
            features: feats,
            default: None,
            host: if host { Some(true) } else { None },
        })
    };

    if !mani.dependencies.iter().any(|d| dep_eq(d, &dep)) {
        mani.dependencies.push(dep);
    }
    let _ = write_json_pretty_changed("vcpkg.json", &mani)?;

    // vcpkg install (manifest mode)
    let vcpkg_bin = vcpkg_exe_path();
    eprintln!("Running vcpkg install (manifest mode)...");
    run(&vcpkg_bin, &["install", "--clean-after-build"], ".")?;

    // Rewrite CMake for this component and root
    rewrite_component_cmake(component, root.components.get(component).unwrap())?;
    regenerate_root_cmake(&root)?;

    eprintln!("Added '{}' to component '{}'.", pkg, component);
    Ok(())
}
