use std::fs;
use std::path::{Path, PathBuf};
use tempfile::tempdir;

use serial_test::serial;

use triton::commands::init::handle_init;
use triton::models::TritonRoot;
use triton::util::read_json;

use fs_extra::dir::{copy as copy_dir, CopyOptions};

fn assert_exists(p: &Path) {
    assert!(p.exists(), "expected path to exist: {}", p.display());
}

/// Copies the pre-cloned offline vcpkg tree from `tests/vcpkg-offline` into `<proj>/vcpkg`.
fn copy_offline_vcpkg_to<P: AsRef<Path>>(proj: P) {
    let offline: PathBuf = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("vcpkg-offline");
    let dest = proj.as_ref().join("vcpkg"); // ensure we place it under vcpkg/
    let mut opts = CopyOptions::new();
    opts.overwrite = true;
    opts.copy_inside = true;
    fs::create_dir_all(&dest).unwrap();
    copy_dir(&offline, &dest, &opts)
        .unwrap_or_else(|e| panic!("Failed to copy offline vcpkg: {e}"));
}

#[test]
#[serial]
fn init_minimal_mode_creates_core_files_and_tests_but_no_app_scaffold() {
    let td = tempdir().unwrap();
    let proj = td.path().join("proj-min");
    fs::create_dir_all(&proj).unwrap();
    copy_offline_vcpkg_to(&proj);
    std::env::set_current_dir(&proj).unwrap();

    handle_init(Some("."), "x64-windows", "Unix Makefiles", "20").unwrap();

    let comps = proj.join("components");
    assert_exists(&comps);
    assert_exists(&comps.join("CMakeLists.txt"));
    assert_exists(&comps.join("CMakePresets.json"));
    assert_exists(&proj.join("triton.json"));
    assert_exists(&proj.join("vcpkg.json"));

    let root_cm = fs::read_to_string(comps.join("CMakeLists.txt")).unwrap();
    assert!(root_cm.contains("# ## triton:components begin"));

    let presets = fs::read_to_string(comps.join("CMakePresets.json")).unwrap();
    assert!(presets.contains("Unix Makefiles"));

    assert!(
        !comps.join("proj-min").exists(),
        "minimal mode should NOT scaffold a main app component"
    );

    assert_exists(&comps.join("tests"));
    assert_exists(&comps.join("tests/src/test_main.cpp"));
    assert_exists(&comps.join("tests/CMakeLists.txt"));

    let meta: TritonRoot = read_json(proj.join("triton.json")).unwrap();
    assert_eq!(meta.app_name, "proj-min");

    let tcomp = meta.components.get("tests").expect("tests component present");
    assert_eq!(tcomp.kind, "exe");

    let meta_str = serde_json::to_string(&meta).unwrap();
    assert!(meta_str.contains("\"tests\""));
    assert!(meta_str.to_lowercase().contains("gtest"));

    let vcpkg: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(proj.join("vcpkg.json")).unwrap()).unwrap();
    let deps = vcpkg["dependencies"].as_array().cloned().unwrap_or_default();
    let gtest_count = deps
        .iter()
        .filter(|d| d.is_string() && d.as_str().unwrap().eq_ignore_ascii_case("gtest"))
        .count();
    assert_eq!(gtest_count, 1);
}

#[test]
#[serial]
fn init_scaffold_mode_creates_app_component_manifest_and_tests() {
    let td = tempdir().unwrap();
    let cwd = td.path();
    std::env::set_current_dir(cwd).unwrap();
    let app = "MyApp";

    let proj = cwd.join(app);
    fs::create_dir_all(&proj).unwrap();
    copy_offline_vcpkg_to(&proj);

    handle_init(Some(app), "x64-windows", "Unix Makefiles", "20").unwrap();

    let comps = proj.join("components");

    assert_exists(&proj.join("triton.json"));
    assert_exists(&proj.join("vcpkg.json"));
    assert_exists(&comps.join("CMakeLists.txt"));
    assert_exists(&comps.join("CMakePresets.json"));

    let app_dir = comps.join(app);
    assert_exists(&app_dir.join("src/main.cpp"));
    assert_exists(&app_dir.join("include"));
    assert_exists(&app_dir.join("CMakeLists.txt"));

    let main_cpp = fs::read_to_string(app_dir.join("src/main.cpp")).unwrap();
    assert!(main_cpp.contains("Hello from triton app!"));

    let app_cm = fs::read_to_string(app_dir.join("CMakeLists.txt")).unwrap();
    assert!(app_cm.contains("# ## triton:deps begin"));

    assert_exists(&comps.join("tests/src/test_main.cpp"));
    assert_exists(&comps.join("tests/CMakeLists.txt"));

    let meta: TritonRoot = read_json(proj.join("triton.json")).unwrap();
    assert_eq!(meta.app_name, app);
    let comp = meta.components.get(app).expect("app component present");
    assert_eq!(comp.kind, "exe");
    assert!(meta.components.contains_key("tests"));

    let vcpkg: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(proj.join("vcpkg.json")).unwrap()).unwrap();
    let deps = vcpkg["dependencies"].as_array().cloned().unwrap_or_default();
    let gtest_count = deps
        .iter()
        .filter(|d| d.is_string() && d.as_str().unwrap().eq_ignore_ascii_case("gtest"))
        .count();
    assert_eq!(gtest_count, 1);
}

#[test]
#[serial]
fn init_is_idempotent_and_adds_missing_without_overwriting() {
    let td = tempdir().unwrap();
    let proj = td.path().join("proj-idem");
    std::env::set_current_dir(&td).unwrap();

    copy_offline_vcpkg_to(&proj);

    handle_init(Some("proj-idem"), "x64-windows", "Ninja", "20").unwrap();

    let comps = proj.join("components");
    let tests_cpp = comps.join("tests/src/test_main.cpp");
    assert_exists(&tests_cpp);
    assert_exists(&proj.join("triton.json"));
    assert_exists(&proj.join("vcpkg.json"));

    fs::write(&tests_cpp, "// KEEP ME\n").unwrap();

    let vcpkg_path = proj.join("vcpkg.json");
    let orig_vcpkg = fs::read_to_string(&vcpkg_path).unwrap();
    let mut v: serde_json::Value = serde_json::from_str(&orig_vcpkg).unwrap();
    if let Some(arr) = v["dependencies"].as_array_mut() {
        assert!(arr.iter().any(|d| d == "gtest"));
    }
    fs::write(&vcpkg_path, serde_json::to_string_pretty(&v).unwrap()).unwrap();

    std::env::set_current_dir(&proj).unwrap();
    handle_init(Some("."), "x64-windows", "Ninja", "20").unwrap();

    let after = fs::read_to_string(&tests_cpp).unwrap();
    assert_eq!(after, "// KEEP ME\n");

    let vcpkg_after: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&vcpkg_path).unwrap()).unwrap();
    let deps = vcpkg_after["dependencies"].as_array().cloned().unwrap_or_default();
    let gtest_count = deps
        .iter()
        .filter(|d| d.is_string() && d.as_str().unwrap().eq_ignore_ascii_case("gtest"))
        .count();
    assert_eq!(gtest_count, 1);

    let meta: TritonRoot = read_json(proj.join("triton.json")).unwrap();
    let t = meta.components.get("tests").expect("tests component present");
    assert_eq!(t.kind, "exe");
}
