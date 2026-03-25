use std::collections::{BTreeMap, HashMap};
use std::fs;
use tempfile::tempdir;
use serial_test::serial;

use triton::models::*;
use triton::util::*;

// ---------------------------------------------------------------------------
// Helper: build a full TritonRoot with all dep types
// ---------------------------------------------------------------------------
fn sample_root() -> TritonRoot {
    let mut components = BTreeMap::new();
    components.insert(
        "Engine".into(),
        TritonComponent {
            kind: "lib".into(),
            link: vec![
                LinkEntry::Name("glm".into()),
                LinkEntry::Named {
                    name: "sdl2".into(),
                    package: Some("SDL2".into()),
                    targets: Some(vec!["SDL2::SDL2".into()]),
                },
            ],
            defines: vec!["GLM_ENABLE_EXPERIMENTAL".into()],
            exports: vec!["glm".into()],
            resources: vec!["resources".into()],
            link_options: LinkOptions::All(vec!["-Wl,--export-dynamic".into()]),
            vendor_libs: VendorLibs::None,
            assets: vec!["shaders".into()],
        },
    );

    TritonRoot {
        app_name: "demo".into(),
        generator: "Ninja".into(),
        cxx_std: "20".into(),
        deps: vec![
            DepSpec::Simple("glm".into()),
            DepSpec::Git(GitDep {
                repo: "https://github.com/example/imgui.git".into(),
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
            }),
            DepSpec::Detailed(DepDetailed {
                name: "sdl2".into(),
                os: vec!["windows".into(), "linux".into()],
                package: Some("SDL2".into()),
                triplet: vec!["x64-windows".into()],
                features: vec!["vulkan".into()],
            }),
        ],
        components,
        scripts: {
            let mut m = HashMap::new();
            m.insert("build".into(), "cargo build".into());
            m
        },
    }
}

// ===========================================================================
// 1. TritonRoot round-trip with all dep types
// ===========================================================================
#[test]
fn triton_root_round_trip() {
    let root = sample_root();
    let json = serde_json::to_string(&root).unwrap();
    let back: TritonRoot = serde_json::from_str(&json).unwrap();

    assert_eq!(back.app_name, "demo");
    assert_eq!(back.generator, "Ninja");
    assert_eq!(back.cxx_std, "20");
    assert_eq!(back.deps.len(), 3);
    assert!(back.components.contains_key("Engine"));
    assert_eq!(back.scripts.get("build").map(String::as_str), Some("cargo build"));
}

// ===========================================================================
// 2. LinkEntry deserialization: Name, Named, Map
// ===========================================================================
#[test]
fn link_entry_name_deserialize() {
    let json = r#""glm""#;
    let entry: LinkEntry = serde_json::from_str(json).unwrap();
    match &entry {
        LinkEntry::Name(n) => assert_eq!(n, "glm"),
        other => panic!("expected Name, got {:?}", other),
    }
}

#[test]
fn link_entry_named_deserialize() {
    let json = r#"{"name":"rmlui","package":"RmlUi","targets":["RmlUi::RmlUi"]}"#;
    let entry: LinkEntry = serde_json::from_str(json).unwrap();
    match &entry {
        LinkEntry::Named { name, package, targets } => {
            assert_eq!(name, "rmlui");
            assert_eq!(package.as_deref(), Some("RmlUi"));
            assert_eq!(targets.as_ref().unwrap(), &["RmlUi::RmlUi"]);
        }
        other => panic!("expected Named, got {:?}", other),
    }
}

#[test]
fn link_entry_named_minimal_deserialize() {
    let json = r#"{"name":"lua"}"#;
    let entry: LinkEntry = serde_json::from_str(json).unwrap();
    match &entry {
        LinkEntry::Named { name, package, targets } => {
            assert_eq!(name, "lua");
            assert!(package.is_none());
            assert!(targets.is_none());
        }
        other => panic!("expected Named, got {:?}", other),
    }
}

#[test]
fn link_entry_map_deserialize() {
    let json = r#"{"filament":{"package":"Filament","targets":["filament","utils","math"]}}"#;
    let entry: LinkEntry = serde_json::from_str(json).unwrap();
    match &entry {
        LinkEntry::Map(map) => {
            let hint = map.get("filament").unwrap();
            assert_eq!(hint.package.as_deref(), Some("Filament"));
            assert_eq!(
                hint.targets.as_ref().unwrap(),
                &["filament", "utils", "math"]
            );
        }
        other => panic!("expected Map, got {:?}", other),
    }
}

// ===========================================================================
// 3. LinkOptions deserialization: None, All, PerPlatform
// ===========================================================================
#[test]
fn link_options_none_default() {
    // When the field is absent, serde(default) gives None
    let json = r#"{"kind":"lib"}"#;
    let comp: TritonComponent = serde_json::from_str(json).unwrap();
    match comp.link_options {
        LinkOptions::None => {}
        other => panic!("expected LinkOptions::None, got {:?}", other),
    }
}

#[test]
fn link_options_all_deserialize() {
    let json = r#"{"kind":"lib","link_options":["-Wl,--export-dynamic"]}"#;
    let comp: TritonComponent = serde_json::from_str(json).unwrap();
    match &comp.link_options {
        LinkOptions::All(v) => assert_eq!(v, &["-Wl,--export-dynamic"]),
        other => panic!("expected All, got {:?}", other),
    }
}

#[test]
fn link_options_per_platform_deserialize() {
    let json = r#"{"kind":"lib","link_options":{"linux":["-Wl,--export-dynamic"],"windows":[]}}"#;
    let comp: TritonComponent = serde_json::from_str(json).unwrap();
    match &comp.link_options {
        LinkOptions::PerPlatform(map) => {
            assert_eq!(map.get("linux").unwrap(), &["-Wl,--export-dynamic"]);
            assert!(map.get("windows").unwrap().is_empty());
        }
        other => panic!("expected PerPlatform, got {:?}", other),
    }
}

// ===========================================================================
// 4. VendorLibs deserialization: None, All, PerPlatform
// ===========================================================================
#[test]
fn vendor_libs_none_default() {
    let json = r#"{"kind":"lib"}"#;
    let comp: TritonComponent = serde_json::from_str(json).unwrap();
    match comp.vendor_libs {
        VendorLibs::None => {}
        other => panic!("expected VendorLibs::None, got {:?}", other),
    }
}

#[test]
fn vendor_libs_all_deserialize() {
    let json = r#"{"kind":"lib","vendor_libs":["vendor/dotnet/libnethost.a"]}"#;
    let comp: TritonComponent = serde_json::from_str(json).unwrap();
    match &comp.vendor_libs {
        VendorLibs::All(v) => assert_eq!(v, &["vendor/dotnet/libnethost.a"]),
        other => panic!("expected All, got {:?}", other),
    }
}

#[test]
fn vendor_libs_per_platform_deserialize() {
    let json = r#"{"kind":"lib","vendor_libs":{"linux":["vendor/libnethost.a"],"windows":["vendor/nethost.lib"]}}"#;
    let comp: TritonComponent = serde_json::from_str(json).unwrap();
    match &comp.vendor_libs {
        VendorLibs::PerPlatform(map) => {
            assert_eq!(map.get("linux").unwrap(), &["vendor/libnethost.a"]);
            assert_eq!(map.get("windows").unwrap(), &["vendor/nethost.lib"]);
        }
        other => panic!("expected PerPlatform, got {:?}", other),
    }
}

// ===========================================================================
// 5. CMakeOverride deserialization: Entry and KV
// ===========================================================================
#[test]
fn cmake_override_kv_deserialize() {
    let json = r#""IMGUI_BUILD_EXAMPLES=OFF""#;
    let ov: CMakeOverride = serde_json::from_str(json).unwrap();
    match &ov {
        CMakeOverride::KV(s) => assert_eq!(s, "IMGUI_BUILD_EXAMPLES=OFF"),
        other => panic!("expected KV, got {:?}", other),
    }
}

#[test]
fn cmake_override_entry_deserialize() {
    let json = r#"{"var":"FREETYPE","val":"ON","typ":"BOOL"}"#;
    let ov: CMakeOverride = serde_json::from_str(json).unwrap();
    match &ov {
        CMakeOverride::Entry(e) => {
            assert_eq!(e.var, "FREETYPE");
            assert_eq!(e.val, "ON");
            assert_eq!(e.typ, "BOOL");
        }
        other => panic!("expected Entry, got {:?}", other),
    }
}

#[test]
fn cmake_cache_entry_default_type() {
    let json = r#"{"var":"FOO","val":"bar"}"#;
    let entry: CMakeCacheEntry = serde_json::from_str(json).unwrap();
    assert_eq!(entry.typ, "STRING", "typ should default to STRING");
}

// ===========================================================================
// 6. TritonComponent full round-trip
// ===========================================================================
#[test]
fn triton_component_full_round_trip() {
    let mut map = BTreeMap::new();
    map.insert(
        "filament".into(),
        LinkHint {
            package: Some("Filament".into()),
            targets: Some(vec!["filament".into(), "utils".into()]),
        },
    );

    let comp = TritonComponent {
        kind: "exe".into(),
        link: vec![
            LinkEntry::Name("glm".into()),
            LinkEntry::Named {
                name: "sdl2".into(),
                package: Some("SDL2".into()),
                targets: Some(vec!["SDL2::SDL2".into()]),
            },
            LinkEntry::Map(map),
        ],
        defines: vec!["MY_DEF".into()],
        exports: vec!["glm".into()],
        resources: vec!["resources".into()],
        link_options: LinkOptions::PerPlatform({
            let mut m = BTreeMap::new();
            m.insert("linux".into(), vec!["-Wl,--export-dynamic".into()]);
            m.insert("windows".into(), vec![]);
            m
        }),
        vendor_libs: VendorLibs::All(vec!["vendor/lib.a".into()]),
        assets: vec!["shaders".into(), "textures".into()],
    };

    let json = serde_json::to_string(&comp).unwrap();
    let back: TritonComponent = serde_json::from_str(&json).unwrap();

    assert_eq!(back.kind, "exe");
    assert_eq!(back.link.len(), 3);
    assert_eq!(back.defines, vec!["MY_DEF"]);
    assert_eq!(back.exports, vec!["glm"]);
    assert_eq!(back.resources, vec!["resources"]);
    assert_eq!(back.assets, vec!["shaders", "textures"]);

    match &back.link_options {
        LinkOptions::PerPlatform(m) => {
            assert!(m.contains_key("linux"));
            assert!(m.contains_key("windows"));
        }
        other => panic!("expected PerPlatform, got {:?}", other),
    }
    match &back.vendor_libs {
        VendorLibs::All(v) => assert_eq!(v, &["vendor/lib.a"]),
        other => panic!("expected All, got {:?}", other),
    }
}

// ===========================================================================
// 7. Default/empty fields via serde(default)
// ===========================================================================
#[test]
fn triton_component_defaults() {
    let json = r#"{"kind":"lib"}"#;
    let comp: TritonComponent = serde_json::from_str(json).unwrap();
    assert_eq!(comp.kind, "lib");
    assert!(comp.link.is_empty());
    assert!(comp.defines.is_empty());
    assert!(comp.exports.is_empty());
    assert!(comp.resources.is_empty());
    assert!(comp.assets.is_empty());
}

#[test]
fn triton_root_scripts_default() {
    let json = r#"{
        "app_name":"x","generator":"Ninja","cxx_std":"20",
        "deps":[],"components":{}
    }"#;
    let root: TritonRoot = serde_json::from_str(json).unwrap();
    assert!(root.scripts.is_empty(), "scripts should default to empty");
}

// ===========================================================================
// 8. DepDetailed with os and triplet filters
// ===========================================================================
#[test]
fn dep_detailed_with_os_and_triplet() {
    let json = r#"{"name":"sdl2","os":["windows","linux"],"package":"SDL2","triplet":["x64-windows"],"features":["vulkan"]}"#;
    let dep: DepDetailed = serde_json::from_str(json).unwrap();
    assert_eq!(dep.name, "sdl2");
    assert_eq!(dep.os, vec!["windows", "linux"]);
    assert_eq!(dep.package.as_deref(), Some("SDL2"));
    assert_eq!(dep.triplet, vec!["x64-windows"]);
    assert_eq!(dep.features, vec!["vulkan"]);
}

#[test]
fn dep_detailed_minimal() {
    let json = r#"{"name":"lua"}"#;
    let dep: DepDetailed = serde_json::from_str(json).unwrap();
    assert_eq!(dep.name, "lua");
    assert!(dep.os.is_empty());
    assert!(dep.package.is_none());
    assert!(dep.triplet.is_empty());
    assert!(dep.features.is_empty());
}

#[test]
fn dep_spec_round_trip_all_variants() {
    let specs = vec![
        DepSpec::Simple("glm".into()),
        DepSpec::Git(GitDep {
            repo: "https://example.com/repo.git".into(),
            name: "mylib".into(),
            branch: Some("main".into()),
            cmake: vec![CMakeOverride::KV("OPT=ON".into())],
        }),
        DepSpec::Detailed(DepDetailed {
            name: "sdl2".into(),
            os: vec!["linux".into()],
            package: None,
            triplet: vec![],
            features: vec!["vulkan".into()],
        }),
    ];

    let json = serde_json::to_string(&specs).unwrap();
    let back: Vec<DepSpec> = serde_json::from_str(&json).unwrap();
    assert_eq!(back.len(), 3);

    match &back[0] {
        DepSpec::Simple(n) => assert_eq!(n, "glm"),
        other => panic!("expected Simple, got {:?}", other),
    }
    match &back[1] {
        DepSpec::Git(g) => {
            assert_eq!(g.name, "mylib");
            assert_eq!(g.branch.as_deref(), Some("main"));
            assert_eq!(g.cmake.len(), 1);
        }
        other => panic!("expected Git, got {:?}", other),
    }
    match &back[2] {
        DepSpec::Detailed(d) => {
            assert_eq!(d.name, "sdl2");
            assert_eq!(d.features, vec!["vulkan"]);
        }
        other => panic!("expected Detailed, got {:?}", other),
    }
}

// ===========================================================================
// 9. LinkEntry::normalize()
// ===========================================================================
#[test]
fn normalize_name_variant() {
    let entry = LinkEntry::Name("glm".into());
    let (name, pkg) = entry.normalize();
    assert_eq!(name, "glm");
    assert!(pkg.is_none());
}

#[test]
fn normalize_named_variant() {
    let entry = LinkEntry::Named {
        name: "sdl2".into(),
        package: Some("SDL2".into()),
        targets: Some(vec!["SDL2::SDL2".into()]),
    };
    let (name, pkg) = entry.normalize();
    assert_eq!(name, "sdl2");
    assert_eq!(pkg.as_deref(), Some("SDL2"));
}

#[test]
fn normalize_named_no_package() {
    let entry = LinkEntry::Named {
        name: "lua".into(),
        package: None,
        targets: None,
    };
    let (name, pkg) = entry.normalize();
    assert_eq!(name, "lua");
    assert!(pkg.is_none());
}

#[test]
fn normalize_map_variant() {
    let mut map = BTreeMap::new();
    map.insert(
        "filament".into(),
        LinkHint {
            package: Some("Filament".into()),
            targets: None,
        },
    );
    let entry = LinkEntry::Map(map);
    let (name, pkg) = entry.normalize();
    assert_eq!(name, "filament");
    assert_eq!(pkg.as_deref(), Some("Filament"));
}

#[test]
fn normalize_empty_map() {
    let entry = LinkEntry::Map(BTreeMap::new());
    let (name, pkg) = entry.normalize();
    assert_eq!(name, "");
    assert!(pkg.is_none());
}

// ===========================================================================
// 10. LinkEntry::all_targets()
// ===========================================================================
#[test]
fn all_targets_name_variant() {
    let entry = LinkEntry::Name("glm".into());
    assert!(entry.all_targets().is_empty());
}

#[test]
fn all_targets_named_with_targets() {
    let entry = LinkEntry::Named {
        name: "sdl2".into(),
        package: None,
        targets: Some(vec!["SDL2::SDL2".into(), "SDL2::SDL2main".into()]),
    };
    assert_eq!(entry.all_targets(), vec!["SDL2::SDL2", "SDL2::SDL2main"]);
}

#[test]
fn all_targets_named_no_targets() {
    let entry = LinkEntry::Named {
        name: "lua".into(),
        package: None,
        targets: None,
    };
    assert!(entry.all_targets().is_empty());
}

#[test]
fn all_targets_map_variant() {
    let mut map = BTreeMap::new();
    map.insert(
        "filament".into(),
        LinkHint {
            package: None,
            targets: Some(vec!["filament".into(), "utils".into(), "math".into()]),
        },
    );
    let entry = LinkEntry::Map(map);
    assert_eq!(entry.all_targets(), vec!["filament", "utils", "math"]);
}

#[test]
fn all_targets_empty_map() {
    let entry = LinkEntry::Map(BTreeMap::new());
    assert!(entry.all_targets().is_empty());
}

// ===========================================================================
// 11. cmake_quote
// ===========================================================================
#[test]
fn cmake_quote_normal() {
    assert_eq!(cmake_quote("hello"), r#""hello""#);
}

#[test]
fn cmake_quote_with_inner_quotes() {
    assert_eq!(cmake_quote(r#"say "hi""#), r#""say \"hi\"""#);
}

#[test]
fn cmake_quote_empty() {
    assert_eq!(cmake_quote(""), r#""""#);
}

#[test]
fn cmake_quote_trims_whitespace() {
    assert_eq!(cmake_quote("  foo  "), r#""foo""#);
}

// ===========================================================================
// 12. infer_cmake_type
// ===========================================================================
#[test]
fn infer_cmake_type_bool_values() {
    for v in &["ON", "OFF", "TRUE", "FALSE", "YES", "NO"] {
        assert_eq!(infer_cmake_type(v), "BOOL", "expected BOOL for {v}");
    }
}

#[test]
fn infer_cmake_type_case_insensitive() {
    assert_eq!(infer_cmake_type("on"), "BOOL");
    assert_eq!(infer_cmake_type("Off"), "BOOL");
    assert_eq!(infer_cmake_type("true"), "BOOL");
    assert_eq!(infer_cmake_type("False"), "BOOL");
    assert_eq!(infer_cmake_type("yes"), "BOOL");
    assert_eq!(infer_cmake_type("No"), "BOOL");
}

#[test]
fn infer_cmake_type_string_values() {
    assert_eq!(infer_cmake_type("Release"), "STRING");
    assert_eq!(infer_cmake_type("/usr/local"), "STRING");
    assert_eq!(infer_cmake_type("42"), "STRING");
    assert_eq!(infer_cmake_type(""), "STRING");
}

// ===========================================================================
// 13. split_kv
// ===========================================================================
#[test]
fn split_kv_simple() {
    let (k, v) = split_kv("FOO=bar");
    assert_eq!(k, "FOO");
    assert_eq!(v, "bar");
}

#[test]
fn split_kv_strips_outer_quotes() {
    let (k, v) = split_kv(r#"FOO="bar""#);
    assert_eq!(k, "FOO");
    assert_eq!(v, "bar");
}

#[test]
fn split_kv_bare_key() {
    let (k, v) = split_kv("FOO");
    assert_eq!(k, "FOO");
    assert_eq!(v, "ON");
}

#[test]
fn split_kv_empty_value() {
    let (k, v) = split_kv("FOO=");
    assert_eq!(k, "FOO");
    assert_eq!(v, "ON", "empty value defaults to ON");
}

#[test]
fn split_kv_value_with_equals() {
    let (k, v) = split_kv("PATH=/usr/local/bin");
    assert_eq!(k, "PATH");
    assert_eq!(v, "/usr/local/bin");
}

#[test]
fn split_kv_whitespace_trimmed() {
    let (k, v) = split_kv("  FOO  =  bar  ");
    assert_eq!(k, "FOO");
    assert_eq!(v, "bar");
}

// ===========================================================================
// 14. normalize_path
// ===========================================================================
#[test]
fn normalize_path_strips_verbatim_prefix_backslash() {
    let result = normalize_path(r"\\?\C:\Users\test");
    assert!(!result.contains(r"\\?\"), "verbatim prefix should be stripped");
    assert!(result.contains("C:") || result.contains("Users"));
}

#[test]
fn normalize_path_strips_verbatim_prefix_forward() {
    let result = normalize_path("//?/C:/Users/test");
    assert!(!result.contains("//?/"), "verbatim prefix should be stripped");
}

#[test]
fn normalize_path_plain_path() {
    let result = normalize_path("some/path/to/file");
    // On Windows: backslashes; on Unix: forward slashes
    if cfg!(windows) {
        assert!(result.contains('\\'), "Windows should use backslashes: {result}");
        assert!(!result.contains('/'), "Windows should not have forward slashes: {result}");
    } else {
        assert!(result.contains('/'), "Unix should use forward slashes: {result}");
    }
}

// ===========================================================================
// 15. is_dep
// ===========================================================================
#[test]
fn is_dep_finds_simple() {
    let root = sample_root();
    assert!(is_dep(&root, "glm"));
}

#[test]
fn is_dep_finds_git() {
    let root = sample_root();
    assert!(is_dep(&root, "imgui"));
}

#[test]
fn is_dep_finds_detailed() {
    let root = sample_root();
    assert!(is_dep(&root, "sdl2"));
}

#[test]
fn is_dep_returns_false_for_unknown() {
    let root = sample_root();
    assert!(!is_dep(&root, "nonexistent"));
}

// ===========================================================================
// 16. has_link_to_name
// ===========================================================================
#[test]
fn has_link_to_name_finds_name_variant() {
    let comp = TritonComponent {
        kind: "lib".into(),
        link: vec![LinkEntry::Name("glm".into())],
        ..Default::default()
    };
    assert!(has_link_to_name(&comp, "glm"));
    assert!(!has_link_to_name(&comp, "sdl2"));
}

#[test]
fn has_link_to_name_finds_named_variant() {
    let comp = TritonComponent {
        kind: "exe".into(),
        link: vec![LinkEntry::Named {
            name: "sdl2".into(),
            package: Some("SDL2".into()),
            targets: None,
        }],
        ..Default::default()
    };
    assert!(has_link_to_name(&comp, "sdl2"));
    assert!(!has_link_to_name(&comp, "SDL2"), "should match name, not package");
}

#[test]
fn has_link_to_name_finds_map_variant() {
    let mut map = BTreeMap::new();
    map.insert("filament".into(), LinkHint::default());
    let comp = TritonComponent {
        kind: "lib".into(),
        link: vec![LinkEntry::Map(map)],
        ..Default::default()
    };
    assert!(has_link_to_name(&comp, "filament"));
    assert!(!has_link_to_name(&comp, "other"));
}

#[test]
fn has_link_to_name_empty_link() {
    let comp = TritonComponent {
        kind: "lib".into(),
        ..Default::default()
    };
    assert!(!has_link_to_name(&comp, "anything"));
}

// ===========================================================================
// 17. write_text_if_changed
// ===========================================================================
#[test]
#[serial]
fn write_text_if_changed_creates_new_file() {
    let td = tempdir().unwrap();
    let file = td.path().join("new.txt");
    let result = write_text_if_changed(&file, "hello").unwrap();
    assert!(matches!(result, Change::Created));
    assert_eq!(fs::read_to_string(&file).unwrap(), "hello");
}

#[test]
#[serial]
fn write_text_if_changed_modifies_existing() {
    let td = tempdir().unwrap();
    let file = td.path().join("exist.txt");
    fs::write(&file, "old").unwrap();
    let result = write_text_if_changed(&file, "new").unwrap();
    assert!(matches!(result, Change::Modified));
    assert_eq!(fs::read_to_string(&file).unwrap(), "new");
}

#[test]
#[serial]
fn write_text_if_changed_unchanged() {
    let td = tempdir().unwrap();
    let file = td.path().join("same.txt");
    fs::write(&file, "content").unwrap();
    let result = write_text_if_changed(&file, "content").unwrap();
    assert!(matches!(result, Change::Unchanged));
}

#[test]
#[serial]
fn write_text_if_changed_creates_parent_dirs() {
    let td = tempdir().unwrap();
    let file = td.path().join("a").join("b").join("c.txt");
    let result = write_text_if_changed(&file, "deep").unwrap();
    assert!(matches!(result, Change::Created));
    assert_eq!(fs::read_to_string(&file).unwrap(), "deep");
}

// ===========================================================================
// 18. read_json
// ===========================================================================
#[test]
#[serial]
fn read_json_valid() {
    let td = tempdir().unwrap();
    let file = td.path().join("data.json");
    let root = sample_root();
    fs::write(&file, serde_json::to_string_pretty(&root).unwrap()).unwrap();
    let back: TritonRoot = read_json(&file).unwrap();
    assert_eq!(back.app_name, "demo");
    assert_eq!(back.deps.len(), 3);
}

#[test]
#[serial]
fn read_json_invalid_returns_error() {
    let td = tempdir().unwrap();
    let file = td.path().join("bad.json");
    fs::write(&file, "not valid json {{{").unwrap();
    let result: Result<TritonRoot, _> = read_json(&file);
    assert!(result.is_err());
}

#[test]
#[serial]
fn read_json_missing_file_returns_error() {
    let td = tempdir().unwrap();
    let file = td.path().join("nonexistent.json");
    let result: Result<TritonRoot, _> = read_json(&file);
    assert!(result.is_err());
}

// ===========================================================================
// 19. ensure_component_scaffold
// ===========================================================================
#[test]
#[serial]
fn ensure_component_scaffold_creates_structure() {
    let td = tempdir().unwrap();
    std::env::set_current_dir(td.path()).unwrap();

    ensure_component_scaffold("MyComp").unwrap();

    let base = td.path().join("components").join("MyComp");
    assert!(base.exists(), "base dir should exist");
    assert!(base.join("src").join("MyComp").exists(), "src/MyComp dir should exist");
    assert!(base.join("include").join("MyComp").exists(), "include/MyComp dir should exist");

    let header = base.join("include").join("MyComp").join("MyComp.hpp");
    assert!(header.exists(), "header placeholder should exist");
    let header_content = fs::read_to_string(&header).unwrap();
    assert!(header_content.contains("#pragma once"), "header should have pragma once");

    let source = base.join("src").join("MyComp").join("MyComp.cpp");
    assert!(source.exists(), "source placeholder should exist");
    let source_content = fs::read_to_string(&source).unwrap();
    assert!(
        source_content.contains("#include <MyComp/MyComp.hpp>"),
        "source should include the header"
    );
}

#[test]
#[serial]
fn ensure_component_scaffold_idempotent() {
    let td = tempdir().unwrap();
    std::env::set_current_dir(td.path()).unwrap();

    ensure_component_scaffold("Foo").unwrap();
    let header = td.path().join("components/Foo/include/Foo/Foo.hpp");
    let content_before = fs::read_to_string(&header).unwrap();

    // Call again -- should not fail or overwrite existing files
    ensure_component_scaffold("Foo").unwrap();
    let content_after = fs::read_to_string(&header).unwrap();
    assert_eq!(content_before, content_after, "scaffold should not overwrite existing files");
}
