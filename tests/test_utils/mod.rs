#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};
use std::{env, ffi::OsString};

use anyhow::Result;

/// Write a file at `path`, creating parent directories as needed.
pub fn write_file(path: impl AsRef<Path>, s: &str) {
    if let Some(parent) = path.as_ref().parent() {
        fs::create_dir_all(parent).ok();
    }
    fs::write(path, s).unwrap();
}

/// Copies the pre-cloned offline vcpkg tree from `tests/vcpkg-offline` into `<proj>/vcpkg`.
/// Panics with a clear error if the source tree doesn't exist.
pub fn copy_offline_vcpkg_to<P: AsRef<Path>>(proj: P) {
    let dest = proj.as_ref().join("vcpkg");
    let buildsystems = dest.join("scripts/buildsystems");
    let gtest_share = dest.join("installed/x64-windows/share/gtest");
    fs::create_dir_all(&buildsystems).unwrap();
    fs::create_dir_all(&gtest_share).unwrap();

    fs::write(
        buildsystems.join("vcpkg.cmake"),
        "# fake vcpkg.cmake for tests\n",
    ).unwrap();

    fs::write(
        gtest_share.join("GTestConfig.cmake"),
        "add_library(GTest::gtest INTERFACE IMPORTED)\n\
         add_library(GTest::gtest_main INTERFACE IMPORTED)\n",
    ).unwrap();
}

/// Create a no-op vcpkg stub and wire it into the environment so that
/// `handle_add` for vcpkg deps does not attempt a real install.
pub fn stub_vcpkg(bin_dir: &Path) -> PathBuf {
    fs::create_dir_all(bin_dir).unwrap();
    #[cfg(windows)]
    let path = {
        let p = bin_dir.join("vcpkg.bat");
        write_file(&p, "@echo off\r\nexit /B 0\r\n");
        p
    };
    #[cfg(not(windows))]
    let path = {
        let p = bin_dir.join("vcpkg");
        write_file(&p, "#!/usr/bin/env bash\nexit 0\n");
        let _ = std::process::Command::new("chmod")
            .args(["+x", p.to_str().unwrap()])
            .status();
        p
    };
    env::set_var("TRITON_VCPKG_EXE", &path);
    env::set_var("VCPKG_EXE", &path);

    let old = env::var_os("PATH");
    let mut newp = bin_dir.to_path_buf().into_os_string().into_string().unwrap();
    if let Some(old_s) = old.as_ref().and_then(|o| o.to_str().map(|s| s.to_string())) {
        #[cfg(windows)]
        {
            if !old_s.is_empty() {
                newp.push(';');
                newp.push_str(&old_s);
            }
        }
        #[cfg(not(windows))]
        {
            if !old_s.is_empty() {
                newp.push(':');
                newp.push_str(&old_s);
            }
        }
    }
    env::set_var("PATH", &newp);
    path
}

/// Run a closure inside a fresh temp dir, saving and restoring cwd + env vars.
pub fn with_temp_dir<F: FnOnce(&Path) -> Result<()>>(f: F) -> Result<()> {
    let t = tempfile::tempdir()?;
    let root = t.path().to_path_buf();

    let old_dir = env::current_dir()?;
    let old_path: Option<OsString> = env::var_os("PATH");
    let old_vcpkg_exe: Option<OsString> = env::var_os("VCPKG_EXE");
    let old_triton_vcpkg_exe: Option<OsString> = env::var_os("TRITON_VCPKG_EXE");

    env::set_current_dir(&root)?;
    let res = f(&root);

    // Restore environment
    match old_path {
        Some(v) => env::set_var("PATH", v),
        None => env::remove_var("PATH"),
    }
    match old_vcpkg_exe {
        Some(v) => env::set_var("VCPKG_EXE", v),
        None => env::remove_var("VCPKG_EXE"),
    }
    match old_triton_vcpkg_exe {
        Some(v) => env::set_var("TRITON_VCPKG_EXE", v),
        None => env::remove_var("TRITON_VCPKG_EXE"),
    }
    env::remove_var("TEST_OLD_PATH");
    env::set_current_dir(old_dir)?;
    res
}

/// Read triton.json from the given root directory.
pub fn read_triton(root: &Path) -> triton::models::TritonRoot {
    triton::util::read_json(root.join("triton.json")).unwrap()
}

/// Read vcpkg.json from the given root directory as a generic JSON value.
pub fn read_vcpkg(root: &Path) -> serde_json::Value {
    triton::util::read_json(root.join("vcpkg.json")).unwrap()
}

/// Write minimal template files under `<root>/resources` that the cmake generator expects.
pub fn write_minimal_resources(root: &Path) {
    let res = root.join("resources");
    fs::create_dir_all(&res).unwrap();

    fs::write(
        res.join("cmake_template.cmake"),
        r#"cmake_minimum_required(VERSION 3.25)
get_filename_component(_comp_name "${CMAKE_CURRENT_SOURCE_DIR}" NAME)

if(EXISTS "${CMAKE_CURRENT_SOURCE_DIR}/src/main.cpp")
  add_executable(${_comp_name})
  set(_is_exe ON)
else()
  add_library(${_comp_name})
  set(_is_exe OFF)
endif()

if(_is_exe)
  target_include_directories(${_comp_name} PRIVATE "include")
else()
  target_include_directories(${_comp_name} PUBLIC "include")
endif()

# ## triton:deps begin
# ## triton:deps end
"#,
    )
    .unwrap();

    fs::write(res.join("cmake_root_template.cmake"), "# (helpers stub)\n").unwrap();

    fs::write(
        res.join("cmake_presets_template.json"),
        r#"{
  "version": 6,
  "configurePresets": [
    {
      "name": "debug",
      "generator": "Ninja",
      "binaryDir": "${sourceDir}/../build/debug",
      "cacheVariables": { "CMAKE_BUILD_TYPE": "Debug" }
    }
  ],
  "buildPresets": [ { "name": "debug", "configurePreset": "debug" } ]
}"#,
    )
    .unwrap();
}
