use std::fs;
use std::path::{Path, PathBuf};
use tempfile::tempdir;

use triton::commands::build::{
    build_dir_for,
    is_configured_for_generator,
    load_presets,
    normalize_config,
    preset_for,
    resolve_generator_for_preset,
};

fn write_presets(dir: &Path, text: &str) {
    fs::create_dir_all(dir).unwrap();
    fs::write(dir.join("CMakePresets.json"), text).unwrap();
}

#[test]
fn normalize_and_preset_helpers() {
    assert_eq!(normalize_config("release"), "release");
    assert_eq!(normalize_config("REL"), "release");
    assert_eq!(normalize_config("debug"), "debug");
    assert_eq!(normalize_config("whatever"), "debug");

    assert_eq!(preset_for("release"), "release");
    assert_eq!(preset_for("rel"), "release");
    assert_eq!(preset_for("debug"), "debug");
    assert_eq!(preset_for("other"), "debug");
}

#[test]
fn build_dir_for_joins_correctly() {
    use triton::cmake::detect_vcpkg_triplet;
    use triton::cmake::arch_label_for_triplet;

    let root = PathBuf::from("/tmp/myproj");
    let triplet = detect_vcpkg_triplet();
    let arch = arch_label_for_triplet(&triplet);

    assert_eq!(
        build_dir_for(&root, "debug"),
        PathBuf::from(format!("/tmp/myproj/build/{}/debug", arch))
    );
    assert_eq!(
        build_dir_for(&root, "release"),
        PathBuf::from(format!("/tmp/myproj/build/{}/release", arch))
    );
}

#[test]
fn configured_check_detects_ninja() {
    let td = tempdir().unwrap();
    let b = td.path().join("build/debug");
    fs::create_dir_all(&b).unwrap();

    // no cache yet -> false
    assert!(!is_configured_for_generator(&b, "Ninja"));

    // cache but no build.ninja -> false
    fs::write(b.join("CMakeCache.txt"), "# fake").unwrap();
    assert!(!is_configured_for_generator(&b, "Ninja"));

    // build.ninja present -> true
    fs::write(b.join("build.ninja"), "# fake").unwrap();
    assert!(is_configured_for_generator(&b, "Ninja"));
}

#[test]
fn configured_check_detects_unix_makefiles() {
    let td = tempdir().unwrap();
    let b = td.path().join("build/release");
    fs::create_dir_all(&b).unwrap();

    // cache + Makefile -> true for Unix Makefiles
    fs::write(b.join("CMakeCache.txt"), "# fake").unwrap();
    fs::write(b.join("Makefile"), "# fake").unwrap();
    assert!(is_configured_for_generator(&b, "Unix Makefiles"));

    // wrong generator should still read as "configured", but our check only keys off file presence.
    // If you want stricter behavior, adjust is_configured_for_generator accordingly.
}

#[test]
fn load_presets_and_resolve_direct_generator() {
    let td = tempdir().unwrap();
    let comps = td.path().join("components");
    write_presets(
        &comps,
        r#"{
  "version": 6,
  "configurePresets": [
    { "name": "debug", "generator": "Ninja", "binaryDir":"${sourceDir}/../build/debug" }
  ]
}"#,
    );

    let (_v, map) = load_presets(&comps).unwrap();
    let mut guard = Vec::new();
    let g = resolve_generator_for_preset(&map, "debug", &mut guard).unwrap();
    assert_eq!(g, "Ninja");
}

#[test]
fn resolve_generator_with_inheritance_chain() {
    let td = tempdir().unwrap();
    let comps = td.path().join("components");
    write_presets(
        &comps,
        r#"{
  "version": 6,
  "configurePresets": [
    { "name": "base", "generator": "Ninja", "binaryDir":"${sourceDir}/../build/base" },
    { "name": "mid",  "inherits": "base", "binaryDir":"${sourceDir}/../build/mid" },
    { "name": "dbg",  "inherits": ["mid"], "binaryDir":"${sourceDir}/../build/debug" }
  ]
}"#,
    );

    let (_v, map) = load_presets(&comps).unwrap();
    let mut guard = Vec::new();
    let g = resolve_generator_for_preset(&map, "dbg", &mut guard).unwrap();
    assert_eq!(g, "Ninja");
}

#[test]
fn resolve_generator_handles_missing_and_cycles_gracefully() {
    let td = tempdir().unwrap();
    let comps = td.path().join("components");
    write_presets(
        &comps,
        r#"{
  "version": 6,
  "configurePresets": [
    { "name": "a", "inherits": "b" },
    { "name": "b", "inherits": "a" }
  ]
}"#,
    );

    let (_v, map) = load_presets(&comps).unwrap();
    let mut guard = Vec::new();
    // Cycle => None (our resolver bails after 32 hops). Just ensure it doesn't panic.
    assert!(resolve_generator_for_preset(&map, "a", &mut guard).is_none());
}

#[test]
fn clap_build_supports_component_flag() {
    use clap::Parser;
    use triton::cli::{Cli, Commands};

    let cli = Cli::parse_from(["triton", "build", ".", "--component", "sptconv"]);
    match cli.command {
        Commands::Build { path, component, .. } => {
            assert_eq!(path, ".");
            assert_eq!(component.as_deref(), Some("sptconv"));
        }
        other => panic!("expected build command, got {:?}", std::mem::discriminant(&other)),
    }
}
