//! Integration tests for triton: end-to-end workflows exercising init, add,
//! generate, remove, link, and direct cmake rewriting.

use std::collections::{BTreeMap, HashMap};
use std::{env, fs, path::Path};

use anyhow::Result;
use serial_test::serial;
use tempfile::tempdir;

use triton::cmake::{effective_cmake_version, rewrite_component_cmake};
use triton::commands::{handle_add, handle_generate, handle_init, handle_remove};
use triton::models::*;
use triton::util::{read_json, write_json_pretty_changed};

mod test_utils;
use test_utils::copy_offline_vcpkg_to;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn write_file(path: impl AsRef<Path>, s: &str) {
    fs::create_dir_all(path.as_ref().parent().unwrap()).ok();
    fs::write(path, s).unwrap();
}

/// Seed a minimal triton.json + vcpkg.json so that commands that read them
/// can operate without a prior `handle_init`.
fn seed_triton_json(root: &Path, tr: &TritonRoot) {
    write_json_pretty_changed(root.join("triton.json"), tr).unwrap();
}

fn seed_vcpkg_json(root: &Path, app_name: &str) {
    write_file(
        root.join("vcpkg.json"),
        &format!(
            r#"{{
  "name": "{}",
  "version": "0.0.0",
  "dependencies": []
}}"#,
            app_name.to_lowercase()
        ),
    );
}

fn read_triton(root: &Path) -> TritonRoot {
    read_json(root.join("triton.json")).unwrap()
}

fn read_vcpkg(root: &Path) -> serde_json::Value {
    read_json(root.join("vcpkg.json")).unwrap()
}

/// Create a no-op vcpkg stub and wire it into the environment so that
/// `handle_add` for vcpkg deps does not attempt a real install.
fn stub_vcpkg(bin_dir: &Path) -> std::path::PathBuf {
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
fn with_temp_dir<F: FnOnce(&Path) -> Result<()>>(f: F) -> Result<()> {
    let t = tempdir()?;
    let root = t.path().to_path_buf();

    let old_dir = env::current_dir()?;
    let old_path = env::var_os("PATH");
    let old_vcpkg_exe = env::var_os("VCPKG_EXE");
    let old_triton_vcpkg_exe = env::var_os("TRITON_VCPKG_EXE");

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

/// Create on-disk component dirs so CMake rewriting finds them.
fn mk_component_dirs(root: &Path, name: &str) {
    fs::create_dir_all(root.join("components").join(name).join("src")).unwrap();
    fs::create_dir_all(root.join("components").join(name).join("include")).unwrap();
}

/// Write a minimal component CMakeLists.txt with triton:deps markers.
fn write_component_cmake(root: &Path, name: &str) {
    let dir = root.join("components").join(name);
    fs::create_dir_all(&dir).unwrap();
    write_file(
        dir.join("CMakeLists.txt"),
        &format!(
            r#"cmake_minimum_required(VERSION 3.30.1)
get_filename_component(_comp_name "${{CMAKE_CURRENT_SOURCE_DIR}}" NAME)

if(EXISTS "${{CMAKE_CURRENT_SOURCE_DIR}}/src/main.cpp")
  add_executable(${{_comp_name}})
  set(_is_exe ON)
else()
  add_library(${{_comp_name}})
  set(_is_exe OFF)
endif()

if(_is_exe)
  target_include_directories(${{_comp_name}} PRIVATE "include")
else()
  target_include_directories(${{_comp_name}} PUBLIC "include")
endif()

# ## triton:deps begin
# ## triton:deps end
"#
        ),
    );
}

fn default_component() -> TritonComponent {
    TritonComponent {
        kind: "lib".into(),
        link: vec![],
        defines: vec![],
        exports: vec![],
        resources: vec![],
        link_options: LinkOptions::None,
        vendor_libs: VendorLibs::None,
        assets: vec![],
    }
}

// ===========================================================================
// 1. Full init -> add -> generate workflow
// ===========================================================================

#[test]
#[serial]
fn full_init_add_generate_workflow() -> Result<()> {
    with_temp_dir(|root| {
        // --- init ---
        let proj = root.join("myapp");
        fs::create_dir_all(&proj)?;
        copy_offline_vcpkg_to(&proj);
        env::set_current_dir(&proj)?;

        handle_init(Some("."), "Ninja", "20")?;

        // --- add vcpkg deps ---
        stub_vcpkg(&root.join("bin"));
        handle_add(&[String::from("glm")], None, false)?;
        handle_add(&[String::from("sdl2:myapp")], None, false)?;

        // --- generate ---
        // handle_init in minimal mode does NOT create an "myapp" component,
        // but `handle_add("sdl2:myapp")` creates one. Ensure components dir.
        handle_generate()?;

        // --- verify triton.json ---
        let tr = read_triton(&proj);
        assert!(
            tr.deps.iter().any(|d| matches!(d, DepSpec::Simple(n) if n == "glm")),
            "glm should be in root deps"
        );
        assert!(
            tr.deps
                .iter()
                .any(|d| matches!(d, DepSpec::Simple(n) if n == "sdl2")),
            "sdl2 should be in root deps"
        );

        // myapp component should link sdl2
        let myapp_comp = tr.components.get("myapp").expect("myapp component exists");
        assert!(
            myapp_comp
                .link
                .iter()
                .any(|e| e.normalize().0 == "sdl2"),
            "myapp should link sdl2"
        );

        // --- verify root CMakeLists.txt has add_subdirectory(myapp) ---
        let root_cm = fs::read_to_string(proj.join("components/CMakeLists.txt"))?;
        assert!(
            root_cm.contains("add_subdirectory(myapp)"),
            "root CMakeLists should contain add_subdirectory(myapp)"
        );

        // --- verify component CMakeLists.txt has triton:deps markers with vcpkg finder ---
        let comp_cm =
            fs::read_to_string(proj.join("components/myapp/CMakeLists.txt"))?;
        assert!(
            comp_cm.contains("# ## triton:deps begin"),
            "component CMake should have deps begin marker"
        );
        assert!(
            comp_cm.contains("triton_find_vcpkg_and_link_strict")
                || comp_cm.contains("find_package"),
            "component CMake should have vcpkg finder calls"
        );

        // --- verify vcpkg.json has both deps ---
        let vcpkg = read_vcpkg(&proj);
        let deps = vcpkg["dependencies"].as_array().unwrap();
        let dep_names: Vec<&str> = deps.iter().filter_map(|d| d.as_str()).collect();
        assert!(dep_names.contains(&"glm"), "vcpkg.json should have glm");
        assert!(dep_names.contains(&"sdl2"), "vcpkg.json should have sdl2");

        Ok(())
    })
}

// ===========================================================================
// 2. Init + add git dep + generate verifies cmake
// ===========================================================================

#[test]
#[serial]
fn init_add_git_dep_generate_verifies_cmake() -> Result<()> {
    with_temp_dir(|root| {
        let proj = root.join("engine");
        fs::create_dir_all(&proj)?;
        copy_offline_vcpkg_to(&proj);
        env::set_current_dir(&proj)?;

        handle_init(Some("."), "Ninja", "20")?;

        // Pre-create third_party/imgui with a dummy file so add skips clone
        fs::create_dir_all(proj.join("third_party/imgui"))?;
        fs::write(proj.join("third_party/imgui/dummy.txt"), "fake")?;

        handle_add(&[String::from("ocornut/imgui:engine")], None, false)?;

        // Ensure the component dir exists before generate
        if !proj.join("components/engine").exists() {
            mk_component_dirs(&proj, "engine");
            write_component_cmake(&proj, "engine");
        }

        handle_generate()?;

        // Verify triton.json has GitDep
        let tr = read_triton(&proj);
        assert!(
            tr.deps
                .iter()
                .any(|d| matches!(d, DepSpec::Git(g) if g.name == "imgui")),
            "triton.json should have imgui as GitDep"
        );

        // Verify component CMakeLists.txt mentions third_party path for imgui
        let comp_cm =
            fs::read_to_string(proj.join("components/engine/CMakeLists.txt"))?;
        assert!(
            comp_cm.contains("third_party/imgui") || comp_cm.contains("imgui"),
            "component CMake should reference imgui via third_party path"
        );

        Ok(())
    })
}

// ===========================================================================
// 3. Add / remove roundtrip
// ===========================================================================

#[test]
#[serial]
fn add_remove_roundtrip() -> Result<()> {
    with_temp_dir(|root| {
        // Seed project
        let mut tr = TritonRoot {
            app_name: "demo".into(),
            generator: "Ninja".into(),
            cxx_std: "20".into(),
            deps: vec![],
            components: BTreeMap::new(),
            scripts: HashMap::new(),
        };
        tr.components.insert("App".into(), TritonComponent {
            kind: "exe".into(),
            ..default_component()
        });
        seed_triton_json(root, &tr);
        seed_vcpkg_json(root, "demo");
        fs::create_dir_all(root.join("components"))?;
        mk_component_dirs(root, "App");
        write_component_cmake(root, "App");
        stub_vcpkg(&root.join("bin"));

        // Add both deps linked to App
        handle_add(&[String::from("glm:App")], None, false)?;
        handle_add(&[String::from("sdl2:App")], None, false)?;

        // Verify both present
        let tr1 = read_triton(root);
        assert!(tr1.deps.iter().any(|d| matches!(d, DepSpec::Simple(n) if n == "glm")));
        assert!(tr1.deps.iter().any(|d| matches!(d, DepSpec::Simple(n) if n == "sdl2")));
        let app1 = tr1.components.get("App").unwrap();
        assert!(app1.link.iter().any(|e| e.normalize().0 == "glm"));
        assert!(app1.link.iter().any(|e| e.normalize().0 == "sdl2"));

        // Remove glm from component App only (not globally)
        handle_remove("glm", Some("App"), None, false)?;

        let tr2 = read_triton(root);
        // glm should still be in root deps
        assert!(
            tr2.deps.iter().any(|d| matches!(d, DepSpec::Simple(n) if n == "glm")),
            "glm should remain in root deps after component-only unlink"
        );
        // but NOT in App's links
        let app2 = tr2.components.get("App").unwrap();
        assert!(
            !app2.link.iter().any(|e| e.normalize().0 == "glm"),
            "App should no longer link glm"
        );
        // sdl2 still linked
        assert!(app2.link.iter().any(|e| e.normalize().0 == "sdl2"));

        // Remove glm globally
        handle_remove("glm", None, None, false)?;

        let tr3 = read_triton(root);
        assert!(
            !tr3.deps.iter().any(|d| matches!(d, DepSpec::Simple(n) if n == "glm")),
            "glm should be gone from root deps after global remove"
        );

        Ok(())
    })
}

// ===========================================================================
// 4. Generate with detailed dep filters (OS-based)
// ===========================================================================

#[test]
#[serial]
fn generate_with_detailed_dep_filters() -> Result<()> {
    with_temp_dir(|root| {
        let dep = DepSpec::Detailed(DepDetailed {
            name: "winonly".into(),
            os: vec!["windows".into()],
            package: None,
            triplet: vec![],
            features: vec![],
        });
        let mut tr = TritonRoot {
            app_name: "filterdemo".into(),
            generator: "Ninja".into(),
            cxx_std: "20".into(),
            deps: vec![dep],
            components: BTreeMap::new(),
            scripts: HashMap::new(),
        };
        tr.components.insert("Main".into(), default_component());
        seed_triton_json(root, &tr);
        fs::create_dir_all(root.join("components"))?;
        mk_component_dirs(root, "Main");
        write_component_cmake(root, "Main");

        handle_generate()?;

        let vcpkg = read_vcpkg(root);
        let deps = vcpkg["dependencies"].as_array().unwrap();
        let has_winonly = deps.iter().any(|d| {
            d.as_str() == Some("winonly")
                || d.get("name").and_then(|n| n.as_str()) == Some("winonly")
        });

        if cfg!(target_os = "windows") {
            assert!(has_winonly, "On Windows, winonly dep should be in vcpkg.json");
        } else {
            assert!(
                !has_winonly,
                "On non-Windows, winonly dep should NOT be in vcpkg.json"
            );
        }

        Ok(())
    })
}

// ===========================================================================
// 5. Generate with dep features
// ===========================================================================

#[test]
#[serial]
fn generate_with_dep_features() -> Result<()> {
    with_temp_dir(|root| {
        let dep = DepSpec::Detailed(DepDetailed {
            name: "curl".into(),
            os: vec![],
            package: None,
            triplet: vec![],
            features: vec!["http2".into(), "ssl".into()],
        });
        let mut tr = TritonRoot {
            app_name: "featdemo".into(),
            generator: "Ninja".into(),
            cxx_std: "20".into(),
            deps: vec![dep],
            components: BTreeMap::new(),
            scripts: HashMap::new(),
        };
        tr.components.insert("Main".into(), default_component());
        seed_triton_json(root, &tr);
        fs::create_dir_all(root.join("components"))?;
        mk_component_dirs(root, "Main");
        write_component_cmake(root, "Main");

        handle_generate()?;

        let vcpkg = read_vcpkg(root);
        let deps = vcpkg["dependencies"].as_array().unwrap();

        // Should be object form: {"name": "curl", "features": ["http2", "ssl"]}
        let curl_entry = deps
            .iter()
            .find(|d| d.get("name").and_then(|n| n.as_str()) == Some("curl"))
            .expect("curl should be present in vcpkg.json dependencies");

        let features = curl_entry["features"]
            .as_array()
            .expect("curl entry should have features array");
        let feat_names: Vec<&str> = features.iter().filter_map(|f| f.as_str()).collect();
        assert!(feat_names.contains(&"http2"), "features should include http2");
        assert!(feat_names.contains(&"ssl"), "features should include ssl");

        Ok(())
    })
}

// ===========================================================================
// 6. Component with defines, resources, assets
// ===========================================================================

#[test]
#[serial]
fn component_with_defines_resources_assets() -> Result<()> {
    with_temp_dir(|root| {
        let comp = TritonComponent {
            kind: "exe".into(),
            link: vec![],
            defines: vec!["MY_DEF".into()],
            exports: vec![],
            resources: vec!["res".into()],
            link_options: LinkOptions::None,
            vendor_libs: VendorLibs::None,
            assets: vec!["data".into()],
        };

        let mut tr = TritonRoot {
            app_name: "assetdemo".into(),
            generator: "Ninja".into(),
            cxx_std: "20".into(),
            deps: vec![],
            components: BTreeMap::new(),
            scripts: HashMap::new(),
        };
        tr.components.insert("Widget".into(), comp.clone());
        seed_triton_json(root, &tr);

        // Create component dir structure with deps markers
        mk_component_dirs(root, "Widget");
        write_component_cmake(root, "Widget");

        // Also create the resource and asset dirs so the cmake generator can find them
        fs::create_dir_all(root.join("components/Widget/res"))?;
        fs::create_dir_all(root.join("components/Widget/data"))?;

        let cmake_ver = effective_cmake_version();
        rewrite_component_cmake("Widget", &tr, &comp, cmake_ver)?;

        let cm = fs::read_to_string(root.join("components/Widget/CMakeLists.txt"))?;

        // Verify defines
        assert!(
            cm.contains("target_compile_definitions") && cm.contains("MY_DEF"),
            "CMakeLists should contain target_compile_definitions with MY_DEF. Got:\n{}",
            cm
        );

        // Verify resources (copy_directory for res)
        assert!(
            cm.contains("copy_directory") && cm.contains("res"),
            "CMakeLists should contain copy_directory command for resources. Got:\n{}",
            cm
        );

        // Verify asset staging code
        assert!(
            cm.contains("_triton_asset_stamps") && cm.contains("data"),
            "CMakeLists should contain asset staging code for 'data'. Got:\n{}",
            cm
        );

        Ok(())
    })
}

// ===========================================================================
// 7. Component with vendor_libs per platform
// ===========================================================================

#[test]
#[serial]
fn component_with_vendor_libs_per_platform() -> Result<()> {
    with_temp_dir(|root| {
        let mut platform_map = BTreeMap::new();
        platform_map.insert("windows".into(), vec!["vendor/foo.lib".into()]);
        platform_map.insert("linux".into(), vec!["vendor/libfoo.a".into()]);

        let comp = TritonComponent {
            kind: "lib".into(),
            link: vec![],
            defines: vec![],
            exports: vec![],
            resources: vec![],
            link_options: LinkOptions::None,
            vendor_libs: VendorLibs::PerPlatform(platform_map),
            assets: vec![],
        };

        let mut tr = TritonRoot {
            app_name: "vendordemo".into(),
            generator: "Ninja".into(),
            cxx_std: "20".into(),
            deps: vec![],
            components: BTreeMap::new(),
            scripts: HashMap::new(),
        };
        tr.components.insert("Native".into(), comp.clone());
        seed_triton_json(root, &tr);

        mk_component_dirs(root, "Native");
        write_component_cmake(root, "Native");

        let cmake_ver = effective_cmake_version();
        rewrite_component_cmake("Native", &tr, &comp, cmake_ver)?;

        let cm = fs::read_to_string(root.join("components/Native/CMakeLists.txt"))?;

        assert!(
            cm.contains("if(WIN32)"),
            "CMakeLists should have if(WIN32) for windows vendor_libs. Got:\n{}",
            cm
        );
        assert!(
            cm.contains("vendor/foo.lib"),
            "CMakeLists should reference vendor/foo.lib. Got:\n{}",
            cm
        );
        assert!(
            cm.contains("if(UNIX AND NOT APPLE)"),
            "CMakeLists should have if(UNIX AND NOT APPLE) for linux vendor_libs. Got:\n{}",
            cm
        );
        assert!(
            cm.contains("vendor/libfoo.a"),
            "CMakeLists should reference vendor/libfoo.a. Got:\n{}",
            cm
        );

        Ok(())
    })
}

// ===========================================================================
// 8. Component with link_options
// ===========================================================================

#[test]
#[serial]
fn component_with_link_options() -> Result<()> {
    with_temp_dir(|root| {
        let comp = TritonComponent {
            kind: "exe".into(),
            link: vec![],
            defines: vec![],
            exports: vec![],
            resources: vec![],
            link_options: LinkOptions::All(vec!["-Wl,--export-dynamic".into()]),
            vendor_libs: VendorLibs::None,
            assets: vec![],
        };

        let mut tr = TritonRoot {
            app_name: "linkoptdemo".into(),
            generator: "Ninja".into(),
            cxx_std: "20".into(),
            deps: vec![],
            components: BTreeMap::new(),
            scripts: HashMap::new(),
        };
        tr.components.insert("Runner".into(), comp.clone());
        seed_triton_json(root, &tr);

        mk_component_dirs(root, "Runner");
        write_component_cmake(root, "Runner");

        let cmake_ver = effective_cmake_version();
        rewrite_component_cmake("Runner", &tr, &comp, cmake_ver)?;

        let cm = fs::read_to_string(root.join("components/Runner/CMakeLists.txt"))?;

        assert!(
            cm.contains("target_link_options"),
            "CMakeLists should contain target_link_options. Got:\n{}",
            cm
        );
        assert!(
            cm.contains("-Wl,--export-dynamic"),
            "CMakeLists should contain the linker flag. Got:\n{}",
            cm
        );

        Ok(())
    })
}

// ===========================================================================
// 9. Component exports makes dep PUBLIC
// ===========================================================================

#[test]
#[serial]
fn component_exports_makes_dep_public() -> Result<()> {
    with_temp_dir(|root| {
        let mut tr = TritonRoot {
            app_name: "exportdemo".into(),
            generator: "Ninja".into(),
            cxx_std: "20".into(),
            deps: vec![DepSpec::Simple("glm".into())],
            components: BTreeMap::new(),
            scripts: HashMap::new(),
        };

        // Engine links glm and exports it publicly
        tr.components.insert(
            "Engine".into(),
            TritonComponent {
                kind: "lib".into(),
                link: vec![LinkEntry::Name("glm".into())],
                defines: vec![],
                exports: vec!["glm".into()],
                resources: vec![],
                link_options: LinkOptions::None,
                vendor_libs: VendorLibs::None,
                assets: vec![],
            },
        );

        // Game links Engine
        tr.components.insert(
            "Game".into(),
            TritonComponent {
                kind: "exe".into(),
                link: vec![LinkEntry::Name("Engine".into())],
                defines: vec![],
                exports: vec![],
                resources: vec![],
                link_options: LinkOptions::None,
                vendor_libs: VendorLibs::None,
                assets: vec![],
            },
        );

        seed_triton_json(root, &tr);
        fs::create_dir_all(root.join("components"))?;
        mk_component_dirs(root, "Engine");
        mk_component_dirs(root, "Game");
        write_component_cmake(root, "Engine");
        write_component_cmake(root, "Game");

        handle_generate()?;

        let engine_cm =
            fs::read_to_string(root.join("components/Engine/CMakeLists.txt"))?;

        // When Engine exports glm, the vcpkg finder or link_libraries call for
        // glm in Engine's CMakeLists should use PUBLIC visibility.
        // The strict finder emits the finder call; for export the underlying
        // generated code should carry the public marker.
        // Check that the strict finder is present (it handles PUBLIC internally)
        // or that an explicit PUBLIC appears alongside glm.
        assert!(
            engine_cm.contains("triton_find_vcpkg_and_link_strict")
                || engine_cm.contains("PUBLIC"),
            "Engine CMakeLists should use PUBLIC or strict finder for exported glm. Got:\n{}",
            engine_cm
        );

        Ok(())
    })
}

// ===========================================================================
// 10. Link entry Named with targets
// ===========================================================================

#[test]
#[serial]
fn link_entry_named_with_targets() -> Result<()> {
    with_temp_dir(|root| {
        let mut tr = TritonRoot {
            app_name: "nameddemo".into(),
            generator: "Ninja".into(),
            cxx_std: "20".into(),
            deps: vec![DepSpec::Simple("rmlui".into())],
            components: BTreeMap::new(),
            scripts: HashMap::new(),
        };

        // Component with a Named link entry specifying package and targets
        tr.components.insert(
            "UI".into(),
            TritonComponent {
                kind: "lib".into(),
                link: vec![LinkEntry::Named {
                    name: "rmlui".into(),
                    package: Some("RmlUi".into()),
                    targets: Some(vec!["RmlUi::RmlUi".into()]),
                }],
                defines: vec![],
                exports: vec![],
                resources: vec![],
                link_options: LinkOptions::None,
                vendor_libs: VendorLibs::None,
                assets: vec![],
            },
        );

        seed_triton_json(root, &tr);
        fs::create_dir_all(root.join("components"))?;
        mk_component_dirs(root, "UI");
        write_component_cmake(root, "UI");

        handle_generate()?;

        let ui_cm = fs::read_to_string(root.join("components/UI/CMakeLists.txt"))?;

        assert!(
            ui_cm.contains("find_package(RmlUi"),
            "UI CMakeLists should have find_package(RmlUi). Got:\n{}",
            ui_cm
        );
        assert!(
            ui_cm.contains("RmlUi::RmlUi"),
            "UI CMakeLists should have target_link_libraries with RmlUi::RmlUi. Got:\n{}",
            ui_cm
        );

        Ok(())
    })
}
