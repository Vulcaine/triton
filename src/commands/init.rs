use anyhow::{Context, Result};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use crate::models::{TritonComponent, TritonRoot, VcpkgManifest};
use crate::templates::{cmake_presets, component_cmakelists, root_cmakelists};
use crate::tools::ensure_ninja_dir;
use crate::util::{run, write_json_pretty_changed, write_text_if_changed, Change};

pub fn handle_init(
    name_opt: Option<&str>,
    triplet: &str,
    generator: &str,
    cxx_std: &str,
) -> Result<()> {
    // Resolve absolute project_dir
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

    // If generator is Ninja, try to resolve it NOW (non-fatal)
    if generator.eq_ignore_ascii_case("ninja") {
        if let Err(e) = ensure_ninja_dir(&project_dir) {
            eprintln!("Warning: Ninja not immediately available ({e}). Build will try a portable fallback.");
        }
    }

    // Project name
    let project_name: String = match name_opt {
        Some(n) => n.to_string(),
        None => project_dir.file_name().and_then(|s| s.to_str()).unwrap_or("MyProject").to_string(),
    };

    let mut changes: Vec<(String, Change)> = Vec::new();

    // vcpkg clone if missing
    let vcpkg_dir = Path::new("vcpkg");
    if !vcpkg_dir.exists() {
        eprintln!("Cloning vcpkg...");
        run("git", &["clone", "https://github.com/microsoft/vcpkg.git", "vcpkg"], ".")?;
        changes.push(("vcpkg/ (git clone)".into(), Change::Created));
    } else {
        changes.push(("vcpkg/".into(), Change::Unchanged));
    }

    // folders & sample
    fs::create_dir_all("components/app/src")?;
    fs::create_dir_all("components/app/include")?;
    let hello = r#"#include <iostream>
int main() { std::cout << "Hello from triton app!\n"; return 0; }
"#;
    changes.push((
        "components/app/src/main.cpp".into(),
        write_text_if_changed("components/app/src/main.cpp", hello)?,
    ));

    // metadata
    let mut root = TritonRoot::default();
    root.triplet = triplet.to_string();
    root.generator = generator.to_string();
    root.cxx_std = cxx_std.to_string();
    root.components.insert(
        "app".into(),
        TritonComponent { kind: "exe".into(), deps: vec![] },
    );
    changes.push(("triton.json".into(), write_json_pretty_changed("triton.json", &root)?));

    changes.push((
        "components/app/triton.json".into(),
        write_json_pretty_changed(
            "components/app/triton.json",
            &TritonComponent { kind: "exe".into(), deps: vec![] },
        )?,
    ));

    // vcpkg manifest (no hostDependencies — host tools live inside dependencies with "host": true)
    let manifest = VcpkgManifest {
        name: project_name.clone(),
        version: "0.0.0".into(),
        dependencies: vec![],
    };
    changes.push(("vcpkg.json".into(), write_json_pretty_changed("vcpkg.json", &manifest)?));

    // CMake files
    changes.push((
        "CMakeLists.txt".into(),
        write_text_if_changed("CMakeLists.txt", &root_cmakelists())?,
    ));
    changes.push((
        "components/app/CMakeLists.txt".into(),
        write_text_if_changed("components/app/CMakeLists.txt", &component_cmakelists())?,
    ));
    changes.push((
        "CMakePresets.json".into(),
        write_text_if_changed(
            "CMakePresets.json",
            &cmake_presets(&project_name, generator, triplet),
        )?,
    ));

    eprintln!("\nChanges in {}:", project_dir.display());
    for (path, ch) in &changes {
        eprintln!("  - {:<40} {:?}", path, ch);
    }
    eprintln!("\nInitialized project '{}'.", project_name);
    Ok(())
}
