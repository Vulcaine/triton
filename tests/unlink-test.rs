//! Tests for `triton unlink` command.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::Path;

use serial_test::serial;
use tempfile::tempdir;

use triton::commands::unlink::handle_unlink;
use triton::models::*;
use triton::util::{read_json, write_json_pretty_changed};

mod test_utils;
use test_utils::write_file;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn write_minimal_resources(root: &Path) {
    let res = root.join("resources");
    fs::create_dir_all(&res).unwrap();
    fs::write(
        res.join("cmake_template.cmake"),
        r#"cmake_minimum_required(VERSION 3.25)
get_filename_component(_comp_name "${CMAKE_CURRENT_SOURCE_DIR}" NAME)
add_library(${_comp_name})
set(_is_exe OFF)
target_include_directories(${_comp_name} PUBLIC "include")
# ## triton:deps begin
# ## triton:deps end
"#,
    )
    .unwrap();
    fs::write(res.join("cmake_root_template.cmake"), "# stub\n").unwrap();
    fs::write(
        res.join("cmake_presets_template.json"),
        r#"{ "version": 6, "configurePresets": [], "buildPresets": [] }"#,
    )
    .unwrap();
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

fn seed_project(root: &Path) {
    let mut components = BTreeMap::new();
    components.insert(
        "Game".into(),
        TritonComponent {
            kind: "exe".into(),
            link: vec![
                LinkEntry::Name("glm".into()),
                LinkEntry::Name("sdl2".into()),
                LinkEntry::Name("Core".into()),
            ],
            ..Default::default()
        },
    );
    components.insert(
        "Core".into(),
        TritonComponent {
            kind: "lib".into(),
            link: vec![LinkEntry::Name("glm".into())],
            ..Default::default()
        },
    );
    components.insert(
        "Render".into(),
        TritonComponent {
            kind: "lib".into(),
            link: vec![
                LinkEntry::Name("sdl2".into()),
                LinkEntry::Name("Core".into()),
            ],
            ..Default::default()
        },
    );

    let tr = TritonRoot {
        app_name: "demo".into(),
        generator: "Ninja".into(),
        cxx_std: "20".into(),
        deps: vec![
            DepSpec::Simple("glm".into()),
            DepSpec::Simple("sdl2".into()),
        ],
        components,
        scripts: HashMap::default(),
    };

    fs::create_dir_all(root.join("components")).unwrap();
    write_json_pretty_changed(root.join("triton.json"), &tr).unwrap();
    write_minimal_resources(root);
    scaffold_component(root, "Game");
    scaffold_component(root, "Core");
    scaffold_component(root, "Render");
}

// ===========================================================================
// Tests
// ===========================================================================

#[test]
#[serial]
fn unlink_from_specific_component() {
    let td = tempdir().unwrap();
    std::env::set_current_dir(td.path()).unwrap();
    seed_project(td.path());

    // Game links to [glm, sdl2, Core]. Unlink sdl2 from Game.
    handle_unlink("sdl2", Some("Game")).unwrap();

    let tr: TritonRoot = read_json("triton.json").unwrap();
    let game = tr.components.get("Game").unwrap();

    assert!(
        !game.link.iter().any(|e| e.normalize().0 == "sdl2"),
        "sdl2 should be unlinked from Game"
    );
    assert!(
        game.link.iter().any(|e| e.normalize().0 == "glm"),
        "glm should still be linked to Game"
    );
    assert!(
        game.link.iter().any(|e| e.normalize().0 == "Core"),
        "Core should still be linked to Game"
    );
}

#[test]
#[serial]
fn unlink_from_all_components() {
    let td = tempdir().unwrap();
    std::env::set_current_dir(td.path()).unwrap();
    seed_project(td.path());

    // sdl2 is linked in Game and Render. Unlink from all.
    handle_unlink("sdl2", None).unwrap();

    let tr: TritonRoot = read_json("triton.json").unwrap();

    for (name, comp) in &tr.components {
        assert!(
            !comp.link.iter().any(|e| e.normalize().0 == "sdl2"),
            "sdl2 should be unlinked from {} but wasn't",
            name
        );
    }

    // glm should still be in Game and Core
    let game = tr.components.get("Game").unwrap();
    assert!(game.link.iter().any(|e| e.normalize().0 == "glm"));
}

#[test]
#[serial]
fn unlink_nonexistent_link_is_noop() {
    let td = tempdir().unwrap();
    std::env::set_current_dir(td.path()).unwrap();
    seed_project(td.path());

    // "boost" is not linked anywhere — should succeed silently
    let result = handle_unlink("boost", Some("Game"));
    assert!(result.is_ok());

    // Nothing changed
    let tr: TritonRoot = read_json("triton.json").unwrap();
    let game = tr.components.get("Game").unwrap();
    assert_eq!(game.link.len(), 3);
}

#[test]
#[serial]
fn unlink_from_missing_component_errors() {
    let td = tempdir().unwrap();
    std::env::set_current_dir(td.path()).unwrap();
    seed_project(td.path());

    let result = handle_unlink("glm", Some("NonExistent"));
    assert!(result.is_err());
    let msg = format!("{:#}", result.unwrap_err());
    assert!(msg.contains("No such component"), "got: {msg}");
}

#[test]
#[serial]
fn unlink_component_dep_preserves_deps_list() {
    let td = tempdir().unwrap();
    std::env::set_current_dir(td.path()).unwrap();
    seed_project(td.path());

    // Unlink glm from Core — should NOT remove glm from root.deps
    handle_unlink("glm", Some("Core")).unwrap();

    let tr: TritonRoot = read_json("triton.json").unwrap();
    assert!(
        tr.deps.iter().any(|d| d.name() == "glm"),
        "glm should still be in root.deps (unlink only removes the link, not the dep)"
    );
}

#[test]
#[serial]
fn unlink_case_insensitive() {
    let td = tempdir().unwrap();
    std::env::set_current_dir(td.path()).unwrap();
    seed_project(td.path());

    // Unlink "GLM" (uppercase) from Game — should match "glm"
    handle_unlink("GLM", Some("Game")).unwrap();

    let tr: TritonRoot = read_json("triton.json").unwrap();
    let game = tr.components.get("Game").unwrap();
    assert!(
        !game.link.iter().any(|e| e.normalize().0.eq_ignore_ascii_case("glm")),
        "glm should be unlinked (case-insensitive)"
    );
}

#[test]
#[serial]
fn unlink_regenerates_cmake() {
    let td = tempdir().unwrap();
    std::env::set_current_dir(td.path()).unwrap();
    seed_project(td.path());

    handle_unlink("sdl2", Some("Game")).unwrap();

    // CMakeLists.txt should exist and be regenerated
    let cm = fs::read_to_string(td.path().join("components/Game/CMakeLists.txt")).unwrap();
    assert!(
        cm.contains("triton:deps begin"),
        "CMakeLists.txt should still have managed region"
    );
}
