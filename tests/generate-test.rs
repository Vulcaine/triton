use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::Path;

use serial_test::serial;
use tempfile::tempdir;

use triton::cmake::*;
use triton::commands::handle_generate;
use triton::commands::init::handle_init;
use triton::models::*;
use triton::util::{read_json, write_json_pretty_changed};

mod test_utils;
use test_utils::copy_offline_vcpkg_to;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn assert_exists(p: &Path) {
    assert!(p.exists(), "expected path to exist: {}", p.display());
}

fn write_file(path: impl AsRef<Path>, s: &str) {
    fs::create_dir_all(path.as_ref().parent().unwrap()).ok();
    fs::write(path, s).unwrap();
}

/// Minimal resources directory needed by the cmake template engine.
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
    {
      "name": "debug",
      "generator": "Ninja",
      "binaryDir": "${sourceDir}/../build/debug",
      "cacheVariables": { "CMAKE_BUILD_TYPE": "Debug" }
    }
  ],
  "buildPresets": [ { "name": "debug", "configurePreset": "debug" } ]
}"#,
    )
    .unwrap();
}

/// Create a component directory on disk with a marker CMakeLists.txt.
fn scaffold_component(root: &Path, name: &str) {
    let comp_dir = root.join("components").join(name);
    fs::create_dir_all(comp_dir.join("src")).unwrap();
    fs::create_dir_all(comp_dir.join("include")).unwrap();
    write_file(
        comp_dir.join("CMakeLists.txt"),
        &format!(
            r#"cmake_minimum_required(VERSION 3.25)
get_filename_component(_comp_name "${{CMAKE_CURRENT_SOURCE_DIR}}" NAME)
add_executable(${{_comp_name}})
set(_is_exe ON)
target_include_directories(${{_comp_name}} PRIVATE "include")
# ## triton:deps begin
# ## triton:deps end
"#
        ),
    );
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

// ===========================================================================
// 1. parse_cmake_version
// ===========================================================================

#[test]
fn parse_cmake_version_full() {
    assert_eq!(parse_cmake_version("3.30.1"), (3, 30, 1));
}

#[test]
fn parse_cmake_version_major_minor() {
    assert_eq!(parse_cmake_version("3.30"), (3, 30, 0));
}

#[test]
fn parse_cmake_version_major_only() {
    assert_eq!(parse_cmake_version("3"), (3, 0, 0));
}

#[test]
fn parse_cmake_version_empty() {
    assert_eq!(parse_cmake_version(""), (0, 0, 0));
}

#[test]
fn parse_cmake_version_large_numbers() {
    assert_eq!(parse_cmake_version("12.345.678"), (12, 345, 678));
}

#[test]
fn parse_cmake_version_non_numeric_parts() {
    // Non-numeric parts parse as 0
    assert_eq!(parse_cmake_version("abc.def.ghi"), (0, 0, 0));
}

#[test]
fn parse_cmake_version_trailing_dots() {
    assert_eq!(parse_cmake_version("3.30."), (3, 30, 0));
}

#[test]
fn min_cmake_version_is_valid() {
    let (maj, min, pat) = parse_cmake_version(MIN_CMAKE_VERSION);
    assert!(maj >= 3, "MIN_CMAKE_VERSION major should be >= 3");
    assert!(min > 0 || pat > 0, "MIN_CMAKE_VERSION should have nonzero minor or patch");
}

// ===========================================================================
// 2. dep_is_active
// ===========================================================================

#[test]
fn dep_is_active_simple_matches_name() {
    let dep = DepSpec::Simple("glm".into());
    assert!(dep_is_active(&dep, "glm", "windows", "x64-windows"));
}

#[test]
fn dep_is_active_simple_case_insensitive() {
    let dep = DepSpec::Simple("GLM".into());
    assert!(dep_is_active(&dep, "glm", "windows", "x64-windows"));
}

#[test]
fn dep_is_active_simple_no_match() {
    let dep = DepSpec::Simple("glm".into());
    assert!(!dep_is_active(&dep, "sdl2", "windows", "x64-windows"));
}

#[test]
fn dep_is_active_detailed_no_filters() {
    let dep = DepSpec::Detailed(DepDetailed {
        name: "sdl2".into(),
        os: vec![],
        package: None,
        triplet: vec![],
        features: vec![],
    });
    assert!(dep_is_active(&dep, "sdl2", "windows", "x64-windows"));
    assert!(dep_is_active(&dep, "sdl2", "linux", "x64-linux"));
}

#[test]
fn dep_is_active_detailed_windows_only() {
    let dep = DepSpec::Detailed(DepDetailed {
        name: "d3d12".into(),
        os: vec!["windows".into()],
        package: None,
        triplet: vec![],
        features: vec![],
    });
    assert!(dep_is_active(&dep, "d3d12", "windows", "x64-windows"));
    assert!(!dep_is_active(&dep, "d3d12", "linux", "x64-linux"));
    assert!(!dep_is_active(&dep, "d3d12", "macos", "arm64-osx"));
}

#[test]
fn dep_is_active_detailed_linux_only() {
    let dep = DepSpec::Detailed(DepDetailed {
        name: "x11".into(),
        os: vec!["linux".into()],
        package: None,
        triplet: vec![],
        features: vec![],
    });
    assert!(!dep_is_active(&dep, "x11", "windows", "x64-windows"));
    assert!(dep_is_active(&dep, "x11", "linux", "x64-linux"));
}

#[test]
fn dep_is_active_detailed_macos_aliases() {
    // Test various macOS aliases: "mac", "osx", "darwin", "macos"
    for alias in &["mac", "osx", "darwin", "macos"] {
        let dep = DepSpec::Detailed(DepDetailed {
            name: "metal".into(),
            os: vec![alias.to_string()],
            package: None,
            triplet: vec![],
            features: vec![],
        });
        assert!(
            dep_is_active(&dep, "metal", "macos", "arm64-osx"),
            "alias '{}' should match macos",
            alias
        );
    }
}

#[test]
fn dep_is_active_detailed_windows_alias_win() {
    let dep = DepSpec::Detailed(DepDetailed {
        name: "dx".into(),
        os: vec!["win".into()],
        package: None,
        triplet: vec![],
        features: vec![],
    });
    assert!(dep_is_active(&dep, "dx", "windows", "x64-windows"));
}

#[test]
fn dep_is_active_detailed_triplet_filter() {
    let dep = DepSpec::Detailed(DepDetailed {
        name: "sdl2".into(),
        os: vec![],
        package: None,
        triplet: vec!["x64-windows".into()],
        features: vec![],
    });
    assert!(dep_is_active(&dep, "sdl2", "windows", "x64-windows"));
    assert!(!dep_is_active(&dep, "sdl2", "windows", "arm64-windows"));
}

#[test]
fn dep_is_active_detailed_triplet_case_insensitive() {
    let dep = DepSpec::Detailed(DepDetailed {
        name: "sdl2".into(),
        os: vec![],
        package: None,
        triplet: vec!["X64-WINDOWS".into()],
        features: vec![],
    });
    assert!(dep_is_active(&dep, "sdl2", "windows", "x64-windows"));
}

#[test]
fn dep_is_active_detailed_os_and_triplet_combined() {
    let dep = DepSpec::Detailed(DepDetailed {
        name: "vulkan".into(),
        os: vec!["windows".into()],
        package: None,
        triplet: vec!["x64-windows".into()],
        features: vec![],
    });
    // Both match
    assert!(dep_is_active(&dep, "vulkan", "windows", "x64-windows"));
    // OS matches, triplet does not
    assert!(!dep_is_active(&dep, "vulkan", "windows", "arm64-windows"));
    // Triplet matches, OS does not
    assert!(!dep_is_active(&dep, "vulkan", "linux", "x64-windows"));
    // Neither matches
    assert!(!dep_is_active(&dep, "vulkan", "linux", "x64-linux"));
}

#[test]
fn dep_is_active_detailed_multiple_os() {
    let dep = DepSpec::Detailed(DepDetailed {
        name: "vulkan".into(),
        os: vec!["windows".into(), "linux".into()],
        package: None,
        triplet: vec![],
        features: vec![],
    });
    assert!(dep_is_active(&dep, "vulkan", "windows", "x64-windows"));
    assert!(dep_is_active(&dep, "vulkan", "linux", "x64-linux"));
    assert!(!dep_is_active(&dep, "vulkan", "macos", "arm64-osx"));
}

#[test]
fn dep_is_active_detailed_wrong_name() {
    let dep = DepSpec::Detailed(DepDetailed {
        name: "sdl2".into(),
        os: vec![],
        package: None,
        triplet: vec![],
        features: vec![],
    });
    assert!(!dep_is_active(&dep, "glm", "windows", "x64-windows"));
}

#[test]
fn dep_is_active_detailed_name_case_insensitive() {
    let dep = DepSpec::Detailed(DepDetailed {
        name: "SDL2".into(),
        os: vec![],
        package: None,
        triplet: vec![],
        features: vec![],
    });
    assert!(dep_is_active(&dep, "sdl2", "windows", "x64-windows"));
}

#[test]
fn dep_is_active_git_dep() {
    let dep = DepSpec::Git(GitDep {
        repo: "google/filament".into(),
        name: "filament".into(),
        branch: None,
        cmake: vec![],
    });
    assert!(dep_is_active(&dep, "filament", "windows", "x64-windows"));
    assert!(!dep_is_active(&dep, "other", "windows", "x64-windows"));
}

#[test]
fn dep_is_active_git_dep_case_insensitive() {
    let dep = DepSpec::Git(GitDep {
        repo: "google/filament".into(),
        name: "Filament".into(),
        branch: None,
        cmake: vec![],
    });
    assert!(dep_is_active(&dep, "filament", "windows", "x64-windows"));
}

// ===========================================================================
// 3. generate command integration (init + generate)
// ===========================================================================

#[test]
#[serial]
fn generate_after_init_produces_all_cmake_files() {
    let td = tempdir().unwrap();
    let proj = td.path().join("gen-proj");
    fs::create_dir_all(&proj).unwrap();
    copy_offline_vcpkg_to(&proj);
    std::env::set_current_dir(&proj).unwrap();

    handle_init(Some("."), "Ninja", "20").unwrap();

    // Add a dependency and component to triton.json
    let mut root: TritonRoot = read_json(proj.join("triton.json")).unwrap();
    root.deps.push(DepSpec::Simple("glm".into()));
    root.components.insert(
        "Engine".into(),
        TritonComponent {
            kind: "lib".into(),
            link: vec![LinkEntry::Name("glm".into())],
            defines: vec!["ENGINE_DEBUG".into()],
            ..default_component("lib")
        },
    );
    write_json_pretty_changed(proj.join("triton.json"), &root).unwrap();

    // Scaffold the Engine component directory
    scaffold_component(&proj, "Engine");

    // Run generate
    handle_generate().unwrap();

    // Verify root CMakeLists.txt
    let root_cm = fs::read_to_string(proj.join("components/CMakeLists.txt")).unwrap();
    assert!(
        root_cm.contains("cmake_minimum_required(VERSION"),
        "Root CMakeLists.txt should have cmake_minimum_required"
    );
    assert!(
        root_cm.contains("project(gen_proj LANGUAGES CXX)"),
        "Root CMakeLists.txt should have project() with sanitized name: got\n{}",
        root_cm
    );
    assert!(
        root_cm.contains("# ## triton:components begin"),
        "Root CMakeLists.txt should have components begin marker"
    );
    assert!(
        root_cm.contains("add_subdirectory(Engine)"),
        "Root CMakeLists.txt should include Engine subdirectory"
    );

    // Verify component CMakeLists.txt has triton:deps markers and dep content
    let engine_cm = fs::read_to_string(proj.join("components/Engine/CMakeLists.txt")).unwrap();
    assert!(
        engine_cm.contains("# ## triton:deps begin"),
        "Engine CMakeLists.txt should have deps begin marker"
    );
    assert!(
        engine_cm.contains("# ## triton:deps end"),
        "Engine CMakeLists.txt should have deps end marker"
    );
    assert!(
        engine_cm.contains("glm"),
        "Engine CMakeLists.txt should reference glm"
    );

    // Verify vcpkg.json is regenerated with deps
    let vcpkg: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(proj.join("vcpkg.json")).unwrap()).unwrap();
    let deps = vcpkg["dependencies"].as_array().expect("dependencies array");
    assert!(
        deps.iter().any(|d| d == "glm"),
        "vcpkg.json should contain glm"
    );

    // Verify CMakePresets.json exists
    assert_exists(&proj.join("components/CMakePresets.json"));
    let presets = fs::read_to_string(proj.join("components/CMakePresets.json")).unwrap();
    assert!(
        presets.contains("configurePresets"),
        "CMakePresets.json should have configurePresets"
    );
}

#[test]
#[serial]
fn generate_with_detailed_dep_features_produces_vcpkg_object() {
    let td = tempdir().unwrap();
    let proj = td.path().join("feat-proj");
    fs::create_dir_all(&proj).unwrap();
    copy_offline_vcpkg_to(&proj);
    std::env::set_current_dir(&proj).unwrap();

    handle_init(Some("."), "Ninja", "20").unwrap();

    let mut root: TritonRoot = read_json(proj.join("triton.json")).unwrap();
    root.deps.push(DepSpec::Detailed(DepDetailed {
        name: "sdl2".into(),
        os: vec![],
        package: None,
        triplet: vec![],
        features: vec!["vulkan".into(), "x11".into()],
    }));
    write_json_pretty_changed(proj.join("triton.json"), &root).unwrap();

    handle_generate().unwrap();

    let vcpkg: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(proj.join("vcpkg.json")).unwrap()).unwrap();
    let deps = vcpkg["dependencies"].as_array().expect("dependencies array");

    // Detailed dep with features should produce an object with "name" and "features"
    let sdl2_entry = deps
        .iter()
        .find(|d| {
            d.get("name")
                .and_then(|n| n.as_str())
                .map(|n| n == "sdl2")
                .unwrap_or(false)
        })
        .expect("should have sdl2 dep object in vcpkg.json");

    let features = sdl2_entry["features"].as_array().expect("features array");
    assert!(features.iter().any(|f| f == "vulkan"));
    assert!(features.iter().any(|f| f == "x11"));
}

#[test]
#[serial]
fn generate_skips_git_deps_in_vcpkg_json() {
    let td = tempdir().unwrap();
    let proj = td.path().join("git-proj");
    fs::create_dir_all(&proj).unwrap();
    copy_offline_vcpkg_to(&proj);
    std::env::set_current_dir(&proj).unwrap();

    handle_init(Some("."), "Ninja", "20").unwrap();

    let mut root: TritonRoot = read_json(proj.join("triton.json")).unwrap();
    root.deps.push(DepSpec::Git(GitDep {
        repo: "google/filament".into(),
        name: "filament".into(),
        branch: Some("main".into()),
        cmake: vec![],
    }));
    root.deps.push(DepSpec::Simple("glm".into()));
    write_json_pretty_changed(proj.join("triton.json"), &root).unwrap();

    handle_generate().unwrap();

    let vcpkg: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(proj.join("vcpkg.json")).unwrap()).unwrap();
    let deps = vcpkg["dependencies"].as_array().expect("dependencies array");

    // glm should be present, filament should NOT (git deps are not in vcpkg.json)
    assert!(deps.iter().any(|d| d == "glm"), "glm should be in vcpkg.json");
    assert!(
        !deps.iter().any(|d| {
            d.as_str().map(|s| s == "filament").unwrap_or(false)
                || d.get("name")
                    .and_then(|n| n.as_str())
                    .map(|n| n == "filament")
                    .unwrap_or(false)
        }),
        "filament (git dep) should NOT be in vcpkg.json"
    );
}

// ===========================================================================
// 4. rewrite_component_cmake
// ===========================================================================

#[test]
#[serial]
fn rewrite_component_cmake_with_defines() {
    let td = tempdir().unwrap();
    let root_path = td.path();
    std::env::set_current_dir(root_path).unwrap();
    write_minimal_resources(root_path);

    let mut components = BTreeMap::new();
    let comp = TritonComponent {
        defines: vec!["MY_DEFINE".into(), "FEATURE_X=1".into()],
        ..default_component("exe")
    };
    components.insert("App".into(), comp.clone());

    let root = TritonRoot {
        app_name: "testapp".into(),
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
        cm.contains("target_compile_definitions(${_comp_name} PRIVATE"),
        "Should contain target_compile_definitions"
    );
    assert!(cm.contains("MY_DEFINE"), "Should contain MY_DEFINE");
    assert!(cm.contains("FEATURE_X=1"), "Should contain FEATURE_X=1");
}

#[test]
#[serial]
fn rewrite_component_cmake_with_resources() {
    let td = tempdir().unwrap();
    let root_path = td.path();
    std::env::set_current_dir(root_path).unwrap();
    write_minimal_resources(root_path);

    let mut components = BTreeMap::new();
    let comp = TritonComponent {
        resources: vec!["resources".into(), "data/textures".into()],
        ..default_component("exe")
    };
    components.insert("Game".into(), comp.clone());

    let root = TritonRoot {
        app_name: "testapp".into(),
        generator: "Ninja".into(),
        cxx_std: "20".into(),
        deps: vec![],
        components,
        scripts: HashMap::default(),
    };

    scaffold_component(root_path, "Game");
    rewrite_component_cmake("Game", &root, &comp, (3, 30, 1)).unwrap();

    let cm = fs::read_to_string(root_path.join("components/Game/CMakeLists.txt")).unwrap();
    assert!(
        cm.contains("copy_directory"),
        "Should contain copy_directory for resources"
    );
    assert!(
        cm.contains("resources"),
        "Should reference 'resources' directory"
    );
    assert!(
        cm.contains("textures"),
        "Should reference 'textures' (basename of data/textures)"
    );
}

#[test]
#[serial]
fn rewrite_component_cmake_with_link_options_all() {
    let td = tempdir().unwrap();
    let root_path = td.path();
    std::env::set_current_dir(root_path).unwrap();
    write_minimal_resources(root_path);

    let mut components = BTreeMap::new();
    let comp = TritonComponent {
        link_options: LinkOptions::All(vec![
            "-Wl,--export-dynamic".into(),
            "-lpthread".into(),
        ]),
        ..default_component("exe")
    };
    components.insert("Server".into(), comp.clone());

    let root = TritonRoot {
        app_name: "testapp".into(),
        generator: "Ninja".into(),
        cxx_std: "20".into(),
        deps: vec![],
        components,
        scripts: HashMap::default(),
    };

    scaffold_component(root_path, "Server");
    rewrite_component_cmake("Server", &root, &comp, (3, 30, 1)).unwrap();

    let cm = fs::read_to_string(root_path.join("components/Server/CMakeLists.txt")).unwrap();
    assert!(
        cm.contains("target_link_options(${_comp_name} PRIVATE"),
        "Should contain target_link_options"
    );
    assert!(
        cm.contains("--export-dynamic"),
        "Should contain --export-dynamic"
    );
    assert!(cm.contains("lpthread"), "Should contain lpthread");
}

#[test]
#[serial]
fn rewrite_component_cmake_with_link_options_per_platform() {
    let td = tempdir().unwrap();
    let root_path = td.path();
    std::env::set_current_dir(root_path).unwrap();
    write_minimal_resources(root_path);

    let mut platform_opts = BTreeMap::new();
    platform_opts.insert("windows".into(), vec!["/SUBSYSTEM:WINDOWS".into()]);
    platform_opts.insert("macos".into(), vec!["-framework".into(), "Cocoa".into()]);
    platform_opts.insert("linux".into(), vec!["-Wl,--export-dynamic".into()]);

    let mut components = BTreeMap::new();
    let comp = TritonComponent {
        link_options: LinkOptions::PerPlatform(platform_opts),
        ..default_component("exe")
    };
    components.insert("App".into(), comp.clone());

    let root = TritonRoot {
        app_name: "testapp".into(),
        generator: "Ninja".into(),
        cxx_std: "20".into(),
        deps: vec![],
        components,
        scripts: HashMap::default(),
    };

    scaffold_component(root_path, "App");
    rewrite_component_cmake("App", &root, &comp, (3, 30, 1)).unwrap();

    let cm = fs::read_to_string(root_path.join("components/App/CMakeLists.txt")).unwrap();
    assert!(cm.contains("if(WIN32)"), "Should contain if(WIN32) block");
    assert!(cm.contains("if(APPLE)"), "Should contain if(APPLE) block");
    assert!(
        cm.contains("if(UNIX AND NOT APPLE)"),
        "Should contain if(UNIX AND NOT APPLE) block for linux"
    );
    assert!(
        cm.contains("SUBSYSTEM:WINDOWS"),
        "Should contain Windows link option"
    );
    assert!(cm.contains("Cocoa"), "Should contain macOS Cocoa framework");
    assert!(
        cm.contains("export-dynamic"),
        "Should contain Linux export-dynamic"
    );
}

#[test]
#[serial]
fn rewrite_component_cmake_with_vendor_libs_all() {
    let td = tempdir().unwrap();
    let root_path = td.path();
    std::env::set_current_dir(root_path).unwrap();
    write_minimal_resources(root_path);

    let mut components = BTreeMap::new();
    let comp = TritonComponent {
        vendor_libs: VendorLibs::All(vec![
            "vendor/lib/libfoo.a".into(),
            "vendor/lib/libbar.a".into(),
        ]),
        ..default_component("exe")
    };
    components.insert("App".into(), comp.clone());

    let root = TritonRoot {
        app_name: "testapp".into(),
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
        cm.contains("target_link_libraries(${_comp_name} PRIVATE"),
        "Should contain target_link_libraries with vendor libs"
    );
    assert!(
        cm.contains("vendor/lib/libfoo.a"),
        "Should contain libfoo.a path"
    );
    assert!(
        cm.contains("vendor/lib/libbar.a"),
        "Should contain libbar.a path"
    );
}

#[test]
#[serial]
fn rewrite_component_cmake_with_vendor_libs_per_platform() {
    let td = tempdir().unwrap();
    let root_path = td.path();
    std::env::set_current_dir(root_path).unwrap();
    write_minimal_resources(root_path);

    let mut platform_libs = BTreeMap::new();
    platform_libs.insert(
        "windows".into(),
        vec!["vendor/win/nethost.lib".into()],
    );
    platform_libs.insert(
        "linux".into(),
        vec!["vendor/linux/libnethost.a".into()],
    );

    let mut components = BTreeMap::new();
    let comp = TritonComponent {
        vendor_libs: VendorLibs::PerPlatform(platform_libs),
        ..default_component("exe")
    };
    components.insert("App".into(), comp.clone());

    let root = TritonRoot {
        app_name: "testapp".into(),
        generator: "Ninja".into(),
        cxx_std: "20".into(),
        deps: vec![],
        components,
        scripts: HashMap::default(),
    };

    scaffold_component(root_path, "App");
    rewrite_component_cmake("App", &root, &comp, (3, 30, 1)).unwrap();

    let cm = fs::read_to_string(root_path.join("components/App/CMakeLists.txt")).unwrap();
    assert!(cm.contains("if(WIN32)"), "Should contain if(WIN32) for Windows vendor libs");
    assert!(
        cm.contains("if(UNIX AND NOT APPLE)"),
        "Should contain if(UNIX AND NOT APPLE) for Linux vendor libs"
    );
    assert!(
        cm.contains("vendor/win/nethost.lib"),
        "Should contain Windows vendor lib path"
    );
    assert!(
        cm.contains("vendor/linux/libnethost.a"),
        "Should contain Linux vendor lib path"
    );
}

#[test]
#[serial]
fn rewrite_component_cmake_with_assets() {
    let td = tempdir().unwrap();
    let root_path = td.path();
    std::env::set_current_dir(root_path).unwrap();
    write_minimal_resources(root_path);

    let mut components = BTreeMap::new();
    let comp = TritonComponent {
        assets: vec!["shaders".into(), "config.ini".into()],
        ..default_component("exe")
    };
    components.insert("App".into(), comp.clone());

    let root = TritonRoot {
        app_name: "testapp".into(),
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
        "Should contain asset staging comment"
    );
    assert!(
        cm.contains("_triton_asset_stamps"),
        "Should contain stamp variable"
    );
    assert!(
        cm.contains("_triton_asset_src_shaders"),
        "Should contain shaders asset source variable"
    );
    assert!(
        cm.contains("_triton_asset_src_config_ini"),
        "Should contain config_ini asset source variable"
    );
    assert!(
        cm.contains("${_comp_name}_assets"),
        "Should contain custom target for assets"
    );
    assert!(
        cm.contains("add_dependencies(${_comp_name} ${_comp_name}_assets)"),
        "Should wire asset target as dependency"
    );
}

#[test]
#[serial]
fn rewrite_component_cmake_with_exports_marks_public() {
    let td = tempdir().unwrap();
    let root_path = td.path();
    std::env::set_current_dir(root_path).unwrap();
    write_minimal_resources(root_path);

    let mut components = BTreeMap::new();
    let comp = TritonComponent {
        link: vec![LinkEntry::Named {
            name: "glm".into(),
            package: Some("glm".into()),
            targets: Some(vec!["glm::glm".into()]),
        }],
        exports: vec!["glm".into()],
        ..default_component("lib")
    };
    components.insert("Engine".into(), comp.clone());

    let root = TritonRoot {
        app_name: "testapp".into(),
        generator: "Ninja".into(),
        cxx_std: "20".into(),
        deps: vec![DepSpec::Simple("glm".into())],
        components,
        scripts: HashMap::default(),
    };

    scaffold_component(root_path, "Engine");
    rewrite_component_cmake("Engine", &root, &comp, (3, 30, 1)).unwrap();

    let cm = fs::read_to_string(root_path.join("components/Engine/CMakeLists.txt")).unwrap();
    // Exported deps should be PUBLIC
    assert!(
        cm.contains("PUBLIC"),
        "Exported dep should use PUBLIC visibility"
    );
    assert!(
        cm.contains("find_package(glm CONFIG REQUIRED)"),
        "Should find_package for glm with explicit package hint"
    );
}

#[test]
#[serial]
fn rewrite_component_cmake_skips_missing_dir() {
    let td = tempdir().unwrap();
    let root_path = td.path();
    std::env::set_current_dir(root_path).unwrap();

    // Do NOT create the component dir
    fs::create_dir_all(root_path.join("components")).unwrap();

    let comp = default_component("exe");
    let root = TritonRoot {
        app_name: "testapp".into(),
        generator: "Ninja".into(),
        cxx_std: "20".into(),
        deps: vec![],
        components: BTreeMap::new(),
        scripts: HashMap::default(),
    };

    // This should succeed (skip silently) without the directory
    let result = rewrite_component_cmake("NonExistent", &root, &comp, (3, 30, 1));
    assert!(result.is_ok(), "Should silently skip missing component dir");
    assert!(
        !root_path
            .join("components/NonExistent/CMakeLists.txt")
            .exists(),
        "Should not create files for missing component"
    );
}

#[test]
#[serial]
fn rewrite_component_cmake_updates_cmake_minimum_required() {
    let td = tempdir().unwrap();
    let root_path = td.path();
    std::env::set_current_dir(root_path).unwrap();
    write_minimal_resources(root_path);

    let comp = default_component("exe");
    let mut components = BTreeMap::new();
    components.insert("App".into(), comp.clone());

    let root = TritonRoot {
        app_name: "testapp".into(),
        generator: "Ninja".into(),
        cxx_std: "20".into(),
        deps: vec![],
        components,
        scripts: HashMap::default(),
    };

    scaffold_component(root_path, "App");

    // Write with old version
    rewrite_component_cmake("App", &root, &comp, (3, 28, 0)).unwrap();
    let cm1 = fs::read_to_string(root_path.join("components/App/CMakeLists.txt")).unwrap();
    assert!(
        cm1.contains("cmake_minimum_required(VERSION 3.28.0)"),
        "Should have version 3.28.0"
    );

    // Rewrite with new version
    rewrite_component_cmake("App", &root, &comp, (3, 30, 1)).unwrap();
    let cm2 = fs::read_to_string(root_path.join("components/App/CMakeLists.txt")).unwrap();
    assert!(
        cm2.contains("cmake_minimum_required(VERSION 3.30.1)"),
        "Should have updated version 3.30.1"
    );
}

// ===========================================================================
// 5. regenerate_root_cmake
// ===========================================================================

#[test]
#[serial]
fn regenerate_root_cmake_includes_subdirectories() {
    let td = tempdir().unwrap();
    let root_path = td.path();
    std::env::set_current_dir(root_path).unwrap();
    write_minimal_resources(root_path);

    // Create component dirs on disk
    scaffold_component(root_path, "Engine");
    scaffold_component(root_path, "Game");
    scaffold_component(root_path, "tests");

    let mut components = BTreeMap::new();
    components.insert("Engine".into(), default_component("lib"));
    components.insert("Game".into(), default_component("exe"));
    components.insert("tests".into(), default_component("exe"));

    let root = TritonRoot {
        app_name: "myapp".into(),
        generator: "Ninja".into(),
        cxx_std: "20".into(),
        deps: vec![],
        components,
        scripts: HashMap::default(),
    };

    regenerate_root_cmake(&root).unwrap();

    let cm = fs::read_to_string(root_path.join("components/CMakeLists.txt")).unwrap();
    assert!(
        cm.contains("cmake_minimum_required(VERSION"),
        "Should have cmake_minimum_required"
    );
    assert!(
        cm.contains("project(myapp LANGUAGES CXX)"),
        "Should have project(myapp)"
    );
    assert!(
        cm.contains("# ## triton:components begin"),
        "Should have components begin marker"
    );
    assert!(
        cm.contains("# ## triton:components end"),
        "Should have components end marker"
    );
    assert!(
        cm.contains("add_subdirectory(Engine)"),
        "Should include Engine subdirectory"
    );
    assert!(
        cm.contains("add_subdirectory(Game)"),
        "Should include Game subdirectory"
    );
    assert!(
        cm.contains("add_subdirectory(tests)"),
        "Should include tests subdirectory"
    );
}

#[test]
#[serial]
fn regenerate_root_cmake_skips_components_without_dir() {
    let td = tempdir().unwrap();
    let root_path = td.path();
    std::env::set_current_dir(root_path).unwrap();
    write_minimal_resources(root_path);

    // Only create Engine on disk, not Phantom
    scaffold_component(root_path, "Engine");
    fs::create_dir_all(root_path.join("components")).unwrap();

    let mut components = BTreeMap::new();
    components.insert("Engine".into(), default_component("lib"));
    components.insert("Phantom".into(), default_component("exe"));

    let root = TritonRoot {
        app_name: "myapp".into(),
        generator: "Ninja".into(),
        cxx_std: "20".into(),
        deps: vec![],
        components,
        scripts: HashMap::default(),
    };

    regenerate_root_cmake(&root).unwrap();

    let cm = fs::read_to_string(root_path.join("components/CMakeLists.txt")).unwrap();
    assert!(
        cm.contains("add_subdirectory(Engine)"),
        "Should include Engine (directory exists)"
    );
    assert!(
        !cm.contains("add_subdirectory(Phantom)"),
        "Should NOT include Phantom (directory missing)"
    );
}

#[test]
#[serial]
fn regenerate_root_cmake_sanitizes_app_name_with_hyphens() {
    let td = tempdir().unwrap();
    let root_path = td.path();
    std::env::set_current_dir(root_path).unwrap();
    write_minimal_resources(root_path);

    fs::create_dir_all(root_path.join("components")).unwrap();

    let root = TritonRoot {
        app_name: "my-cool-app".into(),
        generator: "Ninja".into(),
        cxx_std: "20".into(),
        deps: vec![],
        components: BTreeMap::new(),
        scripts: HashMap::default(),
    };

    regenerate_root_cmake(&root).unwrap();

    let cm = fs::read_to_string(root_path.join("components/CMakeLists.txt")).unwrap();
    assert!(
        cm.contains("project(my_cool_app LANGUAGES CXX)"),
        "Hyphens should be replaced with underscores in project name"
    );
}

#[test]
#[serial]
fn regenerate_root_cmake_components_sorted_alphabetically() {
    let td = tempdir().unwrap();
    let root_path = td.path();
    std::env::set_current_dir(root_path).unwrap();
    write_minimal_resources(root_path);

    scaffold_component(root_path, "Zebra");
    scaffold_component(root_path, "Alpha");
    scaffold_component(root_path, "Middle");

    let mut components = BTreeMap::new();
    components.insert("Zebra".into(), default_component("lib"));
    components.insert("Alpha".into(), default_component("lib"));
    components.insert("Middle".into(), default_component("lib"));

    let root = TritonRoot {
        app_name: "sorttest".into(),
        generator: "Ninja".into(),
        cxx_std: "20".into(),
        deps: vec![],
        components,
        scripts: HashMap::default(),
    };

    regenerate_root_cmake(&root).unwrap();

    let cm = fs::read_to_string(root_path.join("components/CMakeLists.txt")).unwrap();
    let alpha_pos = cm.find("add_subdirectory(Alpha)").expect("Alpha present");
    let middle_pos = cm.find("add_subdirectory(Middle)").expect("Middle present");
    let zebra_pos = cm.find("add_subdirectory(Zebra)").expect("Zebra present");
    assert!(
        alpha_pos < middle_pos && middle_pos < zebra_pos,
        "Components should be sorted alphabetically"
    );
}

// ===========================================================================
// Additional edge-case tests
// ===========================================================================

#[test]
#[serial]
fn rewrite_component_cmake_with_empty_defines_no_output() {
    let td = tempdir().unwrap();
    let root_path = td.path();
    std::env::set_current_dir(root_path).unwrap();
    write_minimal_resources(root_path);

    let comp = TritonComponent {
        defines: vec![],
        ..default_component("exe")
    };
    let mut components = BTreeMap::new();
    components.insert("App".into(), comp.clone());

    let root = TritonRoot {
        app_name: "testapp".into(),
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
        !cm.contains("target_compile_definitions"),
        "Empty defines should not produce target_compile_definitions"
    );
}

#[test]
#[serial]
fn rewrite_component_cmake_with_empty_assets_no_output() {
    let td = tempdir().unwrap();
    let root_path = td.path();
    std::env::set_current_dir(root_path).unwrap();
    write_minimal_resources(root_path);

    let comp = TritonComponent {
        assets: vec![],
        ..default_component("exe")
    };
    let mut components = BTreeMap::new();
    components.insert("App".into(), comp.clone());

    let root = TritonRoot {
        app_name: "testapp".into(),
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
        !cm.contains("_triton_asset_stamps"),
        "Empty assets should not produce asset staging code"
    );
}

#[test]
#[serial]
fn rewrite_component_cmake_with_git_dep_generates_subdirectory() {
    let td = tempdir().unwrap();
    let root_path = td.path();
    std::env::set_current_dir(root_path).unwrap();
    write_minimal_resources(root_path);

    let mut components = BTreeMap::new();
    let comp = TritonComponent {
        link: vec![LinkEntry::Named {
            name: "filament".into(),
            package: None,
            targets: Some(vec!["filament".into(), "utils".into()]),
        }],
        ..default_component("exe")
    };
    components.insert("App".into(), comp.clone());

    let root = TritonRoot {
        app_name: "testapp".into(),
        generator: "Ninja".into(),
        cxx_std: "20".into(),
        deps: vec![DepSpec::Git(GitDep {
            repo: "google/filament".into(),
            name: "filament".into(),
            branch: Some("main".into()),
            cmake: vec![],
        })],
        components,
        scripts: HashMap::default(),
    };

    scaffold_component(root_path, "App");
    rewrite_component_cmake("App", &root, &comp, (3, 30, 1)).unwrap();

    let cm = fs::read_to_string(root_path.join("components/App/CMakeLists.txt")).unwrap();
    assert!(
        cm.contains("third_party/filament"),
        "Should reference third_party/filament for git dep"
    );
    assert!(
        cm.contains("add_subdirectory"),
        "Should add_subdirectory for git dep"
    );
}

#[test]
fn detect_vcpkg_triplet_returns_nonempty() {
    let triplet = detect_vcpkg_triplet();
    assert!(
        !triplet.is_empty(),
        "detect_vcpkg_triplet should return a non-empty string"
    );
    // Should contain a known platform identifier
    assert!(
        triplet.contains("windows") || triplet.contains("linux") || triplet.contains("osx"),
        "triplet should contain a platform identifier, got: {}",
        triplet
    );
}

#[test]
fn effective_cmake_version_at_least_minimum() {
    let (maj, min, pat) = effective_cmake_version();
    let (min_maj, min_min, min_pat) = parse_cmake_version(MIN_CMAKE_VERSION);
    assert!(
        (maj, min, pat) >= (min_maj, min_min, min_pat),
        "effective_cmake_version should be >= MIN_CMAKE_VERSION"
    );
}
