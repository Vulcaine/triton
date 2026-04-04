//! Tests for vcpkg package name auto-detection, find-target command,
//! validation, and feature verification.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::Path;

use serial_test::serial;
use tempfile::tempdir;

use triton::commands::find_target::handle_find_target;
use triton::models::*;
use triton::util::*;

mod test_utils;
use test_utils::{write_file, CwdGuard};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Create a fake vcpkg share directory with Config.cmake files.
fn create_fake_share(root: &Path, triplet: &str, packages: &[&str]) {
    for pkg in packages {
        let dir = root
            .join("vcpkg/installed")
            .join(triplet)
            .join("share")
            .join(pkg);
        fs::create_dir_all(&dir).unwrap();
        write_file(
            dir.join(format!("{}Config.cmake", pkg)),
            &format!("# fake config for {}\n", pkg),
        );
    }
}

fn share_dir(root: &Path, triplet: &str) -> std::path::PathBuf {
    root.join("vcpkg/installed").join(triplet).join("share")
}

// ===========================================================================
// scan_vcpkg_share_for_configs
// ===========================================================================

#[test]
fn scan_finds_config_cmake_files() {
    let td = tempdir().unwrap();
    create_fake_share(td.path(), "x64-windows", &["OpenAL", "SDL2", "glm"]);

    let configs = scan_vcpkg_share_for_configs(&share_dir(td.path(), "x64-windows"));
    let names: Vec<&str> = configs.iter().map(|(n, _)| n.as_str()).collect();

    assert!(names.contains(&"OpenAL"));
    assert!(names.contains(&"SDL2"));
    assert!(names.contains(&"glm"));
}

#[test]
fn scan_ignores_dirs_without_config() {
    let td = tempdir().unwrap();
    let share = share_dir(td.path(), "x64-windows");
    // Dir with config
    create_fake_share(td.path(), "x64-windows", &["SDL2"]);
    // Dir WITHOUT config
    fs::create_dir_all(share.join("pkgconfig")).unwrap();
    write_file(share.join("pkgconfig/some.pc"), "# not a cmake config");

    let configs = scan_vcpkg_share_for_configs(&share);
    assert_eq!(configs.len(), 1);
    assert_eq!(configs[0].0, "SDL2");
}

#[test]
fn scan_returns_empty_for_missing_dir() {
    let td = tempdir().unwrap();
    let configs = scan_vcpkg_share_for_configs(&td.path().join("nonexistent"));
    assert!(configs.is_empty());
}

// ===========================================================================
// match_dep_to_packages
// ===========================================================================

#[test]
fn match_exact_case_insensitive() {
    let td = tempdir().unwrap();
    create_fake_share(td.path(), "x64-windows", &["DirectXTex", "SDL2"]);
    let all = scan_vcpkg_share_for_configs(&share_dir(td.path(), "x64-windows"));

    let matches = match_dep_to_packages("directxtex", &all);
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].0, "DirectXTex");
}

#[test]
fn match_hyphen_underscore_normalized() {
    let td = tempdir().unwrap();
    create_fake_share(td.path(), "x64-windows", &["SDL2_mixer", "SDL2"]);
    let all = scan_vcpkg_share_for_configs(&share_dir(td.path(), "x64-windows"));

    let matches = match_dep_to_packages("sdl2-mixer", &all);
    // SDL2_mixer is exact (normalized), SDL2 is substring match — exact comes first
    assert!(!matches.is_empty());
    assert_eq!(matches[0].0, "SDL2_mixer", "exact normalized match should be first");
}

#[test]
fn match_returns_multiple_when_ambiguous() {
    let td = tempdir().unwrap();
    create_fake_share(
        td.path(),
        "x64-windows",
        &["SDL2", "SDL2_mixer", "SDL2_image", "SDL2_ttf"],
    );
    let all = scan_vcpkg_share_for_configs(&share_dir(td.path(), "x64-windows"));

    let matches = match_dep_to_packages("sdl2", &all);
    // "sdl2" matches SDL2 (exact) and SDL2_mixer, SDL2_image, SDL2_ttf (substring)
    assert!(matches.len() >= 2, "should find multiple matches for 'sdl2'");
    // Exact match should be first
    assert_eq!(matches[0].0, "SDL2");
}

#[test]
fn match_returns_empty_for_unknown() {
    let td = tempdir().unwrap();
    create_fake_share(td.path(), "x64-windows", &["SDL2", "glm"]);
    let all = scan_vcpkg_share_for_configs(&share_dir(td.path(), "x64-windows"));

    let matches = match_dep_to_packages("boost", &all);
    assert!(matches.is_empty());
}

// ===========================================================================
// find-target command
// ===========================================================================

#[test]
#[serial]
fn find_target_single_match() {
    let td = tempdir().unwrap();
    let _guard = CwdGuard::set(td.path());
    create_fake_share(td.path(), &triton::cmake::detect_vcpkg_triplet(), &["OpenAL"]);

    // Should succeed without error
    let result = handle_find_target("openal-soft");
    assert!(result.is_ok());
}

#[test]
#[serial]
fn find_target_multiple_matches() {
    let td = tempdir().unwrap();
    let _guard = CwdGuard::set(td.path());
    let triplet = triton::cmake::detect_vcpkg_triplet();
    create_fake_share(td.path(), &triplet, &["SDL2", "SDL2_mixer", "SDL2_image"]);

    let result = handle_find_target("sdl2");
    assert!(result.is_ok());
}

#[test]
#[serial]
fn find_target_no_match() {
    let td = tempdir().unwrap();
    let _guard = CwdGuard::set(td.path());
    let triplet = triton::cmake::detect_vcpkg_triplet();
    create_fake_share(td.path(), &triplet, &["SDL2"]);

    let result = handle_find_target("nonexistent");
    assert!(result.is_ok()); // doesn't error, just prints "not found"
}

#[test]
#[serial]
fn find_target_no_share_dir() {
    let td = tempdir().unwrap();
    let _guard = CwdGuard::set(td.path());
    // No vcpkg dir at all

    let result = handle_find_target("anything");
    assert!(result.is_ok()); // prints guidance, doesn't crash
}

#[test]
#[serial]
fn find_target_case_insensitive() {
    let td = tempdir().unwrap();
    let _guard = CwdGuard::set(td.path());
    let triplet = triton::cmake::detect_vcpkg_triplet();
    create_fake_share(td.path(), &triplet, &["DirectXTex"]);

    let result = handle_find_target("directxtex");
    assert!(result.is_ok());
}

// ===========================================================================
// validate_triton_root
// ===========================================================================

#[test]
fn validate_accepts_valid_config() {
    let mut components = BTreeMap::new();
    components.insert(
        "App".into(),
        TritonComponent {
            kind: "exe".into(),
            link: vec![LinkEntry::Name("glm".into()), LinkEntry::Name("Core".into())],
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

    let root = TritonRoot {
        app_name: "demo".into(),
        generator: "Ninja".into(),
        cxx_std: "20".into(),
        deps: vec![DepSpec::Simple("glm".into())],
        components,
        scripts: HashMap::default(),
    };

    assert!(validate_triton_root(&root).is_ok());
}

#[test]
fn validate_rejects_empty_app_name() {
    let root = TritonRoot {
        app_name: "".into(),
        generator: "Ninja".into(),
        cxx_std: "20".into(),
        deps: vec![],
        components: Default::default(),
        scripts: HashMap::default(),
    };

    let err = validate_triton_root(&root).unwrap_err();
    assert!(
        format!("{err:#}").contains("app_name cannot be empty"),
        "got: {err:#}"
    );
}

#[test]
fn validate_rejects_invalid_kind() {
    let mut components = BTreeMap::new();
    components.insert(
        "Bad".into(),
        TritonComponent {
            kind: "shared_lib".into(),
            ..Default::default()
        },
    );

    let root = TritonRoot {
        app_name: "demo".into(),
        generator: "Ninja".into(),
        cxx_std: "20".into(),
        deps: vec![],
        components,
        scripts: HashMap::default(),
    };

    let err = validate_triton_root(&root).unwrap_err();
    let msg = format!("{err:#}");
    assert!(msg.contains("invalid kind"), "got: {msg}");
    assert!(msg.contains("shared_lib"), "got: {msg}");
}

#[test]
fn validate_rejects_self_link() {
    let mut components = BTreeMap::new();
    components.insert(
        "Core".into(),
        TritonComponent {
            kind: "lib".into(),
            link: vec![LinkEntry::Name("Core".into())],
            ..Default::default()
        },
    );

    let root = TritonRoot {
        app_name: "demo".into(),
        generator: "Ninja".into(),
        cxx_std: "20".into(),
        deps: vec![],
        components,
        scripts: HashMap::default(),
    };

    let err = validate_triton_root(&root).unwrap_err();
    assert!(
        format!("{err:#}").contains("cannot link to itself"),
        "got: {err:#}"
    );
}

#[test]
fn validate_rejects_unknown_link_target() {
    let mut components = BTreeMap::new();
    components.insert(
        "App".into(),
        TritonComponent {
            kind: "exe".into(),
            link: vec![LinkEntry::Name("ghost_dep".into())],
            ..Default::default()
        },
    );

    let root = TritonRoot {
        app_name: "demo".into(),
        generator: "Ninja".into(),
        cxx_std: "20".into(),
        deps: vec![], // ghost_dep not in deps
        components,
        scripts: HashMap::default(),
    };

    let err = validate_triton_root(&root).unwrap_err();
    let msg = format!("{err:#}");
    assert!(msg.contains("ghost_dep"), "got: {msg}");
    assert!(
        msg.contains("not a known dep or component"),
        "got: {msg}"
    );
}

#[test]
fn validate_rejects_circular_deps() {
    let mut components = BTreeMap::new();
    components.insert(
        "A".into(),
        TritonComponent {
            kind: "lib".into(),
            link: vec![LinkEntry::Name("B".into())],
            ..Default::default()
        },
    );
    components.insert(
        "B".into(),
        TritonComponent {
            kind: "lib".into(),
            link: vec![LinkEntry::Name("A".into())],
            ..Default::default()
        },
    );

    let root = TritonRoot {
        app_name: "demo".into(),
        generator: "Ninja".into(),
        cxx_std: "20".into(),
        deps: vec![],
        components,
        scripts: HashMap::default(),
    };

    let err = validate_triton_root(&root).unwrap_err();
    let msg = format!("{err:#}");
    assert!(
        msg.contains("Circular dependency"),
        "got: {msg}"
    );
}

#[test]
fn validate_allows_deps_in_links() {
    // A component linking to a dep (not a component) should be valid
    let mut components = BTreeMap::new();
    components.insert(
        "App".into(),
        TritonComponent {
            kind: "exe".into(),
            link: vec![LinkEntry::Name("glm".into())],
            ..Default::default()
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

    assert!(validate_triton_root(&root).is_ok());
}

// ===========================================================================
// detect_cycles
// ===========================================================================

fn make_root_with(deps: Vec<DepSpec>, components: BTreeMap<String, TritonComponent>) -> TritonRoot {
    TritonRoot {
        app_name: "demo".into(),
        generator: "Ninja".into(),
        cxx_std: "20".into(),
        deps,
        components,
        scripts: HashMap::default(),
    }
}

#[test]
fn detect_cycles_finds_direct_cycle() {
    let mut components = BTreeMap::new();
    components.insert(
        "A".into(),
        TritonComponent {
            kind: "lib".into(),
            link: vec![LinkEntry::Name("B".into())],
            ..Default::default()
        },
    );
    components.insert(
        "B".into(),
        TritonComponent {
            kind: "lib".into(),
            link: vec![LinkEntry::Name("A".into())],
            ..Default::default()
        },
    );

    let root = make_root_with(vec![], components);
    let cycle = detect_cycles(&root);
    assert!(cycle.is_some(), "should detect A <-> B cycle");
    let path = cycle.unwrap();
    assert!(path.len() >= 3, "cycle path should be at least [A, B, A]");
}

#[test]
fn detect_cycles_finds_indirect_cycle() {
    let mut components = BTreeMap::new();
    components.insert(
        "A".into(),
        TritonComponent {
            kind: "lib".into(),
            link: vec![LinkEntry::Name("B".into())],
            ..Default::default()
        },
    );
    components.insert(
        "B".into(),
        TritonComponent {
            kind: "lib".into(),
            link: vec![LinkEntry::Name("C".into())],
            ..Default::default()
        },
    );
    components.insert(
        "C".into(),
        TritonComponent {
            kind: "lib".into(),
            link: vec![LinkEntry::Name("A".into())],
            ..Default::default()
        },
    );

    let root = make_root_with(vec![], components);
    let cycle = detect_cycles(&root);
    assert!(cycle.is_some(), "should detect A -> B -> C -> A cycle");
}

#[test]
fn detect_cycles_no_cycle() {
    let mut components = BTreeMap::new();
    components.insert(
        "A".into(),
        TritonComponent {
            kind: "lib".into(),
            link: vec![LinkEntry::Name("B".into())],
            ..Default::default()
        },
    );
    components.insert(
        "B".into(),
        TritonComponent {
            kind: "lib".into(),
            link: vec![],
            ..Default::default()
        },
    );

    let root = make_root_with(vec![], components);
    assert!(detect_cycles(&root).is_none());
}

#[test]
fn detect_cycles_ignores_dep_links() {
    // Links to deps (not in components) should not count as cycles
    let mut components = BTreeMap::new();
    components.insert(
        "A".into(),
        TritonComponent {
            kind: "lib".into(),
            link: vec![LinkEntry::Name("glm".into())], // glm is a dep, not component
            ..Default::default()
        },
    );

    let root = make_root_with(vec![], components);
    assert!(detect_cycles(&root).is_none());
}

#[test]
fn detect_cycles_skips_when_link_target_is_also_a_dep() {
    // If directxtex is both a component and a dep, linking to it should not
    // create a cycle — CMake resolves the dep via find_package.
    let mut components = BTreeMap::new();
    components.insert(
        "directxtex".into(),
        TritonComponent {
            kind: "lib".into(),
            link: vec![LinkEntry::Name("valeria_render".into())],
            ..Default::default()
        },
    );
    components.insert(
        "valeria_render".into(),
        TritonComponent {
            kind: "lib".into(),
            link: vec![LinkEntry::Name("directxtex".into())],
            ..Default::default()
        },
    );

    // directxtex is ALSO a dep — so the cycle should be ignored
    let root = make_root_with(
        vec![DepSpec::Simple("directxtex".into())],
        components,
    );
    assert!(
        detect_cycles(&root).is_none(),
        "Should not detect cycle when link target is also a dep"
    );
}

// ===========================================================================
// link self-link rejection
// ===========================================================================

#[test]
#[serial]
fn link_rejects_self_link() {
    let td = tempdir().unwrap();
    let root_path = td.path();
    let _guard = CwdGuard::set(root_path);

    // Write minimal resources for link command
    let res = root_path.join("resources");
    fs::create_dir_all(&res).unwrap();
    fs::write(
        res.join("cmake_template.cmake"),
        "# ## triton:deps begin\n# ## triton:deps end\n",
    )
    .unwrap();
    fs::write(res.join("cmake_root_template.cmake"), "# stub\n").unwrap();
    fs::write(res.join("cmake_presets_template.json"), "{}").unwrap();

    let root = TritonRoot {
        app_name: "demo".into(),
        generator: "Ninja".into(),
        cxx_std: "20".into(),
        deps: vec![],
        components: Default::default(),
        scripts: HashMap::default(),
    };
    fs::create_dir_all(root_path.join("components")).unwrap();
    write_json_pretty_changed(root_path.join("triton.json"), &root).unwrap();

    let err = triton::handle_link("Core", "Core").unwrap_err();
    let msg = format!("{err:#}");
    assert!(
        msg.contains("cannot link to itself"),
        "self-link should be rejected, got: {msg}"
    );
}

// ===========================================================================
// generate rejects invalid config
// ===========================================================================

#[test]
#[serial]
fn generate_rejects_invalid_kind() {
    let td = tempdir().unwrap();
    let root_path = td.path();
    let _guard = CwdGuard::set(root_path);

    let mut components = BTreeMap::new();
    components.insert(
        "Bad".into(),
        TritonComponent {
            kind: "typo".into(),
            ..Default::default()
        },
    );

    let root = TritonRoot {
        app_name: "demo".into(),
        generator: "Ninja".into(),
        cxx_std: "20".into(),
        deps: vec![],
        components,
        scripts: HashMap::default(),
    };
    fs::create_dir_all(root_path.join("components")).unwrap();
    write_json_pretty_changed(root_path.join("triton.json"), &root).unwrap();

    let err = triton::commands::handle_generate().unwrap_err();
    let msg = format!("{err:#}");
    assert!(msg.contains("invalid kind"), "got: {msg}");
}

// ===========================================================================
// dep name collision: dep and component with same name
// ===========================================================================

#[test]
fn dep_and_component_same_name_is_valid() {
    // It's valid to have a dep and component with the same name.
    // The component wraps the dep — "directxtex" in the link list refers
    // to the vcpkg dep (since it's in root.deps), not a self-link.
    let mut components = BTreeMap::new();
    components.insert(
        "directxtex".into(),
        TritonComponent {
            kind: "lib".into(),
            link: vec![LinkEntry::Name("directxtex".into())], // links to vcpkg dep
            ..Default::default()
        },
    );

    let root = TritonRoot {
        app_name: "demo".into(),
        generator: "Ninja".into(),
        cxx_std: "20".into(),
        deps: vec![DepSpec::Simple("directxtex".into())],
        components,
        scripts: HashMap::default(),
    };

    // Should be valid — the name matches a dep, so it's a dep link, not self-link
    assert!(
        validate_triton_root(&root).is_ok(),
        "Component with same name as dep should be allowed to link to the dep"
    );
}

#[test]
fn self_link_still_rejected_when_not_a_dep() {
    // If a component links to itself and it's NOT also a dep, it's a real self-link
    let mut components = BTreeMap::new();
    components.insert(
        "Core".into(),
        TritonComponent {
            kind: "lib".into(),
            link: vec![LinkEntry::Name("Core".into())],
            ..Default::default()
        },
    );

    let root = TritonRoot {
        app_name: "demo".into(),
        generator: "Ninja".into(),
        cxx_std: "20".into(),
        deps: vec![], // Core is NOT a dep
        components,
        scripts: HashMap::default(),
    };

    assert!(
        validate_triton_root(&root).is_err(),
        "Pure self-link should still be rejected"
    );
}
