// src/commands/init.rs
use anyhow::{Context, Result};
use std::env;
use std::fs;
use std::path::Path;

use crate::models::{RootDep, TritonComponent, TritonRoot};
use crate::templates::{cmake_presets, component_cmakelists, components_dir_cmakelists};
use crate::tools::ensure_ninja_dir;
use crate::util::{run, write_json_pretty_changed, write_text_if_changed, Change};

pub fn handle_init(name_opt: Option<&str>, triplet: &str, generator: &str, cxx_std: &str) -> Result<()> {
    // Determine project directory
    let cwd = env::current_dir().context("cannot get current directory")?;
    let (project_dir, minimal_mode) = match name_opt {
        // `triton init .` or just `triton init` => minimal init in current directory, no component scaffold
        Some(".") | None => (cwd.clone(), true),
        // `triton init NAME` => create a new subfolder project with scaffold
        Some(name) => (cwd.join(name), false),
    };

    if !project_dir.exists() {
        fs::create_dir_all(&project_dir)?;
    }
    env::set_current_dir(&project_dir)
        .with_context(|| format!("cd into {}", project_dir.display()))?;

    if generator.eq_ignore_ascii_case("ninja") {
        let _ = ensure_ninja_dir(&project_dir);
    }

    // app name from folder name (even for minimal mode)
    let app_name: String = project_dir
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("MyProject")
        .to_string();

    let mut changes: Vec<(String, Change)> = Vec::new();

    // Minimal mode: write triton.json + components/CMakeLists.txt + components/CMakePresets.json
    if minimal_mode {
        fs::create_dir_all("components")?;
        changes.push((
            "components/CMakeLists.txt".into(),
            write_text_if_changed("components/CMakeLists.txt", &components_dir_cmakelists())?,
        ));
        changes.push((
            "components/CMakePresets.json".into(),
            write_text_if_changed(
                "components/CMakePresets.json",
                &cmake_presets(&app_name, generator, triplet),
            )?,
        ));

        let mut root = TritonRoot::default();
        root.app_name = app_name.clone();
        root.triplet = triplet.to_string();
        root.generator = generator.to_string();
        root.cxx_std = cxx_std.to_string();
        root.deps = Vec::<RootDep>::new();
        // No auto component scaffold
        changes.push(("triton.json".into(), write_json_pretty_changed("triton.json", &root)?));

        eprintln!("\nInitialized Triton in existing project '{}'.", app_name);
        eprintln!("• Created triton.json, components/CMakeLists.txt, components/CMakePresets.json");
        eprintln!("• Put your existing components under ./components/<name> with their own CMakeLists.txt");
        eprintln!("• You can add new ones with: triton link App->Lib, triton add <dep>, etc.");
        return Ok(());
    }

    // NEW PROJECT scaffold mode (full)
    // vcpkg clone if missing (still at repo root)
    if !Path::new("vcpkg").exists() {
        eprintln!("Cloning vcpkg...");
        run("git", &["clone", "https://github.com/microsoft/vcpkg.git", "vcpkg"], ".")?;
        changes.push(("vcpkg/ (git clone)".into(), Change::Created));
    } else {
        changes.push(("vcpkg/".into(), Change::Unchanged));
    }

    // components/ root files
    fs::create_dir_all("components")?;
    changes.push((
        "components/CMakeLists.txt".into(),
        write_text_if_changed("components/CMakeLists.txt", &components_dir_cmakelists())?,
    ));
    changes.push((
        "components/CMakePresets.json".into(),
        write_text_if_changed(
            "components/CMakePresets.json",
            &cmake_presets(&app_name, generator, triplet),
        )?,
    ));

    // folders & sample for the executable component
    fs::create_dir_all(format!("components/{}/src", app_name))?;
    fs::create_dir_all(format!("components/{}/include", app_name))?;
    let hello = r#"#include <iostream>
int main() { std::cout << "Hello from triton app!\n"; return 0; }
"#;
    changes.push((
        format!("components/{}/src/main.cpp", app_name),
        write_text_if_changed(format!("components/{}/src/main.cpp", app_name), hello)?,
    ));
    changes.push((
        format!("components/{}/CMakeLists.txt", app_name),
        write_text_if_changed(
            format!("components/{}/CMakeLists.txt", app_name),
            &component_cmakelists(),
        )?,
    ));

    // root metadata
    let mut root = TritonRoot::default();
    root.app_name = app_name.clone();
    root.triplet = triplet.to_string();
    root.generator = generator.to_string();
    root.cxx_std = cxx_std.to_string();
    root.deps = Vec::<RootDep>::new();
    root.components
        .insert(app_name.clone(), TritonComponent { kind: "exe".into(), link: vec![], defines: vec![] });

    changes.push(("triton.json".into(), write_json_pretty_changed("triton.json", &root)?));

    // vcpkg manifest (empty to start)
    let manifest =
        serde_json::json!({ "name": app_name, "version": "0.0.0", "dependencies": [] });
    changes.push((
        "vcpkg.json".into(),
        write_text_if_changed("vcpkg.json", &serde_json::to_string_pretty(&manifest)?)?,
    ));

    eprintln!("\nInitialized project '{}'.", app_name);
    Ok(())
}
