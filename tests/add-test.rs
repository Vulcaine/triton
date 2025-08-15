// tests/add-test.rs
use std::{env, fs, path::Path};
use anyhow::Result;
use serial_test::serial;

use triton::{
    commands::handle_add,
    models::TritonRoot,
    util::read_json,
};

fn write(path: impl AsRef<Path>, s: &str) {
    fs::create_dir_all(path.as_ref().parent().unwrap()).ok();
    fs::write(path, s).unwrap();
}

fn init_min_triton_json(root: &Path) {
    write(
        root.join("triton.json"),
        r#"{
  "app_name": "demo",
  "triplet": "x64-windows",
  "generator": "Ninja",
  "cxx_std": "20",
  "deps": [],
  "components": {}
}
"#,
    );
}

fn init_empty_vcpkg_manifest(root: &Path) {
    write(
        root.join("vcpkg.json"),
        r#"{
  "name": "demo",
  "version": "0.0.0",
  "dependencies": []
}
"#,
    );
}

fn read_triton(root: &Path) -> TritonRoot {
    read_json(root.join("triton.json")).unwrap()
}

fn read_vcpkg(root: &Path) -> serde_json::Value {
    read_json(root.join("vcpkg.json")).unwrap()
}

fn prepend_path(dir: &Path) -> (Option<std::ffi::OsString>, String) {
    let old = env::var_os("PATH");
    let mut newp = dir.to_path_buf().into_os_string().into_string().unwrap();
    if let Some(old_s) = old.as_ref().and_then(|o| o.to_str().map(|s| s.to_string())) {
        #[cfg(windows)]
        {
            if !old_s.is_empty() { newp.push(';'); newp.push_str(&old_s); }
        }
        #[cfg(not(windows))]
        {
            if !old_s.is_empty() { newp.push(':'); newp.push_str(&old_s); }
        }
    }
    (old, newp)
}

/// Create a no-op `vcpkg` in `bin_dir` and put it at the front of PATH.
/// Also sets TRITON_VCPKG_EXE and VCPKG_EXE to the exact path.
fn stub_vcpkg(bin_dir: &Path) -> std::path::PathBuf {
    fs::create_dir_all(bin_dir).unwrap();
    #[cfg(windows)]
    let path = {
        let p = bin_dir.join("vcpkg.bat");
        write(&p, "@echo off\r\nexit /B 0\r\n");
        p
    };
    #[cfg(not(windows))]
    let path = {
        let p = bin_dir.join("vcpkg");
        write(&p, "#!/usr/bin/env bash\nexit 0\n");
        let _ = std::process::Command::new("chmod")
            .args(["+x", p.to_str().unwrap()])
            .status();
        p
    };

    env::set_var("TRITON_VCPKG_EXE", &path);
    env::set_var("VCPKG_EXE", &path);
    let (old_path, newp) = prepend_path(bin_dir);
    env::set_var("PATH", &newp);
    if let Some(old) = old_path { env::set_var("TEST_OLD_PATH", old); }
    path
}

fn with_temp_dir<F: FnOnce(&Path) -> Result<()>>(f: F) -> Result<()> {
    let t = tempfile::tempdir()?;
    let root = t.path().to_path_buf();

    // Save / switch cwd
    let old_dir = env::current_dir()?;
    let old_path = env::var_os("PATH");
    let old_vcpkg_exe = env::var_os("VCPKG_EXE");
    let old_triton_vcpkg_exe = env::var_os("TRITON_VCPKG_EXE");

    env::set_current_dir(&root)?;
    let res = f(&root);

    // Restore env + cwd
    match old_path { Some(v) => env::set_var("PATH", v), None => env::remove_var("PATH") }
    match old_vcpkg_exe { Some(v) => env::set_var("VCPKG_EXE", v), None => env::remove_var("VCPKG_EXE") }
    match old_triton_vcpkg_exe { Some(v) => env::set_var("TRITON_VCPKG_EXE", v), None => env::remove_var("TRITON_VCPKG_EXE") }
    env::remove_var("TEST_OLD_PATH");

    env::set_current_dir(old_dir)?;
    res
}

#[test]
#[serial]
fn add_vcpkg_dep_updates_triton_and_manifest_and_calls_stub_vcpkg() -> Result<()> {
    with_temp_dir(|root| {
        init_min_triton_json(root);
        init_empty_vcpkg_manifest(root);

        // Stub vcpkg to avoid a real install.
        let bin_dir = root.join("bin");
        stub_vcpkg(&bin_dir);

        // Add a new vcpkg dep
        handle_add(&[String::from("glm")], None, false)?;

        // triton.json contains "glm"
        let t = read_triton(root);
        assert!(t.deps.iter().any(|d| matches!(d, triton::models::RootDep::Name(n) if n == "glm")));

        // vcpkg.json contains "glm"
        let mani = read_vcpkg(root);
        let deps = mani["dependencies"].as_array().unwrap();
        assert!(deps.iter().any(|v| v == "glm"));

        Ok(())
    })
}

#[test]
#[serial]
fn add_vcpkg_dep_with_component_scaffolds_and_links() -> Result<()> {
    with_temp_dir(|root| {
        init_min_triton_json(root);
        init_empty_vcpkg_manifest(root);
        stub_vcpkg(&root.join("bin"));

        // Add + link: "sdl2:Game"
        handle_add(&[String::from("sdl2:Game")], None, false)?;

        // component directories exist
        assert!(root.join("components/Game/src").exists());
        assert!(root.join("components/Game/include").exists());
        assert!(root.join("components/Game/CMakeLists.txt").exists());

        // triton.json has component + link entry
        let t = read_triton(root);
        let game = t.components.get("Game").expect("Game component missing");
        assert!(game.link.iter().any(|e| matches!(e, triton::models::LinkEntry::Name(n) if n == "sdl2")));

        // vcpkg.json contains sdl2
        let mani = read_vcpkg(root);
        let deps = mani["dependencies"].as_array().unwrap();
        assert!(deps.iter().any(|v| v == "sdl2"));

        Ok(())
    })
}

#[test]
#[serial]
fn add_git_dep_records_and_links_without_clone_when_already_present() -> Result<()> {
    with_temp_dir(|root| {
        init_min_triton_json(root);

        // Pre-create third_party folder so `git clone` is skipped.
        let third = root.join("third_party/filament");
        fs::create_dir_all(&third)?;

        // Add git dep + link to UI
        handle_add(&[String::from("google/filament@main:UI")], None, false)?;

        let t = read_triton(root);
        // dep recorded as Git
        assert!(t.deps.iter().any(|d| matches!(d,
            triton::models::RootDep::Git(g) if g.name == "filament" && g.repo == "google/filament"
        )));

        // component link present
        let ui = t.components.get("UI").expect("UI component missing");
        assert!(ui.link.iter().any(|e| matches!(e, triton::models::LinkEntry::Name(n) if n == "filament")));

        // component scaffolded
        assert!(root.join("components/UI/src").exists());
        assert!(root.join("components/UI/include").exists());
        assert!(root.join("components/UI/CMakeLists.txt").exists());

        Ok(())
    })
}

#[test]
#[serial]
fn add_vcpkg_dep_is_idempotent() -> Result<()> {
    with_temp_dir(|root| {
        init_min_triton_json(root);
        init_empty_vcpkg_manifest(root);
        stub_vcpkg(&root.join("bin"));

        handle_add(&[String::from("entt")], None, false)?;
        // second add shouldn't duplicate
        handle_add(&[String::from("entt")], None, false)?;

        let t = read_triton(root);
        let count = t.deps.iter().filter(|d| matches!(d, triton::models::RootDep::Name(n) if n == "entt")).count();
        assert_eq!(count, 1, "dep duplicated in triton.json");

        let mani = read_vcpkg(root);
        let deps = mani["dependencies"].as_array().unwrap();
        let count2 = deps.iter().filter(|v| *v == "entt").count();
        assert_eq!(count2, 1, "dep duplicated in vcpkg.json");

        Ok(())
    })
}
