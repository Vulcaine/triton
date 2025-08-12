use anyhow::{Context, Result};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use crate::models::{TritonComponent, TritonRoot};
use crate::templates::{cmake_presets, component_cmakelists, root_cmakelists};
use crate::tools::ensure_ninja_dir;
use crate::util::{run, write_json_pretty_changed, write_text_if_changed, Change};
use crate::models::RootDep;

pub fn handle_init(name_opt: Option<&str>, triplet: &str, generator: &str, cxx_std: &str) -> Result<()> {
    // project dir
    let project_dir: PathBuf = {
        let cwd = env::current_dir().context("cannot get current directory")?;
        if let Some(name) = name_opt { cwd.join(name) } else { cwd }
    };
    if name_opt.is_some() && !project_dir.exists() {
        fs::create_dir_all(&project_dir)?;
    }
    if name_opt.is_some() {
        env::set_current_dir(&project_dir)
            .with_context(|| format!("cd into {}", project_dir.display()))?;
    }

    if generator.eq_ignore_ascii_case("ninja") {
        let _ = ensure_ninja_dir(&project_dir);
    }

    // app name
    let app_name: String = match name_opt {
        Some(n) => n.to_string(),
        None => project_dir.file_name().and_then(|s| s.to_str()).unwrap_or("MyProject").to_string(),
    };

    let mut changes: Vec<(String, Change)> = Vec::new();

    // vcpkg clone if missing
    if !Path::new("vcpkg").exists() {
        eprintln!("Cloning vcpkg...");
        run("git", &["clone", "https://github.com/microsoft/vcpkg.git", "vcpkg"], ".")?;
        changes.push(("vcpkg/ (git clone)".into(), Change::Created));
    } else {
        changes.push(("vcpkg/".into(), Change::Unchanged));
    }

    // folders & sample
    fs::create_dir_all(format!("components/{}/src", app_name))?;
    fs::create_dir_all(format!("components/{}/include", app_name))?;
    let hello = r#"#include <iostream>
int main() { std::cout << "Hello from triton app!\n"; return 0; }
"#;
    changes.push((
        format!("components/{}/src/main.cpp", app_name),
        write_text_if_changed(format!("components/{}/src/main.cpp", app_name), hello)?,
    ));

    // root metadata only
    let mut root = TritonRoot::default();
    root.app_name = app_name.clone();
    root.triplet = triplet.to_string();
    root.generator = generator.to_string();
    root.cxx_std = cxx_std.to_string();
    root.deps = Vec::<RootDep>::new();
    root.components.insert(app_name.clone(), TritonComponent { kind: "exe".into(), link: vec![] });

    changes.push(("triton.json".into(), write_json_pretty_changed("triton.json", &root)?));

    // vcpkg manifest (empty to start)
    let manifest = serde_json::json!({ "name": app_name, "version": "0.0.0", "dependencies": [] });
    changes.push(("vcpkg.json".into(), write_text_if_changed("vcpkg.json", &serde_json::to_string_pretty(&manifest)?)?));

    // CMake files
    changes.push(("CMakeLists.txt".into(), write_text_if_changed("CMakeLists.txt", &root_cmakelists(&app_name))?));
    changes.push((
        format!("components/{}/CMakeLists.txt", app_name),
        write_text_if_changed(format!("components/{}/CMakeLists.txt", app_name), &component_cmakelists())?,
    ));
    changes.push(("CMakePresets.json".into(), write_text_if_changed("CMakePresets.json", &cmake_presets(&app_name, generator, triplet))?));

    eprintln!("\nInitialized project '{}'.", app_name);
    Ok(())
}
