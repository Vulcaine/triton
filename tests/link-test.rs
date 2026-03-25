use std::collections::HashMap;
use std::fs;
use std::path::Path;
use tempfile::tempdir;
use serial_test::serial;

use triton::handle_link;
use triton::models::{DepSpec, LinkEntry, TritonComponent, TritonRoot};
use triton::util::{read_json, write_json_pretty_changed};

mod test_utils;
use test_utils::write_minimal_resources;

/// Start a minimal project at `tmp`, with given deps and no components yet.
fn init_project(tmp: &Path, deps: &[DepSpec]) {
    fs::create_dir_all(tmp.join("components")).unwrap();
    let root = TritonRoot {
        app_name: "demo".into(),
        generator: "Ninja".into(),
        cxx_std: "20".into(),
        deps: deps.to_vec(),
        scripts: HashMap::default(),
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
    init_project(root, &[DepSpec::Simple("glm".into())]);
    std::env::set_current_dir(root).unwrap();

    // glm:Core  => Core depends on glm
    handle_link("glm", "Core").expect("link dep->component");

    // triton.json updated
    let proj: TritonRoot = read_json("triton.json").unwrap();
    let core = proj.components.get("Core").expect("Core exists");
    assert!(
        core.link.iter().any(|e| {
            let (n, _) = e.normalize();
            n == "glm"
        }),
        "Core should link to glm"
    );

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
        proj.components.insert(
            "Engine".into(),
            TritonComponent {
                kind: "lib".into(),
                link: vec![],
                defines: vec![],
                exports: vec![],
                resources: vec![],
                link_options: Default::default(),
                vendor_libs: Default::default(),
                assets: vec![],
            },
        );
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
    assert!(
        game.link.iter().any(|e| {
            let (n, _) = e.normalize();
            n == "Engine"
        }),
        "Game should depend on Engine"
    );

    let engine = proj.components.get("Engine").unwrap();
    assert!(
        !engine.link.iter().any(|e| {
            let (n, _) = e.normalize();
            n == "Game"
        }),
        "Engine must not depend on Game"
    );

    // CMake link + include wiring present
    let game_cm = read_to_string(root.join("components/Game/CMakeLists.txt"));
    assert!(
        game_cm.contains("target_link_libraries(${_comp_name} PRIVATE Engine)"),
        "Game CMake should link to Engine"
    );
    assert!(
        game_cm.contains(r#"${CMAKE_SOURCE_DIR}/Engine/include"#),
        "Game CMake should add Engine/include to include dirs"
    );
}

#[test]
#[serial]
fn rhs_cannot_be_a_dep() {
    let td = tempdir().unwrap();
    let root = td.path();
    write_minimal_resources(root);
    init_project(root, &[DepSpec::Simple("sdl2".into())]);
    std::env::set_current_dir(root).unwrap();

    // Engine:sdl2 => invalid (RHS must be a component)
    let err = handle_link("Engine", "sdl2").err().expect("should error");
    let msg = format!("{err:#}");
    assert!(
        msg.contains("Right-hand side") || msg.contains("must be a component"),
        "Expected RHS dep error, got: {msg}"
    );
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
    let count = b
        .link
        .iter()
        .filter(|e| {
            let (n, _) = e.normalize();
            n == "A"
        })
        .count();
    assert_eq!(count, 1, "Link A should appear exactly once in B.link");
}

#[test]
#[serial]
fn link_preserves_existing_defines() {
    let td = tempdir().unwrap();
    let root = td.path();
    write_minimal_resources(root);
    init_project(root, &[DepSpec::Simple("glm".into())]);
    std::env::set_current_dir(root).unwrap();

    // Pre-create a component with defines
    {
        let mut proj: TritonRoot = read_json("triton.json").unwrap();
        proj.components.insert(
            "Core".into(),
            TritonComponent {
                kind: "lib".into(),
                link: vec![],
                defines: vec!["MY_DEF".into()],
                exports: vec![],
                resources: vec![],
                link_options: Default::default(),
                vendor_libs: Default::default(),
                assets: vec![],
            },
        );
        write_json_pretty_changed("triton.json", &proj).unwrap();
        fs::create_dir_all("components/Core/src").unwrap();
        fs::create_dir_all("components/Core/include").unwrap();
        fs::write(
            "components/Core/CMakeLists.txt",
            read_to_string(root.join("resources/cmake_template.cmake")),
        )
        .unwrap();
    }

    handle_link("glm", "Core").expect("link glm->Core");

    let proj: TritonRoot = read_json("triton.json").unwrap();
    let core = proj.components.get("Core").expect("Core exists");
    assert!(
        core.defines.contains(&"MY_DEF".to_string()),
        "defines should still contain MY_DEF after linking"
    );
    assert!(
        core.link.iter().any(|e| {
            let (n, _) = e.normalize();
            n == "glm"
        }),
        "Core should link to glm"
    );
}

#[test]
#[serial]
fn link_preserves_exports() {
    let td = tempdir().unwrap();
    let root = td.path();
    write_minimal_resources(root);
    init_project(
        root,
        &[
            DepSpec::Simple("glm".into()),
            DepSpec::Simple("sdl2".into()),
        ],
    );
    std::env::set_current_dir(root).unwrap();

    // Pre-create a component with exports and an existing link
    {
        let mut proj: TritonRoot = read_json("triton.json").unwrap();
        proj.components.insert(
            "Core".into(),
            TritonComponent {
                kind: "lib".into(),
                link: vec![LinkEntry::Name("glm".into())],
                defines: vec![],
                exports: vec!["glm".into()],
                resources: vec![],
                link_options: Default::default(),
                vendor_libs: Default::default(),
                assets: vec![],
            },
        );
        write_json_pretty_changed("triton.json", &proj).unwrap();
        fs::create_dir_all("components/Core/src").unwrap();
        fs::create_dir_all("components/Core/include").unwrap();
        fs::write(
            "components/Core/CMakeLists.txt",
            read_to_string(root.join("resources/cmake_template.cmake")),
        )
        .unwrap();
    }

    handle_link("sdl2", "Core").expect("link sdl2->Core");

    let proj: TritonRoot = read_json("triton.json").unwrap();
    let core = proj.components.get("Core").expect("Core exists");
    assert!(
        core.exports.contains(&"glm".to_string()),
        "exports should still contain glm after linking sdl2"
    );
    assert!(
        core.link.iter().any(|e| {
            let (n, _) = e.normalize();
            n == "sdl2"
        }),
        "Core should now also link to sdl2"
    );
}

#[test]
#[serial]
fn link_dep_with_named_entry_package_hint() {
    let td = tempdir().unwrap();
    let root = td.path();
    write_minimal_resources(root);
    init_project(
        root,
        &[
            DepSpec::Simple("glm".into()),
            DepSpec::Simple("entt".into()),
        ],
    );
    std::env::set_current_dir(root).unwrap();

    // Pre-create a component with a Named LinkEntry (with package hint)
    {
        let mut proj: TritonRoot = read_json("triton.json").unwrap();
        proj.components.insert(
            "Core".into(),
            TritonComponent {
                kind: "lib".into(),
                link: vec![LinkEntry::Named {
                    name: "glm".into(),
                    package: Some("glm".into()),
                    targets: None,
                }],
                defines: vec![],
                exports: vec![],
                resources: vec![],
                link_options: Default::default(),
                vendor_libs: Default::default(),
                assets: vec![],
            },
        );
        write_json_pretty_changed("triton.json", &proj).unwrap();
        fs::create_dir_all("components/Core/src").unwrap();
        fs::create_dir_all("components/Core/include").unwrap();
        fs::write(
            "components/Core/CMakeLists.txt",
            read_to_string(root.join("resources/cmake_template.cmake")),
        )
        .unwrap();
    }

    handle_link("entt", "Core").expect("link entt->Core");

    let proj: TritonRoot = read_json("triton.json").unwrap();
    let core = proj.components.get("Core").expect("Core exists");

    // The Named entry for glm should still be present
    let has_named_glm = core.link.iter().any(|e| matches!(
        e,
        LinkEntry::Named { name, package, .. } if name == "glm" && package.as_deref() == Some("glm")
    ));
    assert!(has_named_glm, "Named glm entry with package hint should be preserved");

    // The new entt link should also be present
    assert!(
        core.link.iter().any(|e| {
            let (n, _) = e.normalize();
            n == "entt"
        }),
        "Core should also link to entt"
    );
}

#[test]
#[serial]
fn link_multiple_deps_sequentially() {
    let td = tempdir().unwrap();
    let root = td.path();
    write_minimal_resources(root);
    init_project(
        root,
        &[
            DepSpec::Simple("glm".into()),
            DepSpec::Simple("sdl2".into()),
        ],
    );
    std::env::set_current_dir(root).unwrap();

    handle_link("glm", "Core").expect("link glm->Core");
    handle_link("sdl2", "Core").expect("link sdl2->Core");

    let proj: TritonRoot = read_json("triton.json").unwrap();
    let core = proj.components.get("Core").expect("Core exists");
    assert!(
        core.link.iter().any(|e| {
            let (n, _) = e.normalize();
            n == "glm"
        }),
        "Core should link to glm"
    );
    assert!(
        core.link.iter().any(|e| {
            let (n, _) = e.normalize();
            n == "sdl2"
        }),
        "Core should link to sdl2"
    );
}

#[test]
#[serial]
fn link_creates_component_cmake_with_deps_markers() {
    let td = tempdir().unwrap();
    let root = td.path();
    write_minimal_resources(root);
    init_project(root, &[DepSpec::Simple("glm".into())]);
    std::env::set_current_dir(root).unwrap();

    handle_link("glm", "Render").expect("link glm->Render");

    let cm = read_to_string(root.join("components/Render/CMakeLists.txt"));
    assert!(
        cm.contains("triton:deps begin"),
        "CMakeLists.txt should contain 'triton:deps begin' marker"
    );
    assert!(
        cm.contains("triton:deps end"),
        "CMakeLists.txt should contain 'triton:deps end' marker"
    );
}
