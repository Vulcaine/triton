use std::collections::HashMap;
use std::fs;
use std::path::Path;
use tempfile::tempdir;

use serial_test::serial;

use triton::commands::remove_component::handle_remove_component;
use triton::models::{DepSpec, LinkEntry, TritonComponent, TritonRoot};
use triton::util::{read_json, write_json_pretty_changed};

mod test_utils;
use test_utils::{write_file, write_minimal_resources, CwdGuard};

fn seed_project(root: &Path, tr: &TritonRoot) {
    fs::create_dir_all(root.join("components")).unwrap();
    write_json_pretty_changed(root.join("triton.json"), tr).unwrap();
}

fn mk_component_dirs(root: &Path, name: &str) {
    let base = root.join("components").join(name);
    fs::create_dir_all(base.join("src")).unwrap();
    fs::create_dir_all(base.join("include")).unwrap();
    write_file(base.join("CMakeLists.txt"), "# placeholder\n# ## triton:deps begin\n# ## triton:deps end\n");
}

fn make_root(components: Vec<(&str, TritonComponent)>) -> TritonRoot {
    let mut root = TritonRoot {
        app_name: "demo".into(),
        generator: "Ninja".into(),
        cxx_std: "20".into(),
        deps: vec![DepSpec::Simple("glm".into()), DepSpec::Simple("sdl2".into())],
        components: Default::default(),
        scripts: HashMap::default(),
    };
    for (name, comp) in components {
        root.components.insert(name.into(), comp);
    }
    root
}

#[test]
#[serial]
fn remove_component_deletes_from_triton_json_and_disk() {
    let td = tempdir().unwrap();
    let root = td.path();
    let _guard = CwdGuard::set(root);

    let meta = make_root(vec![
        ("Core", TritonComponent { kind: "lib".into(), ..Default::default() }),
        ("App", TritonComponent { kind: "exe".into(), ..Default::default() }),
    ]);
    seed_project(root, &meta);
    write_minimal_resources(root);
    mk_component_dirs(root, "Core");
    mk_component_dirs(root, "App");

    handle_remove_component("Core").unwrap();

    let after: TritonRoot = read_json("triton.json").unwrap();
    assert!(!after.components.contains_key("Core"), "Core should be removed from triton.json");
    assert!(after.components.contains_key("App"), "App should still exist");
    assert!(!root.join("components/Core").exists(), "Core directory should be deleted");
    assert!(root.join("components/App").exists(), "App directory should remain");
}

#[test]
#[serial]
fn remove_component_unlinks_from_dependents() {
    let td = tempdir().unwrap();
    let root = td.path();
    let _guard = CwdGuard::set(root);

    let meta = make_root(vec![
        ("Engine", TritonComponent {
            kind: "lib".into(),
            link: vec![LinkEntry::Name("glm".into())],
            ..Default::default()
        }),
        ("Game", TritonComponent {
            kind: "exe".into(),
            link: vec![
                LinkEntry::Name("Engine".into()),
                LinkEntry::Name("sdl2".into()),
            ],
            ..Default::default()
        }),
    ]);
    seed_project(root, &meta);
    write_minimal_resources(root);
    mk_component_dirs(root, "Engine");
    mk_component_dirs(root, "Game");

    handle_remove_component("Engine").unwrap();

    let after: TritonRoot = read_json("triton.json").unwrap();
    assert!(!after.components.contains_key("Engine"));

    let game = after.components.get("Game").expect("Game should still exist");
    assert!(
        !game.link.iter().any(|e| e.normalize().0 == "Engine"),
        "Game should no longer link Engine"
    );
    assert!(
        game.link.iter().any(|e| e.normalize().0 == "sdl2"),
        "Game should still link sdl2"
    );
}

#[test]
#[serial]
fn remove_component_clears_exports_referencing_it() {
    let td = tempdir().unwrap();
    let root = td.path();
    let _guard = CwdGuard::set(root);

    let meta = make_root(vec![
        ("Core", TritonComponent {
            kind: "lib".into(),
            ..Default::default()
        }),
        ("App", TritonComponent {
            kind: "exe".into(),
            link: vec![LinkEntry::Name("Core".into())],
            exports: vec!["Core".into()],
            ..Default::default()
        }),
    ]);
    seed_project(root, &meta);
    write_minimal_resources(root);
    mk_component_dirs(root, "Core");
    mk_component_dirs(root, "App");

    handle_remove_component("Core").unwrap();

    let after: TritonRoot = read_json("triton.json").unwrap();
    let app = after.components.get("App").unwrap();
    assert!(!app.exports.contains(&"Core".into()), "Core should be removed from App's exports");
    assert!(!app.link.iter().any(|e| e.normalize().0 == "Core"), "Core should be removed from App's links");
}

#[test]
#[serial]
fn remove_nonexistent_component_returns_error() {
    let td = tempdir().unwrap();
    let root = td.path();
    let _guard = CwdGuard::set(root);

    let meta = make_root(vec![
        ("App", TritonComponent { kind: "exe".into(), ..Default::default() }),
    ]);
    seed_project(root, &meta);

    let err = handle_remove_component("Ghost").unwrap_err();
    let msg = format!("{err:#}");
    assert!(msg.contains("No such component"), "expected 'No such component' error, got: {msg}");
}

#[test]
#[serial]
fn remove_component_preserves_deps() {
    let td = tempdir().unwrap();
    let root = td.path();
    let _guard = CwdGuard::set(root);

    let meta = make_root(vec![
        ("Core", TritonComponent {
            kind: "lib".into(),
            link: vec![LinkEntry::Name("glm".into())],
            ..Default::default()
        }),
    ]);
    seed_project(root, &meta);
    write_minimal_resources(root);
    mk_component_dirs(root, "Core");

    handle_remove_component("Core").unwrap();

    let after: TritonRoot = read_json("triton.json").unwrap();
    // Deps should remain — removing a component doesn't remove global deps
    assert!(after.deps.iter().any(|d| d.name() == "glm"), "glm dep should still exist");
    assert!(after.deps.iter().any(|d| d.name() == "sdl2"), "sdl2 dep should still exist");
}
