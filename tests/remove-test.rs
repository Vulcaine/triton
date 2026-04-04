use std::collections::HashMap;
// tests/remove-test.rs
use std::fs;
use std::path::Path;
use tempfile::tempdir;

use serial_test::serial;

use triton::commands::remove::handle_remove;
use triton::models::{LinkEntry, DepSpec, TritonComponent, TritonRoot};
use triton::util::{read_json, write_json_pretty_changed};

mod test_utils;
use test_utils::CwdGuard;

fn assert_exists(p: &Path) {
    assert!(p.exists(), "expected to exist: {}", p.display());
}

fn assert_not_exists(p: &Path) {
    assert!(!p.exists(), "expected NOT to exist: {}", p.display());
}

/// Minimal project seed: components/ exists and `triton.json` is written.
fn seed_project(root: &Path, tr: &TritonRoot) {
    fs::create_dir_all(root.join("components")).unwrap();
    write_json_pretty_changed(root.join("triton.json"), tr).unwrap();
}

/// Make on-disk dirs for a component so CMake files can be written.
fn mk_component_dirs(root: &Path, name: &str) {
    fs::create_dir_all(root.join("components").join(name).join("src")).unwrap();
    fs::create_dir_all(root.join("components").join(name).join("include")).unwrap();
}

/* -------------------------------------------------------------------------- */
/*                              tests start here                               */
/* -------------------------------------------------------------------------- */

#[test]
#[serial]
fn remove_unlinks_only_from_target_component_when_component_opt_is_used() {
    let td = tempdir().unwrap();
    let root = td.path();
    let _guard = CwdGuard::set(root);

    // Triton metadata: two vcpkg deps, two components A and B, both link "glm"
    let mut meta = TritonRoot {
        app_name: "demo".into(),
        
        generator: "Ninja".into(),
        cxx_std: "20".into(),
        deps: vec![DepSpec::Simple("glm".into()), DepSpec::Simple("sdl2".into())],
        components: Default::default(),
        scripts: HashMap::default(),
    };
    meta.components.insert(
        "A".into(),
        TritonComponent { kind: "lib".into(), link: vec![LinkEntry::Name("glm".into()), LinkEntry::Name("sdl2".into())], ..Default::default() },
    );
    meta.components.insert(
        "B".into(),
        TritonComponent { kind: "lib".into(), link: vec![LinkEntry::Name("glm".into())], ..Default::default() },
    );

    seed_project(root, &meta);
    mk_component_dirs(root, "A");
    mk_component_dirs(root, "B");

    // Act: unlink glm only from component A
    handle_remove("glm", Some("A"), None, false).unwrap();

    // Assert: deps list unchanged; only A lost glm
    let after: TritonRoot = read_json("triton.json").unwrap();
    assert!(
        after.deps.iter().any(|d| matches!(d, DepSpec::Simple(n) if n == "glm")),
        "glm must remain in root.deps when unlinking only from one component"
    );
    let a = after.components.get("A").unwrap();
    assert!(
        !a.link.iter().any(|e| e.normalize().0 == "glm"),
        "A should no longer link glm"
    );
    assert!(
        a.link.iter().any(|e| e.normalize().0 == "sdl2"),
        "A should still link sdl2"
    );
    let b = after.components.get("B").unwrap();
    assert!(
        b.link.iter().any(|e| e.normalize().0 == "glm"),
        "B should still link glm"
    );

    // Root & component CMakeLists exist (rewritten)
    assert_exists(&root.join("components/CMakeLists.txt"));
    assert_exists(&root.join("components/A/CMakeLists.txt"));
    assert_exists(&root.join("components/B/CMakeLists.txt"));
}

#[test]
#[serial]
fn remove_vcpkg_dep_globally_updates_manifest_and_unlinks_everywhere() {
    let td = tempdir().unwrap();
    let root = td.path();
    let _guard = CwdGuard::set(root);

    // Root has glm + sdl2; two components link both
    let mut meta = TritonRoot {
        app_name: "demo".into(),
        
        generator: "Ninja".into(),
        cxx_std: "20".into(),
        deps: vec![DepSpec::Simple("glm".into()), DepSpec::Simple("sdl2".into())],
        components: Default::default(),
        scripts: HashMap::default(),
    };
    meta.components.insert(
        "Core".into(),
        TritonComponent { kind: "lib".into(), link: vec![LinkEntry::Name("glm".into()), LinkEntry::Name("sdl2".into())], ..Default::default() },
    );
    meta.components.insert(
        "App".into(),
        TritonComponent { kind: "exe".into(), link: vec![LinkEntry::Name("glm".into()), LinkEntry::Name("sdl2".into())], ..Default::default() },
    );

    seed_project(root, &meta);
    mk_component_dirs(root, "Core");
    mk_component_dirs(root, "App");

    // Act: remove glm project-wide
    handle_remove("glm", None, None, false).unwrap();

    // Assert: glm gone from deps and from all components; sdl2 remains
    let after: TritonRoot = read_json("triton.json").unwrap();
    assert!(
        !matches!(after.deps.iter().find(|d| matches!(d, DepSpec::Simple(n) if n == "glm")), Some(_)),
        "glm should be removed from root.deps"
    );
    for (name, comp) in &after.components {
        assert!(
            !comp.link.iter().any(|e| e.normalize().0 == "glm"),
            "component {} must no longer link glm",
            name
        );
        assert!(
            comp.link.iter().any(|e| e.normalize().0 == "sdl2"),
            "component {} should still link sdl2",
            name
        );
    }

    // vcpkg.json contains only remaining vcpkg deps (sdl2)
    let mani_text = fs::read_to_string("vcpkg.json").unwrap();
    let v: serde_json::Value = serde_json::from_str(&mani_text).unwrap();
    let deps = v.get("dependencies").and_then(|x| x.as_array()).unwrap();
    let names: Vec<&str> = deps.iter().filter_map(|d| d.as_str()).collect();
    assert_eq!(names, vec!["sdl2"]);
}

#[test]
#[serial]
fn remove_git_dep_globally_unlinks_everywhere_and_prunes_third_party_if_unused() {
    let td = tempdir().unwrap();
    let root = td.path();
    let _guard = CwdGuard::set(root);

    // One git dep "filament" + a vcpkg dep to ensure vcpkg.json still valid
    let git = DepSpec::Git(triton::models::GitDep {
        repo: "google/filament".into(),
        name: "filament".into(),
        branch: None,
        cmake: vec![],
    });

    let mut meta = TritonRoot {
        app_name: "demo".into(),
        
        generator: "Ninja".into(),
        cxx_std: "20".into(),
        deps: vec![git, DepSpec::Simple("sdl2".into())],
        components: Default::default(),
        scripts: HashMap::default(),
    };
    meta.components.insert(
        "Engine".into(),
        TritonComponent { kind: "lib".into(), link: vec![LinkEntry::Name("filament".into())], ..Default::default() },
    );

    seed_project(root, &meta);
    mk_component_dirs(root, "Engine");

    // create a fake vendored dir that should be pruned if unused
    let vendored = root.join("third_party/filament");
    fs::create_dir_all(&vendored).unwrap();
    assert_exists(&vendored);

    // Act: remove git dep globally
    handle_remove("filament", None, None, false).unwrap();

    // Assert: dep removed and component unlinked
    let after: TritonRoot = read_json("triton.json").unwrap();
    assert!(
        after
            .deps
            .iter()
            .find(|d| matches!(d, DepSpec::Git(g) if g.name == "filament"))
            .is_none(),
        "git dep 'filament' should be removed from root.deps"
    );
    let eng = after.components.get("Engine").unwrap();
    assert!(
        !eng.link.iter().any(|e| e.normalize().0 == "filament"),
        "Engine should no longer link 'filament'"
    );

    // vendored dir pruned
    assert_not_exists(&vendored);

    // vcpkg.json still lists sdl2 only
    let mani_text = fs::read_to_string("vcpkg.json").unwrap();
    let v: serde_json::Value = serde_json::from_str(&mani_text).unwrap();
    let deps = v.get("dependencies").and_then(|x| x.as_array()).unwrap();
    let names: Vec<&str> = deps.iter().filter_map(|d| d.as_str()).collect();
    assert_eq!(names, vec!["sdl2"]);
}

#[test]
#[serial]
fn remove_from_missing_component_returns_error() {
    let td = tempdir().unwrap();
    let root = td.path();
    let _guard = CwdGuard::set(root);

    let meta = TritonRoot {
        app_name: "demo".into(),
        
        generator: "Ninja".into(),
        cxx_std: "20".into(),
        deps: vec![DepSpec::Simple("glm".into())],
        components: Default::default(),
        scripts: HashMap::default(),
    };
    seed_project(root, &meta);

    let err = handle_remove("glm", Some("NoSuchComponent"), None, false).unwrap_err();
    let msg = format!("{err:#}");
    assert!(
        msg.contains("No such component"),
        "expected 'No such component' error, got: {msg}"
    );
}

#[test]
#[serial]
fn remove_twice_is_idempotent() {
    let td = tempdir().unwrap();
    let root = td.path();
    let _guard = CwdGuard::set(root);

    let mut meta = TritonRoot {
        app_name: "demo".into(),
        generator: "Ninja".into(),
        cxx_std: "20".into(),
        deps: vec![DepSpec::Simple("glm".into()), DepSpec::Simple("sdl2".into())],
        components: Default::default(),
        scripts: HashMap::default(),
    };
    meta.components.insert(
        "App".into(),
        TritonComponent {
            kind: "exe".into(),
            link: vec![LinkEntry::Name("glm".into()), LinkEntry::Name("sdl2".into())],
            ..Default::default()
        },
    );

    seed_project(root, &meta);
    mk_component_dirs(root, "App");

    // Remove glm twice — second call should be a no-op, not an error
    handle_remove("glm", None, None, false).unwrap();
    handle_remove("glm", None, None, false).unwrap();

    let after: TritonRoot = read_json("triton.json").unwrap();
    assert!(!after.deps.iter().any(|d| d.name() == "glm"), "glm should be gone");
    assert!(after.deps.iter().any(|d| d.name() == "sdl2"), "sdl2 should remain");

    let app = after.components.get("App").unwrap();
    assert!(!app.link.iter().any(|e| e.normalize().0 == "glm"), "glm link should be gone");
    assert!(app.link.iter().any(|e| e.normalize().0 == "sdl2"), "sdl2 link should remain");
}

#[test]
#[serial]
fn remove_preserves_features_in_vcpkg_json() {
    let td = tempdir().unwrap();
    let root = td.path();
    let _guard = CwdGuard::set(root);

    let mut meta = TritonRoot {
        app_name: "demo".into(),
        generator: "Ninja".into(),
        cxx_std: "20".into(),
        deps: vec![
            DepSpec::Simple("glm".into()),
            DepSpec::Detailed(triton::models::DepDetailed {
                name: "directxtex".into(),
                features: vec!["dx12".into()],
                package: Some("directxtex".into()),
                ..Default::default()
            }),
        ],
        components: Default::default(),
        scripts: HashMap::default(),
    };
    meta.components.insert("App".into(), TritonComponent {
        kind: "exe".into(),
        link: vec![LinkEntry::Name("glm".into()), LinkEntry::Name("directxtex".into())],
        ..Default::default()
    });

    seed_project(root, &meta);
    mk_component_dirs(root, "App");

    // Remove glm, directxtex should keep its features in vcpkg.json
    handle_remove("glm", None, None, false).unwrap();

    let vcpkg_raw = fs::read_to_string("vcpkg.json").unwrap();
    let vcpkg: serde_json::Value = serde_json::from_str(&vcpkg_raw).unwrap();
    let deps = vcpkg["dependencies"].as_array().unwrap();

    // directxtex should be object with features
    let dtex = deps.iter().find(|v| {
        v.get("name").and_then(|n| n.as_str()) == Some("directxtex")
    });
    assert!(dtex.is_some(), "directxtex should remain in vcpkg.json as object");
    let feats = dtex.unwrap()["features"].as_array().unwrap();
    assert!(feats.iter().any(|f| f.as_str() == Some("dx12")),
        "dx12 feature should be preserved in vcpkg.json");

    // glm should be gone
    assert!(!deps.iter().any(|v| v.as_str() == Some("glm")),
        "glm should be removed from vcpkg.json");
}
