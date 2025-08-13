use std::fs;
use std::path::Path;
use tempfile::tempdir;

use serial_test::serial;

use triton::commands::init::handle_init;
use triton::models::TritonRoot;
use triton::util::read_json;

fn assert_exists(p: &Path) {
    assert!(p.exists(), "expected path to exist: {}", p.display());
}

#[test]
#[serial] // run this test alone to avoid set_current_dir races
fn init_minimal_mode_creates_core_files_but_no_scaffold() {
    let td = tempdir().unwrap();
    let proj = td.path().join("proj-min");
    fs::create_dir_all(&proj).unwrap();
    std::env::set_current_dir(&proj).unwrap();

    handle_init(Some("."), "x64-windows", "Unix Makefiles", "20").unwrap();

    let comps = proj.join("components");
    assert_exists(&comps);
    assert_exists(&comps.join("CMakeLists.txt"));
    assert_exists(&comps.join("CMakePresets.json"));
    assert_exists(&proj.join("triton.json"));

    let root_cm = fs::read_to_string(comps.join("CMakeLists.txt")).unwrap();
    assert!(root_cm.contains("# ## triton:components begin"));

    let presets = fs::read_to_string(comps.join("CMakePresets.json")).unwrap();
    assert!(presets.contains("Unix Makefiles"));

    assert!(!comps.join("proj-min").exists(), "minimal mode should NOT scaffold a component");

    let meta: TritonRoot = read_json(proj.join("triton.json")).unwrap();
    assert_eq!(meta.app_name, "proj-min");
    assert!(meta.components.is_empty());
    assert!(!proj.join("vcpkg.json").exists());
}

#[test]
#[serial] // run this test alone to avoid set_current_dir races
fn init_scaffold_mode_creates_app_component_and_manifest() {
    let td = tempdir().unwrap();
    let cwd = td.path();
    std::env::set_current_dir(cwd).unwrap();
    let app = "MyApp";

    // Pre-create vcpkg/ in the project dir to avoid network clone
    let proj = cwd.join(app);
    fs::create_dir_all(proj.join("vcpkg")).unwrap();

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

    let meta: TritonRoot = read_json(proj.join("triton.json")).unwrap();
    assert_eq!(meta.app_name, app);
    let comp = meta.components.get(app).expect("app component present");
    assert_eq!(comp.kind, "exe");
}
