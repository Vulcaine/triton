use std::fs;
use std::path::{Path};
use tempfile::tempdir;
use serial_test::serial;

use triton::handle_link;
use triton::models::{RootDep, TritonComponent, TritonRoot};
use triton::util::{read_json, write_json_pretty_changed};

/// Write minimal template files under ./resources that the generator expects.
fn write_minimal_resources(root: &Path) {
    let res = root.join("resources");
    fs::create_dir_all(&res).unwrap();

    // Per-component CMake template: must include the managed region markers.
    fs::write(
        res.join("cmake_template.cmake"),
        r#"cmake_minimum_required(VERSION 3.25)
get_filename_component(_comp_name "${CMAKE_CURRENT_SOURCE_DIR}" NAME)

# Rule: exe if main.cpp exists; else lib
if(EXISTS "${CMAKE_CURRENT_SOURCE_DIR}/src/main.cpp")
  add_executable(${_comp_name})
  set(_is_exe ON)
else()
  add_library(${_comp_name})
  set(_is_exe OFF)
endif()

# Export headers: libs -> PUBLIC, exe -> PRIVATE
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

    // Root helpers template (the big helper block). For tests a stub is enough.
    fs::write(res.join("cmake_root_template.cmake"), "# (helpers stub)\n").unwrap();

    // Presets template (not used by these tests but keep it valid).
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

/// Start a minimal project at `tmp`, with given deps and no components yet.
fn init_project(tmp: &Path, deps: &[RootDep]) {
    fs::create_dir_all(tmp.join("components")).unwrap();
    let root = TritonRoot {
        app_name: "demo".into(),
        triplet: "x64-windows".into(),
        generator: "Ninja".into(),
        cxx_std: "20".into(),
        deps: deps.to_vec(),
        components: Default::default(),
    };
    write_json_pretty_changed(tmp.join("triton.json"), &root).unwrap();
}

fn read_to_string(p: impl AsRef<Path>) -> String {
    fs::read_to_string(p.as_ref())
        .unwrap_or_else(|e| panic!("read {} failed: {e}", p.as_ref().display()))
}

#[test]
#[serial]
fn link_dep_into_component_adds_dep_and_generates_cmake() {
    let td = tempdir().unwrap();
    let root = td.path();
    write_minimal_resources(root);
    init_project(root, &[RootDep::Name("glm".into())]);
    std::env::set_current_dir(root).unwrap();

    // glm:Core  => Core depends on glm
    handle_link("glm", "Core").expect("link dep->component");

    // triton.json updated
    let proj: TritonRoot = read_json("triton.json").unwrap();
    let core = proj.components.get("Core").expect("Core exists");
    assert!(core.link.iter().any(|e| {
        let (n, _) = e.normalize();
        n == "glm"
    }), "Core should link to glm");

    // component CMakeLists has strict vcpkg discovery line or a find_package path
    let cm = read_to_string(root.join("components/Core/CMakeLists.txt"));
    assert!(
        cm.contains(r#"triton_find_vcpkg_and_link_strict(${_comp_name} "glm")"#)
            || cm.contains("find_package(glm"),
        "Expected glm discovery/link in Core CMake"
    );
}

#[test]
#[serial]
fn link_component_to_component_is_rhs_directional() {
    let td = tempdir().unwrap();
    let root = td.path();
    write_minimal_resources(root);
    init_project(root, &[]);
    std::env::set_current_dir(root).unwrap();

    // Pre-create Engine as a component
    {
        let mut proj: TritonRoot = read_json("triton.json").unwrap();
        proj
            .components
            .insert("Engine".into(), TritonComponent { kind: "lib".into(), link: vec![], defines: vec![], exports: vec![] });
        write_json_pretty_changed("triton.json", &proj).unwrap();
        fs::create_dir_all("components/Engine/src").unwrap();
        fs::create_dir_all("components/Engine/include").unwrap();
        fs::write(
            "components/Engine/CMakeLists.txt",
            read_to_string(root.join("resources/cmake_template.cmake")),
        )
        .unwrap();
    }

    // Engine:Game  => Game depends on Engine
    handle_link("Engine", "Game").expect("link comp->comp");

    // JSON wiring: Game.link contains Engine, Engine does NOT contain Game
    let proj: TritonRoot = read_json("triton.json").unwrap();
    let game = proj.components.get("Game").expect("Game exists");
    assert!(game.link.iter().any(|e| {
        let (n, _) = e.normalize();
        n == "Engine"
    }), "Game should depend on Engine");

    let engine = proj.components.get("Engine").unwrap();
    assert!(!engine.link.iter().any(|e| {
        let (n, _) = e.normalize();
        n == "Game"
    }), "Engine must not depend on Game");

    // CMake link + include wiring present
    let game_cm = read_to_string(root.join("components/Game/CMakeLists.txt"));
    assert!(game_cm.contains("target_link_libraries(${_comp_name} PRIVATE Engine)"),
        "Game CMake should link to Engine");
    assert!(game_cm.contains(r#"${CMAKE_SOURCE_DIR}/Engine/include"#),
        "Game CMake should add Engine/include to include dirs");
}

#[test]
#[serial]
fn rhs_cannot_be_a_dep() {
    let td = tempdir().unwrap();
    let root = td.path();
    write_minimal_resources(root);
    init_project(root, &[RootDep::Name("sdl2".into())]);
    std::env::set_current_dir(root).unwrap();

    // Engine:sdl2 => invalid (RHS must be a component)
    let err = handle_link("Engine", "sdl2").err().expect("should error");
    let msg = format!("{err:#}");
    assert!(msg.contains("Right-hand side") || msg.contains("must be a component"),
        "Expected RHS dep error, got: {msg}");
}

#[test]
#[serial]
fn linking_is_idempotent() {
    let td = tempdir().unwrap();
    let root = td.path();
    write_minimal_resources(root);
    init_project(root, &[]);
    std::env::set_current_dir(root).unwrap();

    handle_link("A", "B").unwrap();
    handle_link("A", "B").unwrap();

    let proj: TritonRoot = read_json("triton.json").unwrap();
    let b = proj.components.get("B").unwrap();
    let count = b.link.iter().filter(|e| {
        let (n, _) = e.normalize();
        n == "A"
    }).count();
    assert_eq!(count, 1, "Link A should appear exactly once in B.link");
}
