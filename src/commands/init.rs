use anyhow::{Context, Result};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use crate::cmake::{regenerate_root_cmake, rewrite_component_cmake};
use crate::models::{TritonComponent, TritonRoot, VcpkgManifest};
use crate::templates::{cmake_presets};
use crate::tools::ensure_ninja_dir;
use crate::util::{write_json_pretty_changed, write_text_if_changed, Change};

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

    // Project/app name
    let app_name: String = match name_opt {
        Some(n) => n.to_string(),
        None => project_dir
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("MyProject")
            .to_string(),
    };

    let mut changes: Vec<(String, Change)> = Vec::new();

    // vcpkg clone if missing
    let vcpkg_dir = Path::new("vcpkg");
    if !vcpkg_dir.exists() {
        eprintln!("Cloning vcpkg...");
        // clone into ./vcpkg inside project
        let out = std::process::Command::new("git")
            .args(["clone", "https://github.com/microsoft/vcpkg.git", "vcpkg"])
            .current_dir(".")
            .status()
            .context("failed to spawn git")?;
        if !out.success() {
            anyhow::bail!("git clone vcpkg failed");
        }
        changes.push(("vcpkg/ (git clone)".into(), Change::Created));
    } else {
        changes.push(("vcpkg/".into(), Change::Unchanged));
    }

    // folders & sample (component named after app)
    let comp_dir = format!("components/{app_name}");
    fs::create_dir_all(format!("{comp_dir}/src"))?;
    fs::create_dir_all(format!("{comp_dir}/include"))?;
    let hello = r#"#include <iostream>
int main() { std::cout << "Hello from triton app!\n"; return 0; }
"#;
    changes.push((
        format!("{comp_dir}/src/main.cpp"),
        write_text_if_changed(format!("{comp_dir}/src/main.cpp"), hello)?,
    ));

    // metadata
    let mut root = TritonRoot::default();
    root.app_name = app_name.clone();
    root.triplet = triplet.to_string();
    root.generator = generator.to_string();
    root.cxx_std = cxx_std.to_string();
    root.components.insert(
        app_name.clone(),
        TritonComponent {
            kind: "exe".into(),
            deps: vec![],
            comps: vec![],
            git: vec![],
        },
    );

    changes.push((
        "triton.json".into(),
        write_json_pretty_changed("triton.json", &root)?,
    ));

    changes.push((
        format!("{comp_dir}/triton.json"),
        write_json_pretty_changed(
            format!("{comp_dir}/triton.json"),
            root.components.get(&app_name).unwrap(),
        )?,
    ));

    // vcpkg manifest
    let manifest = VcpkgManifest {
        name: app_name.clone(),
        version: "0.0.0".into(),
        dependencies: vec![],
    };
    changes.push((
        "vcpkg.json".into(),
        write_json_pretty_changed("vcpkg.json", &manifest)?,
    ));

    // Root CMakeLists.txt (regenerate from metadata so it has the correct app name)
    regenerate_root_cmake(&root)?;

    // Component CMakeLists.txt (basic scaffold)
    rewrite_component_cmake(&app_name, root.components.get(&app_name).unwrap())?;

    // CMakePresets.json
    let presets = cmake_presets(&app_name, generator, triplet);
    changes.push((
        "CMakePresets.json".into(),
        write_text_if_changed("CMakePresets.json", &presets)?,
    ));

    eprintln!("\nChanges in {}:", project_dir.display());
    for (path, ch) in &changes {
        eprintln!("  - {:<40} {:?}", path, ch);
    }
    eprintln!("\nInitialized project '{}'.", app_name);
    Ok(())
}
