use anyhow::{Context, Result};
use std::env;
use std::fs;
use std::path::Path;

use crate::cmake::{detect_vcpkg_triplet, effective_cmake_version};
use crate::models::{DepSpec, LinkEntry, TritonComponent, TritonRoot};
use crate::templates::{cmake_presets, component_cmakelists, components_dir_cmakelists};
use crate::tools::ensure_ninja_dir;
use crate::util::{run, write_json_pretty_changed, write_text_if_changed, Change};
use crate::util; // for read_json

pub fn handle_init(
    name_opt: Option<&str>,
    generator: &str,
    cxx_std: &str,
) -> Result<()> {
    let cwd = env::current_dir().context("cannot get current directory")?;

    let minimal_mode = matches!(name_opt, None | Some("."));
    let project_dir = if minimal_mode {
        cwd.clone()
    } else {
        cwd.join(name_opt.unwrap())
    };

    if !project_dir.exists() {
        fs::create_dir_all(&project_dir)?;
    }

    // Ensure Ninja if requested
    if generator.eq_ignore_ascii_case("ninja") {
        let _ = ensure_ninja_dir(&project_dir);
    }

    let app_name: String = project_dir
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("MyProject")
        .to_string();

    let mut _changes: Vec<(String, Change)> = Vec::new();

    setup_project_dirs(&project_dir, &mut _changes)?;
    write_initial_templates(&project_dir, &app_name, generator, &mut _changes)?;

    let mut root = load_or_seed_root(&project_dir, &app_name, generator, cxx_std)?;

    if !minimal_mode {
        scaffold_app_component(&project_dir, &app_name, &mut root, &mut _changes)?;
    }

    discover_existing_components(&project_dir, &mut root);
    setup_tests_component(&project_dir, &mut root, &mut _changes)?;

    _changes.push((
        "triton.json".into(),
        write_json_pretty_changed(project_dir.join("triton.json"), &root)?,
    ));

    setup_vcpkg_json(&project_dir, &app_name, &mut _changes)?;
    clone_vcpkg_if_missing(&project_dir, &mut _changes)?;

    if minimal_mode {
        eprintln!(
            "\nInitialized Triton in existing project '{}'. (tests and core ensured)",
            app_name
        );
    } else {
        eprintln!("\nInitialized project '{}' (app + tests ensured).", app_name);
    }

    Ok(())
}

/// Creates the `components/` directory and its root `CMakeLists.txt`.
fn setup_project_dirs(
    project_dir: &Path,
    changes: &mut Vec<(String, Change)>,
) -> Result<()> {
    let comps = project_dir.join("components");
    fs::create_dir_all(&comps)?;
    let cm_path = comps.join("CMakeLists.txt");
    changes.push((
        "components/CMakeLists.txt".into(),
        write_text_if_changed(&cm_path, &components_dir_cmakelists())?,
    ));
    Ok(())
}

/// Writes `CMakePresets.json` into `components/`.
fn write_initial_templates(
    project_dir: &Path,
    app_name: &str,
    generator: &str,
    changes: &mut Vec<(String, Change)>,
) -> Result<()> {
    let cmake_ver = effective_cmake_version();
    let triplet = detect_vcpkg_triplet();
    let presets_path = project_dir.join("components/CMakePresets.json");
    changes.push((
        "components/CMakePresets.json".into(),
        write_text_if_changed(
            &presets_path,
            &cmake_presets(app_name, generator, &triplet, cmake_ver),
        )?,
    ));
    Ok(())
}

/// Loads an existing `triton.json` or creates a default one.
fn load_or_seed_root(
    project_dir: &Path,
    app_name: &str,
    generator: &str,
    cxx_std: &str,
) -> Result<TritonRoot> {
    let triton_json = project_dir.join("triton.json");
    let mut root: TritonRoot = if triton_json.exists() {
        util::read_json(&triton_json)?
    } else {
        let mut r = TritonRoot::default();
        r.app_name = app_name.to_string();
        r.generator = generator.to_string();
        r.cxx_std = cxx_std.to_string();
        r
    };

    if root.app_name.is_empty() {
        root.app_name = app_name.to_string();
    }
    if root.generator.is_empty() {
        root.generator = generator.to_string();
    }
    if root.cxx_std.is_empty() {
        root.cxx_std = cxx_std.to_string();
    }

    Ok(root)
}

/// Creates the main app component scaffold (src/main.cpp, CMakeLists.txt)
/// when not in minimal mode.
fn scaffold_app_component(
    project_dir: &Path,
    app_name: &str,
    root: &mut TritonRoot,
    changes: &mut Vec<(String, Change)>,
) -> Result<()> {
    let comp_dir = project_dir.join("components").join(app_name);
    if !comp_dir.exists() {
        fs::create_dir_all(comp_dir.join("src"))?;
        fs::create_dir_all(comp_dir.join("include"))?;
        let hello = r#"#include <iostream>
int main() { std::cout << "Hello from triton app!\n"; return 0; }
"#;
        let main_cpp = comp_dir.join("src/main.cpp");
        if !main_cpp.exists() {
            changes.push((
                main_cpp.display().to_string(),
                write_text_if_changed(&main_cpp, hello)?,
            ));
        }
        let comp_cmake = comp_dir.join("CMakeLists.txt");
        if !comp_cmake.exists() {
            changes.push((
                comp_cmake.display().to_string(),
                write_text_if_changed(&comp_cmake, &component_cmakelists(false))?,
            ));
        }
        root.components
            .entry(app_name.to_string())
            .or_insert_with(|| TritonComponent {
                kind: "exe".into(),
                ..Default::default()
            });
    }
    Ok(())
}

/// Scans `components/` for existing directories with a `CMakeLists.txt`
/// and registers them in the root manifest.
fn discover_existing_components(project_dir: &Path, root: &mut TritonRoot) {
    let components_path = project_dir.join("components");
    if components_path.is_dir() {
        if let Ok(entries) = fs::read_dir(&components_path) {
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_dir() { continue; }
                let name = match path.file_name().and_then(|n| n.to_str()) {
                    Some(n) => n.to_string(),
                    None => continue,
                };
                // Skip tests — handled separately below
                if name == "tests" { continue; }
                // Only register if it has a CMakeLists.txt
                if !path.join("CMakeLists.txt").exists() { continue; }
                // Detect kind: exe if main.cpp exists, else lib
                let kind = if path.join("src/main.cpp").exists() { "exe" } else { "lib" };
                root.components
                    .entry(name)
                    .or_insert_with(|| TritonComponent {
                        kind: kind.into(),
                        ..Default::default()
                    });
            }
        }
    }
}

/// Creates the `tests` component with a gtest scaffold and registers the
/// gtest dependency.
fn setup_tests_component(
    project_dir: &Path,
    root: &mut TritonRoot,
    changes: &mut Vec<(String, Change)>,
) -> Result<()> {
    let tests_dir = project_dir.join("components/tests");
    if !tests_dir.exists() {
        fs::create_dir_all(tests_dir.join("src"))?;
        fs::create_dir_all(tests_dir.join("include"))?;
    }

    let test_cpp = tests_dir.join("src/test_main.cpp");
    if !test_cpp.exists() {
        let test_code = r#"#include <gtest/gtest.h>

TEST(SampleTest, BasicAssertions) {
    EXPECT_EQ(1 + 1, 2);
}

int main(int argc, char **argv) {
    ::testing::InitGoogleTest(&argc, argv);
    return RUN_ALL_TESTS();
}
"#;
        changes.push((
            test_cpp.display().to_string(),
            write_text_if_changed(&test_cpp, test_code)?,
        ));
    }

    let tests_cmake = tests_dir.join("CMakeLists.txt");
    if !tests_cmake.exists() {
        let tests_cmake_txt = component_cmakelists(true);
        changes.push((
            tests_cmake.display().to_string(),
            write_text_if_changed(&tests_cmake, &tests_cmake_txt)?,
        ));
    }

    root.components
        .entry("tests".into())
        .or_insert_with(|| TritonComponent {
            kind: "exe".into(),
            link: vec![LinkEntry::Name("GTest".into())],
            ..Default::default()
        });

    if !root
        .deps
        .iter()
        .any(|d| matches!(d, DepSpec::Simple(n) if n.eq_ignore_ascii_case("gtest")))
    {
        root.deps.push(DepSpec::Simple("gtest".into()));
    }

    Ok(())
}

/// Ensures `vcpkg.json` exists with a gtest dependency.
fn setup_vcpkg_json(
    project_dir: &Path,
    app_name: &str,
    changes: &mut Vec<(String, Change)>,
) -> Result<()> {
    let vcpkg_path = project_dir.join("vcpkg.json");
    let mut vcpkg_doc = if vcpkg_path.exists() {
        let s = fs::read_to_string(&vcpkg_path)?;
        serde_json::from_str::<serde_json::Value>(&s).unwrap_or_else(|_| serde_json::json!({}))
    } else {
        serde_json::json!({
            "name": app_name.to_lowercase().replace('_', "-"),
            "version": "0.0.0",
            "dependencies": []
        })
    };
    if !vcpkg_doc
        .get("dependencies")
        .map(|v| v.is_array())
        .unwrap_or(false)
    {
        vcpkg_doc["dependencies"] = serde_json::json!([]);
    }
    let deps = vcpkg_doc["dependencies"].as_array_mut().unwrap();
    if !deps
        .iter()
        .any(|d| d.is_string() && d.as_str().unwrap().eq_ignore_ascii_case("gtest"))
    {
        deps.push(serde_json::json!("gtest"));
    }
    changes.push((
        "vcpkg.json".into(),
        write_text_if_changed(&vcpkg_path, &serde_json::to_string_pretty(&vcpkg_doc)?)?,
    ));
    Ok(())
}

/// Clones vcpkg if it is not already present.
fn clone_vcpkg_if_missing(
    project_dir: &Path,
    changes: &mut Vec<(String, Change)>,
) -> Result<()> {
    let vcpkg_dir = project_dir.join("vcpkg");
    if !vcpkg_dir.exists() {
        eprintln!("Cloning vcpkg...");
        run(
            "git",
            &["clone", "https://github.com/microsoft/vcpkg.git", "vcpkg"],
            project_dir,
        )?;
        changes.push(("vcpkg/ (git clone)".into(), Change::Created));
    }
    Ok(())
}
