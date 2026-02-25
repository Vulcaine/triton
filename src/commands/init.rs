use anyhow::{Context, Result};
use std::env;
use std::fs;
use std::path::Path;

use crate::cmake::effective_cmake_version;
use crate::models::{DepSpec, LinkEntry, TritonComponent, TritonRoot};
use crate::templates::{cmake_presets, component_cmakelists, components_dir_cmakelists};
use crate::tools::ensure_ninja_dir;
use crate::util::{run, write_json_pretty_changed, write_text_if_changed, Change};
use crate::util; // for read_json

pub fn handle_init(
    name_opt: Option<&str>,
    triplet: &str,
    generator: &str,
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
    env::set_current_dir(&project_dir)
        .with_context(|| format!("cd into {}", project_dir.display()))?;

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

    // components/ base
    fs::create_dir_all("components")?;
    _changes.push((
        "components/CMakeLists.txt".into(),
        write_text_if_changed("components/CMakeLists.txt", &components_dir_cmakelists())?,
    ));

    // Parse our minimum CMake version once and pass to templates
    let cmake_ver = effective_cmake_version();

    // Preserve user-passed triplet and generator
    _changes.push((
        "components/CMakePresets.json".into(),
        write_text_if_changed(
            "components/CMakePresets.json",
            &cmake_presets(&app_name, generator, triplet, cmake_ver),
        )?,
    ));

    // Load or seed triton.json
    let mut root: TritonRoot = if Path::new("triton.json").exists() {
        util::read_json("triton.json")?
    } else {
        let mut r = TritonRoot::default();
        r.app_name = app_name.clone();
        r.generator = generator.to_string();
        r
    };

    if root.app_name.is_empty() {
        root.app_name = app_name.clone();
    }

    if root.generator.is_empty() {
        root.generator = generator.to_string();
    }

    // app scaffold
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
                    resources: vec![],
                    link_options: Default::default(),
                    vendor_libs: vec![],
                });
        }
    }

    // tests component
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

    root.components
        .entry("tests".into())
        .or_insert_with(|| TritonComponent {
            kind: "exe".into(),
            link: vec![LinkEntry::Name("GTest".into())],
            defines: vec![],
            exports: vec![],
            resources: vec![],
            link_options: Default::default(),
            vendor_libs: vec![],
        });

    if !root
        .deps
        .iter()
        .any(|d| matches!(d, DepSpec::Simple(n) if n.eq_ignore_ascii_case("gtest")))
    {
        root.deps.push(DepSpec::Simple("gtest".into()));
    }

    _changes.push((
        "triton.json".into(),
        write_json_pretty_changed("triton.json", &root)?,
    ));

    // Ensure vcpkg.json
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
    let deps = vcpkg_doc["dependencies"].as_array_mut().unwrap();
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

    // clone vcpkg if not present
    if !Path::new("vcpkg").exists() {
        eprintln!("Cloning vcpkg...");
        run("git", &["clone", "https://github.com/microsoft/vcpkg.git", "vcpkg"], ".")?;
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
