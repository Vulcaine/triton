use anyhow::{Context, Result};
use std::env;
use std::fs;
use std::path::Path;

use crate::models::{LinkEntry, RootDep, TritonComponent, TritonRoot};
use crate::templates::{cmake_presets, component_cmakelists, components_dir_cmakelists};
use crate::tools::ensure_ninja_dir;
use crate::util::{run, write_json_pretty_changed, write_text_if_changed, Change};
use crate::util; // for read_json

pub fn handle_init(
    name_opt: Option<&str>,
    triplet: &str,
    generator: &str,
    cxx_std: &str,
) -> Result<()> {
    let cwd = env::current_dir().context("cannot get current directory")?;

    // Minimal mode means: do not scaffold an app component. Only ensure/repair core files.
    // This is active when:
    //   - name is None  ( `triton init` )
    //   - name is "."   ( `triton init .` )
    // In contrast, when a non-dot name is given, we create a new folder and scaffold the app.
    let minimal_mode = matches!(name_opt, None | Some("."));

    // Decide the target project directory
    let project_dir = if minimal_mode {
        cwd.clone()
    } else {
        cwd.join(name_opt.unwrap())
    };

    if !project_dir.exists() {
        fs::create_dir_all(&project_dir)?;
    }
    env::set_current_dir(&project_dir)
        .with_context(|| format!("cd into {}", project_dir.display()))?;

    // If using Ninja, ensure portable ninja is present (no-op if already there)
    if generator.eq_ignore_ascii_case("ninja") {
        let _ = ensure_ninja_dir(&project_dir);
    }

    // App name is derived from the project directory name (used for metadata and presets)
    let app_name: String = project_dir
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("MyProject")
        .to_string();

    let mut _changes: Vec<(String, Change)> = Vec::new();

    // Always ensure components/ exists + the root components CMake files are present
    fs::create_dir_all("components")?;
    _changes.push((
        "components/CMakeLists.txt".into(),
        write_text_if_changed("components/CMakeLists.txt", &components_dir_cmakelists())?,
    ));
    _changes.push((
        "components/CMakePresets.json".into(),
        write_text_if_changed(
            "components/CMakePresets.json",
            &cmake_presets(&app_name, generator, triplet),
        )?,
    ));

    // Load existing triton.json if present; otherwise seed defaults.
    let mut root: TritonRoot = if Path::new("triton.json").exists() {
        util::read_json("triton.json")?
    } else {
        let mut r = TritonRoot::default();
        r.app_name = app_name.clone();
        r.triplet = triplet.to_string();
        r.generator = generator.to_string();
        r.cxx_std = cxx_std.to_string();
        r
    };

    // Keep top-level fields fresh (don’t overwrite real values if already set).
    if root.app_name.is_empty() {
        root.app_name = app_name.clone();
    }
    if root.triplet.is_empty() {
        root.triplet = triplet.to_string();
    }
    if root.generator.is_empty() {
        root.generator = generator.to_string();
    }
    if root.cxx_std.is_empty() {
        root.cxx_std = cxx_std.to_string();
    }

    // ── App scaffold ONLY in non-minimal mode ─────────────────────────────────────
    // Minimal mode: triton init  OR  triton init .  -> do NOT create components/<folder-name>
    if !minimal_mode {
        let comp_dir = format!("components/{}", app_name);
        if !Path::new(&comp_dir).exists() {
            fs::create_dir_all(format!("{}/src", comp_dir))?;
            fs::create_dir_all(format!("{}/include", comp_dir))?;
            let hello = r#"#include <iostream>
int main() { std::cout << "Hello from triton app!\n"; return 0; }
"#;
            let main_cpp = format!("{}/src/main.cpp", comp_dir);
            if !Path::new(&main_cpp).exists() {
                _changes.push((main_cpp.clone(), write_text_if_changed(&main_cpp, hello)?));
            }
            let comp_cmake = format!("{}/CMakeLists.txt", comp_dir);
            if !Path::new(&comp_cmake).exists() {
                _changes.push((
                    comp_cmake.clone(),
                    write_text_if_changed(&comp_cmake, &component_cmakelists(false))?,
                ));
            }
            root.components
                .entry(app_name.clone())
                .or_insert_with(|| TritonComponent {
                    kind: "exe".into(),
                    link: vec![],
                    defines: vec![],
                    exports: vec![],
                });
        }
    }

    // ── Always ensure tests component exists (idempotent) ─────────────────────────
    let tests_dir = Path::new("components/tests");
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
        _changes.push((
            test_cpp.display().to_string(),
            write_text_if_changed(test_cpp.to_str().unwrap(), test_code)?,
        ));
    }

    let tests_cmake = tests_dir.join("CMakeLists.txt");
    if !tests_cmake.exists() {
        let tests_cmake_txt = component_cmakelists(true);
        _changes.push((
            tests_cmake.display().to_string(),
            write_text_if_changed(tests_cmake.to_str().unwrap(), &tests_cmake_txt)?,
        ));
    }

    // Ensure tests component is recorded in metadata
    root.components
        .entry("tests".into())
        .or_insert_with(|| TritonComponent {
            kind: "exe".into(),
            link: vec![LinkEntry::Name("GTest".into())],
            defines: vec![],
            exports: vec![],
        });

    // Ensure "gtest" is present in root deps exactly once
    let has_gtest_dep = root.deps.iter().any(|d| matches!(d, RootDep::Name(n) if n.eq_ignore_ascii_case("gtest")));
    if !has_gtest_dep {
        root.deps.push(RootDep::Name("gtest".into()));
    }

    // Persist triton.json (pretty, only if changed)
    _changes.push((
        "triton.json".into(),
        write_json_pretty_changed("triton.json", &root)?,
    ));

    // Ensure vcpkg.json exists and contains "gtest" exactly once
    let vcpkg_path = Path::new("vcpkg.json");
    let mut vcpkg_doc = if vcpkg_path.exists() {
        let s = fs::read_to_string(vcpkg_path)?;
        serde_json::from_str::<serde_json::Value>(&s).unwrap_or_else(|_| serde_json::json!({}))
    } else {
        serde_json::json!({
            "name": app_name,
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
    let deps = vcpkg_doc["dependencies"]
        .as_array_mut()
        .expect("deps normalized to array");
    if !deps
        .iter()
        .any(|d| d.is_string() && d.as_str().unwrap().eq_ignore_ascii_case("gtest"))
    {
        deps.push(serde_json::json!("gtest"));
    }
    _changes.push((
        "vcpkg.json".into(),
        write_text_if_changed("vcpkg.json", &serde_json::to_string_pretty(&vcpkg_doc)?)?,
    ));

    // vcpkg clone: only if the directory doesn't already exist
    if Path::new("vcpkg").exists() {
        _changes.push(("vcpkg/".into(), Change::Unchanged));
    } else {
        eprintln!("Cloning vcpkg...");
        run(
            "git",
            &["clone", "https://github.com/microsoft/vcpkg.git", "vcpkg"],
            ".",
        )?;
        _changes.push(("vcpkg/ (git clone)".into(), Change::Created));
    }

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
