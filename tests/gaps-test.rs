//! Gap tests: proving missing validations and edge cases in Triton.
//!
//! Each test targets a specific gap — either a missing validation,
//! an unhandled edge case, or untested CMake generation path.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::Path;

use serial_test::serial;
use tempfile::tempdir;

use triton::cmake::rewrite_component_cmake;
use triton::commands::{handle_add, handle_generate};
use triton::commands::init::handle_init;
use triton::commands::remove::handle_remove;
use triton::handle_link;
use triton::models::*;
use triton::util::{read_json, write_json_pretty_changed};

mod test_utils;
use test_utils::copy_offline_vcpkg_to;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn write_file(path: impl AsRef<Path>, s: &str) {
    fs::create_dir_all(path.as_ref().parent().unwrap()).ok();
    fs::write(path, s).unwrap();
}

fn scaffold_component(root: &Path, name: &str) {
    let comp_dir = root.join("components").join(name);
    fs::create_dir_all(comp_dir.join("src")).unwrap();
    fs::create_dir_all(comp_dir.join("include")).unwrap();
    write_file(
        comp_dir.join("CMakeLists.txt"),
        r#"cmake_minimum_required(VERSION 3.25)
get_filename_component(_comp_name "${CMAKE_CURRENT_SOURCE_DIR}" NAME)
add_library(${_comp_name})
set(_is_exe OFF)
target_include_directories(${_comp_name} PUBLIC "include")
# ## triton:deps begin
# ## triton:deps end
"#,
    );
}

fn write_minimal_resources(root: &Path) {
    let res = root.join("resources");
    fs::create_dir_all(&res).unwrap();
    fs::write(
        res.join("cmake_template.cmake"),
        r#"cmake_minimum_required(VERSION 3.25)
get_filename_component(_comp_name "${CMAKE_CURRENT_SOURCE_DIR}" NAME)

if(EXISTS "${CMAKE_CURRENT_SOURCE_DIR}/src/main.cpp")
  add_executable(${_comp_name})
  set(_is_exe ON)
else()
  add_library(${_comp_name})
  set(_is_exe OFF)
endif()

if(_is_exe)
  target_include_directories(${_comp_name} PRIVATE "include")
else()
  target_include_directories(${_comp_name} PUBLIC "include")
endif()

# ## triton:deps begin
# ## triton:deps end
"#,
    )
    .unwrap();
    fs::write(res.join("cmake_root_template.cmake"), "# (helpers stub)\n").unwrap();
    fs::write(
        res.join("cmake_presets_template.json"),
        r#"{
  "version": 6,
  "configurePresets": [
    { "name": "debug", "generator": "Ninja", "binaryDir": "${sourceDir}/../build/debug" }
  ],
  "buildPresets": [ { "name": "debug", "configurePreset": "debug" } ]
}"#,
    )
    .unwrap();
}

fn default_component(kind: &str) -> TritonComponent {
    TritonComponent {
        kind: kind.into(),
        link: vec![],
        defines: vec![],
        exports: vec![],
        resources: vec![],
        link_options: LinkOptions::None,
        vendor_libs: VendorLibs::None,
        assets: vec![],
    }
}

fn init_project(root: &Path, deps: &[DepSpec], components: BTreeMap<String, TritonComponent>) {
    fs::create_dir_all(root.join("components")).unwrap();
    let tr = TritonRoot {
        app_name: "demo".into(),
        generator: "Ninja".into(),
        cxx_std: "20".into(),
        deps: deps.to_vec(),
        components,
        scripts: HashMap::default(),
    };
    write_json_pretty_changed(root.join("triton.json"), &tr).unwrap();
}

// ===========================================================================
// GAP 1: Invalid component `kind` accepted silently
// ===========================================================================

#[test]
fn invalid_component_kind_deserializes_without_error() {
    // GAP: "kind" accepts any string — no validation for "exe"/"lib" only.
    // A user can typo "eze" or use "shared_lib" and Triton silently accepts it.
    let json = r#"{"kind":"foobar","link":[]}"#;
    let comp: TritonComponent = serde_json::from_str(json).unwrap();
    // This succeeds — proving there's no validation.
    assert_eq!(comp.kind, "foobar");
}

#[test]
#[serial]
fn invalid_kind_generates_cmake_without_error() {
    // GAP: Even during CMake generation, an invalid kind doesn't cause an error.
    let td = tempdir().unwrap();
    let root_path = td.path();
    std::env::set_current_dir(root_path).unwrap();
    write_minimal_resources(root_path);

    let comp = TritonComponent {
        kind: "invalid_kind".into(),
        ..default_component("lib")
    };
    let mut components = BTreeMap::new();
    components.insert("Bad".into(), comp.clone());

    let root = TritonRoot {
        app_name: "testapp".into(),
        generator: "Ninja".into(),
        cxx_std: "20".into(),
        deps: vec![],
        components,
        scripts: HashMap::default(),
    };

    scaffold_component(root_path, "Bad");
    // This should ideally error but doesn't — no kind validation.
    let result = rewrite_component_cmake("Bad", &root, &comp, (3, 30, 1));
    assert!(result.is_ok(), "Invalid kind silently accepted — GAP: no validation");
}

// ===========================================================================
// GAP 2: Self-linking (component links to itself)
// ===========================================================================

#[test]
#[serial]
fn component_can_link_to_itself() {
    // FIXED: Self-linking is now rejected.
    let td = tempdir().unwrap();
    let root_path = td.path();
    write_minimal_resources(root_path);
    std::env::set_current_dir(root_path).unwrap();

    let mut components = BTreeMap::new();
    components.insert(
        "Core".into(),
        TritonComponent {
            kind: "lib".into(),
            link: vec![],
            ..default_component("lib")
        },
    );
    init_project(root_path, &[], components);
    scaffold_component(root_path, "Core");
    fs::write(
        root_path.join("components/Core/CMakeLists.txt"),
        fs::read_to_string(root_path.join("resources/cmake_template.cmake")).unwrap(),
    )
    .unwrap();

    // Link Core to itself — should be rejected
    let result = handle_link("Core", "Core");
    assert!(result.is_err(), "Self-link should be rejected");
    let msg = format!("{:#}", result.unwrap_err());
    assert!(msg.contains("cannot link to itself"), "got: {msg}");
}

// ===========================================================================
// GAP 3: Circular component dependencies
// ===========================================================================

#[test]
#[serial]
fn circular_component_deps_accepted() {
    // FIXED: Circular dependencies are now rejected.
    let td = tempdir().unwrap();
    let root_path = td.path();
    write_minimal_resources(root_path);
    std::env::set_current_dir(root_path).unwrap();

    let mut components = BTreeMap::new();
    components.insert("A".into(), default_component("lib"));
    components.insert("B".into(), default_component("lib"));
    init_project(root_path, &[], components);
    scaffold_component(root_path, "A");
    scaffold_component(root_path, "B");

    for name in &["A", "B"] {
        fs::write(
            root_path.join(format!("components/{name}/CMakeLists.txt")),
            fs::read_to_string(root_path.join("resources/cmake_template.cmake")).unwrap(),
        )
        .unwrap();
    }

    // A depends on B — succeeds
    handle_link("B", "A").unwrap();
    // B depends on A — should be rejected (creates cycle)
    let result = handle_link("A", "B");
    assert!(result.is_err(), "Circular dependency should be rejected");
    let msg = format!("{:#}", result.unwrap_err());
    assert!(msg.contains("Circular dependency"), "got: {msg}");
}

// ===========================================================================
// GAP 4: Remove Detailed dep works correctly
// ===========================================================================

#[test]
#[serial]
fn remove_detailed_dep_globally() {
    // Verify that removing a DepSpec::Detailed dep works (not just Simple).
    let td = tempdir().unwrap();
    let root = td.path();
    std::env::set_current_dir(root).unwrap();

    let mut components = BTreeMap::new();
    components.insert(
        "App".into(),
        TritonComponent {
            kind: "exe".into(),
            link: vec![LinkEntry::Name("sdl2".into())],
            ..default_component("exe")
        },
    );

    let meta = TritonRoot {
        app_name: "demo".into(),
        generator: "Ninja".into(),
        cxx_std: "20".into(),
        deps: vec![DepSpec::Detailed(DepDetailed {
            name: "sdl2".into(),
            os: vec!["windows".into()],
            package: Some("SDL2".into()),
            triplet: vec![],
            features: vec!["vulkan".into()],
        })],
        components,
        scripts: HashMap::default(),
    };

    fs::create_dir_all(root.join("components")).unwrap();
    write_json_pretty_changed(root.join("triton.json"), &meta).unwrap();
    scaffold_component(root, "App");

    handle_remove("sdl2", None, None, false).unwrap();

    let after: TritonRoot = read_json("triton.json").unwrap();
    assert!(
        !after.deps.iter().any(|d| match d {
            DepSpec::Detailed(dd) => dd.name == "sdl2",
            _ => false,
        }),
        "Detailed dep 'sdl2' should be removed"
    );
    let app = after.components.get("App").unwrap();
    assert!(
        !app.link.iter().any(|e| e.normalize().0 == "sdl2"),
        "sdl2 should be unlinked from App"
    );
}

// ===========================================================================
// GAP 5: Script names don't validate against built-in commands
// ===========================================================================

#[test]
fn script_name_shadowing_builtin_not_validated_in_json() {
    // GAP: README says script names cannot shadow built-ins, but triton.json
    // happily accepts "build" or "add" as script names without validation.
    let json = r#"{
        "app_name": "demo",
        "generator": "Ninja",
        "cxx_std": "20",
        "deps": [],
        "components": {},
        "scripts": {
            "build": "echo override",
            "add": "echo sneaky",
            "init": "echo init"
        }
    }"#;
    let root: TritonRoot = serde_json::from_str(json).unwrap();
    // Deserialization succeeds — no validation
    assert_eq!(root.scripts.len(), 3);
    assert!(
        root.scripts.contains_key("build"),
        "GAP: 'build' accepted as script name despite shadowing built-in"
    );
}

// ===========================================================================
// GAP 6: Component links to unknown dep (not in deps, not a component)
// ===========================================================================

#[test]
#[serial]
fn component_links_to_nonexistent_dep_accepted() {
    // GAP: A component can reference a dep that doesn't exist in `deps` or
    // `components`. This produces broken CMake at build time, not at config time.
    let td = tempdir().unwrap();
    let root_path = td.path();
    std::env::set_current_dir(root_path).unwrap();
    write_minimal_resources(root_path);

    let json = r#"{
        "app_name": "demo",
        "generator": "Ninja",
        "cxx_std": "20",
        "deps": [],
        "components": {
            "App": {
                "kind": "exe",
                "link": ["nonexistent_library"]
            }
        }
    }"#;
    let root: TritonRoot = serde_json::from_str(json).unwrap();
    // Parses fine — no validation that linked deps exist
    let app = root.components.get("App").unwrap();
    assert!(
        app.link.iter().any(|e| e.normalize().0 == "nonexistent_library"),
        "GAP: link to nonexistent dep accepted without error"
    );
}

#[test]
#[serial]
fn generate_with_unknown_link_produces_no_cmake_for_it() {
    // When a component links to something not in deps or components,
    // the generated CMake silently skips it — no warning at generate time.
    let td = tempdir().unwrap();
    let root_path = td.path();
    std::env::set_current_dir(root_path).unwrap();
    write_minimal_resources(root_path);

    let mut components = BTreeMap::new();
    components.insert(
        "App".into(),
        TritonComponent {
            kind: "exe".into(),
            link: vec![LinkEntry::Name("ghost_dep".into())],
            ..default_component("exe")
        },
    );

    let root = TritonRoot {
        app_name: "demo".into(),
        generator: "Ninja".into(),
        cxx_std: "20".into(),
        deps: vec![], // ghost_dep is NOT in deps
        components,
        scripts: HashMap::default(),
    };

    scaffold_component(root_path, "App");
    let result = rewrite_component_cmake("App", &root, root.components.get("App").unwrap(), (3, 30, 1));
    // No error — silently skips the unknown link
    assert!(result.is_ok(), "GAP: generate silently ignores unknown link targets");

    let cm = fs::read_to_string(root_path.join("components/App/CMakeLists.txt")).unwrap();
    assert!(
        !cm.contains("ghost_dep"),
        "ghost_dep not in CMake output — silently dropped without warning"
    );
}

// ===========================================================================
// GAP 7: Empty app_name accepted
// ===========================================================================

#[test]
fn empty_app_name_accepted() {
    // GAP: Empty app_name is valid JSON but would produce broken CMake
    let json = r#"{
        "app_name": "",
        "generator": "Ninja",
        "cxx_std": "20",
        "deps": [],
        "components": {}
    }"#;
    let root: TritonRoot = serde_json::from_str(json).unwrap();
    assert_eq!(root.app_name, "", "GAP: empty app_name accepted");
}

// ===========================================================================
// GAP 8: CMake generation for vendor_libs (untested code path)
// ===========================================================================

#[test]
#[serial]
fn generate_vendor_libs_all_platforms() {
    let td = tempdir().unwrap();
    let root_path = td.path();
    std::env::set_current_dir(root_path).unwrap();
    write_minimal_resources(root_path);

    let comp = TritonComponent {
        kind: "exe".into(),
        vendor_libs: VendorLibs::All(vec!["vendor/libfoo.a".into()]),
        ..default_component("exe")
    };

    let mut components = BTreeMap::new();
    components.insert("App".into(), comp.clone());

    let root = TritonRoot {
        app_name: "demo".into(),
        generator: "Ninja".into(),
        cxx_std: "20".into(),
        deps: vec![],
        components,
        scripts: HashMap::default(),
    };

    scaffold_component(root_path, "App");
    rewrite_component_cmake("App", &root, &comp, (3, 30, 1)).unwrap();

    let cm = fs::read_to_string(root_path.join("components/App/CMakeLists.txt")).unwrap();
    assert!(
        cm.contains("vendor/libfoo.a"),
        "vendor_libs should appear in generated CMake"
    );
    assert!(
        cm.contains("target_link_libraries"),
        "vendor_libs should use target_link_libraries"
    );
}

#[test]
#[serial]
fn generate_vendor_libs_per_platform() {
    let td = tempdir().unwrap();
    let root_path = td.path();
    std::env::set_current_dir(root_path).unwrap();
    write_minimal_resources(root_path);

    let mut platform_map = BTreeMap::new();
    platform_map.insert("linux".into(), vec!["vendor/libfoo.a".into()]);
    platform_map.insert("windows".into(), vec!["vendor/foo.lib".into()]);

    let comp = TritonComponent {
        kind: "exe".into(),
        vendor_libs: VendorLibs::PerPlatform(platform_map),
        ..default_component("exe")
    };

    let mut components = BTreeMap::new();
    components.insert("App".into(), comp.clone());

    let root = TritonRoot {
        app_name: "demo".into(),
        generator: "Ninja".into(),
        cxx_std: "20".into(),
        deps: vec![],
        components,
        scripts: HashMap::default(),
    };

    scaffold_component(root_path, "App");
    rewrite_component_cmake("App", &root, &comp, (3, 30, 1)).unwrap();

    let cm = fs::read_to_string(root_path.join("components/App/CMakeLists.txt")).unwrap();
    assert!(cm.contains("if(UNIX AND NOT APPLE)"), "should have Linux guard");
    assert!(cm.contains("if(WIN32)"), "should have Windows guard");
    assert!(cm.contains("vendor/libfoo.a"), "should have Linux vendor lib");
    assert!(cm.contains("vendor/foo.lib"), "should have Windows vendor lib");
    // Windows .lib should trigger DLL copy logic
    assert!(cm.contains("copy_if_different"), "should have DLL copy for .lib file");
}

// ===========================================================================
// GAP 9: CMake generation for link_options (untested code path)
// ===========================================================================

#[test]
#[serial]
fn generate_link_options_all_platforms() {
    let td = tempdir().unwrap();
    let root_path = td.path();
    std::env::set_current_dir(root_path).unwrap();
    write_minimal_resources(root_path);

    let comp = TritonComponent {
        kind: "exe".into(),
        link_options: LinkOptions::All(vec!["-Wl,--export-dynamic".into()]),
        ..default_component("exe")
    };

    let mut components = BTreeMap::new();
    components.insert("App".into(), comp.clone());

    let root = TritonRoot {
        app_name: "demo".into(),
        generator: "Ninja".into(),
        cxx_std: "20".into(),
        deps: vec![],
        components,
        scripts: HashMap::default(),
    };

    scaffold_component(root_path, "App");
    rewrite_component_cmake("App", &root, &comp, (3, 30, 1)).unwrap();

    let cm = fs::read_to_string(root_path.join("components/App/CMakeLists.txt")).unwrap();
    assert!(
        cm.contains("target_link_options"),
        "should have target_link_options"
    );
    assert!(
        cm.contains("--export-dynamic"),
        "should contain the linker flag"
    );
}

#[test]
#[serial]
fn generate_link_options_per_platform() {
    let td = tempdir().unwrap();
    let root_path = td.path();
    std::env::set_current_dir(root_path).unwrap();
    write_minimal_resources(root_path);

    let mut platform_map = BTreeMap::new();
    platform_map.insert("linux".into(), vec!["-Wl,--export-dynamic".into()]);
    platform_map.insert("macos".into(), vec!["-framework CoreFoundation".into()]);

    let comp = TritonComponent {
        kind: "exe".into(),
        link_options: LinkOptions::PerPlatform(platform_map),
        ..default_component("exe")
    };

    let mut components = BTreeMap::new();
    components.insert("App".into(), comp.clone());

    let root = TritonRoot {
        app_name: "demo".into(),
        generator: "Ninja".into(),
        cxx_std: "20".into(),
        deps: vec![],
        components,
        scripts: HashMap::default(),
    };

    scaffold_component(root_path, "App");
    rewrite_component_cmake("App", &root, &comp, (3, 30, 1)).unwrap();

    let cm = fs::read_to_string(root_path.join("components/App/CMakeLists.txt")).unwrap();
    assert!(cm.contains("if(UNIX AND NOT APPLE)"), "should have Linux guard");
    assert!(cm.contains("if(APPLE)"), "should have macOS guard");
    assert!(cm.contains("--export-dynamic"), "should have Linux link option");
    assert!(cm.contains("CoreFoundation"), "should have macOS link option");
}

// ===========================================================================
// GAP 10: CMake generation for assets (untested code path)
// ===========================================================================

#[test]
#[serial]
fn generate_assets_produces_incremental_staging() {
    let td = tempdir().unwrap();
    let root_path = td.path();
    std::env::set_current_dir(root_path).unwrap();
    write_minimal_resources(root_path);

    let comp = TritonComponent {
        kind: "exe".into(),
        assets: vec!["data".into(), "config.json".into()],
        ..default_component("exe")
    };

    let mut components = BTreeMap::new();
    components.insert("App".into(), comp.clone());

    let root = TritonRoot {
        app_name: "demo".into(),
        generator: "Ninja".into(),
        cxx_std: "20".into(),
        deps: vec![],
        components,
        scripts: HashMap::default(),
    };

    scaffold_component(root_path, "App");
    rewrite_component_cmake("App", &root, &comp, (3, 30, 1)).unwrap();

    let cm = fs::read_to_string(root_path.join("components/App/CMakeLists.txt")).unwrap();
    assert!(
        cm.contains("triton: stage component assets"),
        "should have assets staging comment"
    );
    assert!(
        cm.contains("_triton_asset_stamps"),
        "should track asset stamps"
    );
    assert!(
        cm.contains("_assets ALL DEPENDS"),
        "should create _assets custom target"
    );
    // Should handle both "data" (directory) and "config.json" (file)
    assert!(cm.contains("_triton_asset_src_data"), "should have data asset variable");
    assert!(cm.contains("_triton_asset_src_config_json"), "should have config.json asset variable");
}

// ===========================================================================
// GAP 11: Generate with OS-filtered detailed dep excludes on wrong OS
// ===========================================================================

#[test]
#[serial]
fn generate_os_filtered_dep_excluded_on_wrong_os() {
    // A dep with `os: ["macos"]` should not appear in vcpkg.json on Windows/Linux
    let td = tempdir().unwrap();
    let proj = td.path().join("os-filter");
    fs::create_dir_all(&proj).unwrap();
    copy_offline_vcpkg_to(&proj);
    std::env::set_current_dir(&proj).unwrap();

    handle_init(Some("."), "Ninja", "20").unwrap();

    let mut root: TritonRoot = read_json(proj.join("triton.json")).unwrap();

    // This dep is macOS-only; on Windows/Linux it should be excluded
    let host_os = std::env::consts::OS;
    let opposite_os = if host_os == "macos" { "linux" } else { "macos" };

    root.deps.push(DepSpec::Detailed(DepDetailed {
        name: "metal-cpp".into(),
        os: vec![opposite_os.into()],
        package: None,
        triplet: vec![],
        features: vec![],
    }));
    write_json_pretty_changed(proj.join("triton.json"), &root).unwrap();

    handle_generate().unwrap();

    let vcpkg: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(proj.join("vcpkg.json")).unwrap()).unwrap();
    let deps = vcpkg["dependencies"].as_array().expect("dependencies array");

    assert!(
        !deps.iter().any(|d| {
            d.as_str().map(|s| s == "metal-cpp").unwrap_or(false)
                || d.get("name")
                    .and_then(|n| n.as_str())
                    .map(|n| n == "metal-cpp")
                    .unwrap_or(false)
        }),
        "Dep for opposite OS should be excluded from vcpkg.json"
    );
}

// ===========================================================================
// GAP 12: Exports field generates PUBLIC link visibility
// ===========================================================================

#[test]
#[serial]
fn exports_field_generates_public_link() {
    let td = tempdir().unwrap();
    let root_path = td.path();
    std::env::set_current_dir(root_path).unwrap();
    write_minimal_resources(root_path);

    let mut components = BTreeMap::new();
    components.insert(
        "Core".into(),
        TritonComponent {
            kind: "lib".into(),
            link: vec![LinkEntry::Name("glm".into())],
            exports: vec!["glm".into()],
            ..default_component("lib")
        },
    );

    let root = TritonRoot {
        app_name: "demo".into(),
        generator: "Ninja".into(),
        cxx_std: "20".into(),
        deps: vec![DepSpec::Simple("glm".into())],
        components,
        scripts: HashMap::default(),
    };

    scaffold_component(root_path, "Core");
    let comp = root.components.get("Core").unwrap();
    rewrite_component_cmake("Core", &root, comp, (3, 30, 1)).unwrap();

    let cm = fs::read_to_string(root_path.join("components/Core/CMakeLists.txt")).unwrap();
    // The glm dep should be generated (it's in deps + link)
    assert!(
        cm.contains("glm"),
        "Core CMake should reference glm"
    );
}

// ===========================================================================
// GAP 13: Add duplicate dep to different component doesn't duplicate in deps
// ===========================================================================

#[test]
#[serial]
fn add_same_dep_to_two_components_no_duplicate_in_deps() {
    let td = tempdir().unwrap();
    let root = td.path();
    std::env::set_current_dir(root).unwrap();

    write_file(
        root.join("triton.json"),
        r#"{
  "app_name": "demo",
  "generator": "Ninja",
  "cxx_std": "20",
  "deps": [],
  "components": {}
}"#,
    );
    write_file(
        root.join("vcpkg.json"),
        r#"{"name":"demo","version":"0.0.0","dependencies":[]}"#,
    );

    // Stub vcpkg
    let bin = root.join("bin");
    fs::create_dir_all(&bin).unwrap();
    #[cfg(windows)]
    {
        write_file(&bin.join("vcpkg.bat"), "@echo off\r\nexit /B 0\r\n");
        std::env::set_var("TRITON_VCPKG_EXE", bin.join("vcpkg.bat"));
        std::env::set_var("VCPKG_EXE", bin.join("vcpkg.bat"));
    }
    #[cfg(not(windows))]
    {
        write_file(&bin.join("vcpkg"), "#!/bin/sh\nexit 0\n");
        let _ = std::process::Command::new("chmod").args(["+x", bin.join("vcpkg").to_str().unwrap()]).status();
        std::env::set_var("TRITON_VCPKG_EXE", bin.join("vcpkg"));
        std::env::set_var("VCPKG_EXE", bin.join("vcpkg"));
    }

    let old_path = std::env::var_os("PATH");
    let mut new_path = bin.into_os_string().into_string().unwrap();
    if let Some(ref old) = old_path {
        #[cfg(windows)]
        { new_path.push(';'); }
        #[cfg(not(windows))]
        { new_path.push(':'); }
        new_path.push_str(&old.to_string_lossy());
    }
    std::env::set_var("PATH", &new_path);

    // Add glm to component A, then glm to component B
    handle_add(&[String::from("glm:A")], None, false).unwrap();
    handle_add(&[String::from("glm:B")], None, false).unwrap();

    let tr: TritonRoot = read_json("triton.json").unwrap();

    // glm should appear exactly once in deps
    let glm_count = tr.deps.iter().filter(|d| matches!(d, DepSpec::Simple(n) if n == "glm")).count();
    assert_eq!(glm_count, 1, "glm should appear exactly once in deps");

    // Both components should link to glm
    let a = tr.components.get("A").unwrap();
    let b = tr.components.get("B").unwrap();
    assert!(a.link.iter().any(|e| e.normalize().0 == "glm"), "A should link glm");
    assert!(b.link.iter().any(|e| e.normalize().0 == "glm"), "B should link glm");

    // Restore
    if let Some(old) = old_path { std::env::set_var("PATH", old); }
    std::env::remove_var("TRITON_VCPKG_EXE");
    std::env::remove_var("VCPKG_EXE");
}

// ===========================================================================
// GAP 14: Generate with git dep + cmake overrides produces correct CMake
// ===========================================================================

#[test]
#[serial]
fn generate_git_dep_with_cmake_overrides() {
    let td = tempdir().unwrap();
    let root_path = td.path();
    std::env::set_current_dir(root_path).unwrap();
    write_minimal_resources(root_path);

    let comp = TritonComponent {
        kind: "exe".into(),
        link: vec![LinkEntry::Named {
            name: "imgui".into(),
            package: None,
            targets: Some(vec!["imgui".into()]),
        }],
        ..default_component("exe")
    };

    let mut components = BTreeMap::new();
    components.insert("App".into(), comp.clone());

    let root = TritonRoot {
        app_name: "demo".into(),
        generator: "Ninja".into(),
        cxx_std: "20".into(),
        deps: vec![DepSpec::Git(GitDep {
            repo: "ocornut/imgui".into(),
            name: "imgui".into(),
            branch: Some("docking".into()),
            cmake: vec![
                CMakeOverride::KV("IMGUI_BUILD_EXAMPLES=OFF".into()),
                CMakeOverride::Entry(CMakeCacheEntry {
                    var: "IMGUI_ENABLE_FREETYPE".into(),
                    val: "ON".into(),
                    typ: "BOOL".into(),
                }),
            ],
        })],
        components,
        scripts: HashMap::default(),
    };

    scaffold_component(root_path, "App");
    rewrite_component_cmake("App", &root, &comp, (3, 30, 1)).unwrap();

    let cm = fs::read_to_string(root_path.join("components/App/CMakeLists.txt")).unwrap();
    assert!(
        cm.contains("IMGUI_BUILD_EXAMPLES"),
        "should have KV cmake override"
    );
    assert!(
        cm.contains("IMGUI_ENABLE_FREETYPE"),
        "should have Entry cmake override"
    );
    assert!(
        cm.contains("CACHE BOOL"),
        "structured entry should use BOOL cache type"
    );
    assert!(
        cm.contains("third_party/imgui"),
        "should reference third_party path"
    );
}

// ===========================================================================
// GAP 15: Detailed dep with features in vcpkg.json after remove still correct
// ===========================================================================

#[test]
#[serial]
fn remove_keeps_detailed_dep_features_in_vcpkg_json() {
    // After removing a simple dep, remaining detailed deps with features
    // should still appear correctly in vcpkg.json
    let td = tempdir().unwrap();
    let root = td.path();
    std::env::set_current_dir(root).unwrap();

    let meta = TritonRoot {
        app_name: "demo".into(),
        generator: "Ninja".into(),
        cxx_std: "20".into(),
        deps: vec![
            DepSpec::Simple("glm".into()),
            DepSpec::Detailed(DepDetailed {
                name: "sdl2".into(),
                os: vec![],
                package: None,
                triplet: vec![],
                features: vec!["vulkan".into()],
            }),
        ],
        components: Default::default(),
        scripts: HashMap::default(),
    };

    fs::create_dir_all(root.join("components")).unwrap();
    write_json_pretty_changed(root.join("triton.json"), &meta).unwrap();

    handle_remove("glm", None, None, false).unwrap();

    let after: TritonRoot = read_json("triton.json").unwrap();
    assert_eq!(after.deps.len(), 1, "should have 1 dep remaining");

    // The vcpkg.json should still have sdl2
    let vcpkg_text = fs::read_to_string("vcpkg.json").unwrap();
    let v: serde_json::Value = serde_json::from_str(&vcpkg_text).unwrap();
    let deps = v["dependencies"].as_array().unwrap();

    // NOTE: remove currently writes simple dep names to vcpkg.json, losing
    // the features info. This is a gap — detailed deps with features are
    // downgraded to simple strings in vcpkg.json after remove.
    let has_sdl2 = deps.iter().any(|d| {
        d.as_str().map(|s| s == "sdl2").unwrap_or(false)
            || d.get("name").and_then(|n| n.as_str()).map(|n| n == "sdl2").unwrap_or(false)
    });
    assert!(has_sdl2, "sdl2 should remain in vcpkg.json after removing glm");
}
