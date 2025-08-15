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

    // Determine whether this is a "minimal" init (in-place, no new app scaffold)
    let minimal_mode = match name_opt {
        None => true,                  // no name => minimal
        Some(name) if name == "." => true, // explicit '.' => minimal
        _ => false,
    };

    // Where the project root will live
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

    if generator.eq_ignore_ascii_case("ninja") {
        let _ = ensure_ninja_dir(&project_dir);
    }

    // App name always derived from the final folder name
    let app_name: String = project_dir
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("MyProject")
        .to_string();

    let mut changes: Vec<(String, Change)> = Vec::new();

    // Always ensure components dir + core cmake files
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

    // Load or create root metadata
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

    // Keep metadata fields fresh (idempotent)
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

    // App scaffold ONLY if not minimal
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
                changes.push((main_cpp.clone(), write_text_if_changed(&main_cpp, hello)?));
            }
            let comp_cmake = format!("{}/CMakeLists.txt", comp_dir);
            if !Path::new(&comp_cmake).exists() {
                changes.push((
                    comp_cmake.clone(),
                    write_text_if_changed(&comp_cmake, &component_cmakelists())?,
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

    // Always ensure tests component exists
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
        changes.push((
            test_cpp.display().to_string(),
            write_text_if_changed(test_cpp.to_str().unwrap(), test_code)?,
        ));
    }

    let tests_cmake = tests_dir.join("CMakeLists.txt");
    if !tests_cmake.exists() {
        let tests_cmake_txt = r#"add_executable(tests src/test_main.cpp)
find_package(GTest CONFIG REQUIRED)
target_link_libraries(tests PRIVATE GTest::gtest GTest::gtest_main)
enable_testing()
add_test(NAME all_tests COMMAND tests)
"#;
        changes.push((
            tests_cmake.display().to_string(),
            write_text_if_changed(tests_cmake.to_str().unwrap(), tests_cmake_txt)?,
        ));
    }

    // Ensure 'tests' component exists in metadata
    root.components
        .entry("tests".into())
        .or_insert_with(|| TritonComponent {
            kind: "exe".into(),
            link: vec![LinkEntry::Name("GTest".into())],
            defines: vec![],
            exports: vec![],
        });

    // Ensure gtest in root deps once
    let has_gtest_dep = root.deps.iter().any(|d| match d {
        RootDep::Name(n) => n.eq_ignore_ascii_case("gtest"),
        RootDep::Git(_) => false,
    });
    if !has_gtest_dep {
        root.deps.push(RootDep::Name("gtest".into()));
    }

    // Save metadata
    changes.push((
        "triton.json".into(),
        write_json_pretty_changed("triton.json", &root)?,
    ));

    // Update vcpkg.json (ensure "gtest" exactly once)
    let vcpkg_path = Path::new("vcpkg.json");
    let mut vcpkg_doc = if vcpkg_path.exists() {
        let s = fs::read_to_string(vcpkg_path)?;
        serde_json::from_str::<serde_json::Value>(&s)
            .unwrap_or_else(|_| serde_json::json!({}))
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
        .expect("deps is array after normalization");
    if !deps
        .iter()
        .any(|d| d.is_string() && d.as_str().unwrap().eq_ignore_ascii_case("gtest"))
    {
        deps.push(serde_json::json!("gtest"));
    }
    changes.push((
        "vcpkg.json".into(),
        write_text_if_changed("vcpkg.json", &serde_json::to_string_pretty(&vcpkg_doc)?)?,
    ));

    // ---- vcpkg clone policy ----
    // As requested: do NOT clone if a 'vcpkg/' directory already exists (no extra checks).
    if Path::new("vcpkg").exists() {
        changes.push(("vcpkg/".into(), Change::Unchanged));
    } else {
        eprintln!("Cloning vcpkg...");
        run(
            "git",
            &["clone", "https://github.com/microsoft/vcpkg.git", "vcpkg"],
            ".",
        )?;
        changes.push(("vcpkg/ (git clone)".into(), Change::Created));
    }

    if minimal_mode {
        eprintln!(
            "\nInitialized Triton in existing project '{}'. (tests ensured)",
            app_name
        );
    } else {
        eprintln!("\nInitialized project '{}' (app + tests ensured).", app_name);
    }
    Ok(())
}
