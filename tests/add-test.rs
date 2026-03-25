use std::{fs, path::Path};
use anyhow::Result;
use serial_test::serial;

use triton::{
    commands::handle_add,
    models::{TritonRoot, DepSpec, LinkEntry, TritonComponent},
    util::{read_json, write_json_pretty_changed},
};

mod test_utils;
use test_utils::{write_file, stub_vcpkg, with_temp_dir, read_triton, read_vcpkg};

fn init_min_triton_json(root: &Path) {
    write_file(
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
    write_file(
        root.join("vcpkg.json"),
        r#"{
  "name": "demo",
  "version": "0.0.0",
  "dependencies": []
}
"#,
    );
}

#[test]
#[serial]
fn add_vcpkg_dep_updates_triton_and_manifest_and_calls_stub_vcpkg() -> Result<()> {
    with_temp_dir(|root| {
        init_min_triton_json(root);
        init_empty_vcpkg_manifest(root);

        let bin_dir = root.join("bin");
        stub_vcpkg(&bin_dir);

        handle_add(&[String::from("glm")], None, false)?;

        let t = read_triton(root);
        assert!(t.deps.iter().any(|d| matches!(d, DepSpec::Simple(n) if n == "glm")));

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

        handle_add(&[String::from("sdl2:Game")], None, false)?;

        assert!(root.join("components/Game/src").exists());
        assert!(root.join("components/Game/include").exists());
        assert!(root.join("components/Game/CMakeLists.txt").exists());

        let t = read_triton(root);
        let game = t.components.get("Game").unwrap();
        assert!(game.link.iter().any(|e| matches!(e, LinkEntry::Name(n) if n == "sdl2")));

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
        // Pre-create with a file so clone is skipped (empty dirs get removed)
        fs::create_dir_all(root.join("third_party/filament"))?;
        fs::write(root.join("third_party/filament/.gitkeep"), "")?;

        handle_add(&[String::from("google/filament@main:UI")], None, false)?;

        let t = read_triton(root);
        assert!(t.deps.iter().any(|d| matches!(d,
            DepSpec::Git(g) if g.name == "filament" && g.repo == "google/filament"
        )));

        let ui = t.components.get("UI").unwrap();
        assert!(ui.link.iter().any(|e| matches!(e, LinkEntry::Name(n) if n == "filament")));

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
        handle_add(&[String::from("entt")], None, false)?;

        let t = read_triton(root);
        let count = t.deps.iter().filter(|d| matches!(d, DepSpec::Simple(n) if n == "entt")).count();
        assert_eq!(count, 1);

        let mani = read_vcpkg(root);
        let deps = mani["dependencies"].as_array().unwrap();
        let count2 = deps.iter().filter(|v| *v == "entt").count();
        assert_eq!(count2, 1);

        Ok(())
    })
}

#[test]
#[serial]
fn add_multiple_deps_at_once() -> Result<()> {
    with_temp_dir(|root| {
        init_min_triton_json(root);
        init_empty_vcpkg_manifest(root);
        stub_vcpkg(&root.join("bin"));

        handle_add(
            &[String::from("glm"), String::from("sdl2"), String::from("entt")],
            None,
            false,
        )?;

        let t = read_triton(root);
        assert!(t.deps.iter().any(|d| matches!(d, DepSpec::Simple(n) if n == "glm")));
        assert!(t.deps.iter().any(|d| matches!(d, DepSpec::Simple(n) if n == "sdl2")));
        assert!(t.deps.iter().any(|d| matches!(d, DepSpec::Simple(n) if n == "entt")));

        let mani = read_vcpkg(root);
        let deps = mani["dependencies"].as_array().unwrap();
        assert!(deps.iter().any(|v| v == "glm"));
        assert!(deps.iter().any(|v| v == "sdl2"));
        assert!(deps.iter().any(|v| v == "entt"));

        Ok(())
    })
}

#[test]
#[serial]
fn add_with_arrow_syntax() -> Result<()> {
    with_temp_dir(|root| {
        init_min_triton_json(root);
        init_empty_vcpkg_manifest(root);
        stub_vcpkg(&root.join("bin"));

        handle_add(&[String::from("lua->Engine")], None, false)?;

        let t = read_triton(root);
        assert!(t.deps.iter().any(|d| matches!(d, DepSpec::Simple(n) if n == "lua")));

        let engine = t.components.get("Engine").expect("Engine component should exist");
        assert!(
            engine.link.iter().any(|e| matches!(e, LinkEntry::Name(n) if n == "lua")),
            "Engine should have lua link"
        );

        Ok(())
    })
}

#[test]
#[serial]
fn add_git_dep_full_url() -> Result<()> {
    with_temp_dir(|root| {
        init_min_triton_json(root);

        // Pre-create third_party/imgui with a dummy file to skip clone
        fs::create_dir_all(root.join("third_party/imgui"))?;
        fs::write(root.join("third_party/imgui/.gitkeep"), "")?;

        handle_add(
            &[String::from("https://github.com/ocornut/imgui.git@docking->UI")],
            None,
            false,
        )?;

        let t = read_triton(root);
        let git_dep = t.deps.iter().find(|d| matches!(d, DepSpec::Git(g) if g.name == "imgui"));
        assert!(git_dep.is_some(), "imgui git dep should exist");

        if let Some(DepSpec::Git(g)) = git_dep {
            assert_eq!(g.branch.as_deref(), Some("docking"));
        }

        let ui = t.components.get("UI").expect("UI component should exist");
        assert!(
            ui.link.iter().any(|e| matches!(e, LinkEntry::Name(n) if n == "imgui")),
            "UI should link to imgui"
        );

        Ok(())
    })
}

#[test]
#[serial]
fn add_git_dep_shorthand_no_branch() -> Result<()> {
    with_temp_dir(|root| {
        init_min_triton_json(root);

        // Pre-create third_party/filament with a dummy file to skip clone
        fs::create_dir_all(root.join("third_party/filament"))?;
        fs::write(root.join("third_party/filament/.gitkeep"), "")?;

        handle_add(
            &[String::from("google/filament:Render")],
            None,
            false,
        )?;

        let t = read_triton(root);
        let git_dep = t.deps.iter().find(|d| matches!(d, DepSpec::Git(g) if g.name == "filament"));
        assert!(git_dep.is_some(), "filament git dep should exist");

        if let Some(DepSpec::Git(g)) = git_dep {
            assert!(g.branch.is_none(), "branch should be None when not specified");
        }

        let render = t.components.get("Render").expect("Render component should exist");
        assert!(
            render.link.iter().any(|e| matches!(e, LinkEntry::Name(n) if n == "filament")),
            "Render should link to filament"
        );

        Ok(())
    })
}

#[test]
#[serial]
fn add_to_existing_component_preserves_other_links() -> Result<()> {
    with_temp_dir(|root| {
        init_min_triton_json(root);
        init_empty_vcpkg_manifest(root);
        stub_vcpkg(&root.join("bin"));

        // Pre-create "Game" component with an existing link to "glm"
        {
            let mut t: TritonRoot = read_json(root.join("triton.json")).unwrap();
            t.deps.push(DepSpec::Simple("glm".into()));
            t.components.insert(
                "Game".into(),
                TritonComponent {
                    kind: "exe".into(),
                    link: vec![LinkEntry::Name("glm".into())],
                    defines: vec![],
                    exports: vec![],
                    resources: vec![],
                    link_options: Default::default(),
                    vendor_libs: Default::default(),
                    assets: vec![],
                },
            );
            write_json_pretty_changed(root.join("triton.json"), &t).unwrap();
            fs::create_dir_all(root.join("components/Game/src")).unwrap();
            fs::create_dir_all(root.join("components/Game/include")).unwrap();
        }

        // Add sdl2 linked to the existing Game component
        handle_add(&[String::from("sdl2:Game")], None, false)?;

        let t = read_triton(root);
        let game = t.components.get("Game").expect("Game component should exist");
        assert!(
            game.link.iter().any(|e| matches!(e, LinkEntry::Name(n) if n == "glm")),
            "Game should still have glm link"
        );
        assert!(
            game.link.iter().any(|e| matches!(e, LinkEntry::Name(n) if n == "sdl2")),
            "Game should also have sdl2 link"
        );

        Ok(())
    })
}

#[test]
#[serial]
fn add_creates_vcpkg_json_if_missing() -> Result<()> {
    with_temp_dir(|root| {
        init_min_triton_json(root);
        stub_vcpkg(&root.join("bin"));

        // Deliberately do NOT create vcpkg.json
        assert!(!root.join("vcpkg.json").exists());

        handle_add(&[String::from("glm")], None, false)?;

        // vcpkg.json should now exist
        assert!(root.join("vcpkg.json").exists(), "vcpkg.json should be created automatically");

        let mani = read_vcpkg(root);
        let deps = mani["dependencies"].as_array().unwrap();
        assert!(deps.iter().any(|v| v == "glm"));

        Ok(())
    })
}

// ── Feature flag tests ──────────────────────────────────────────────────

#[test]
#[serial]
fn add_with_features_creates_detailed_dep_in_triton_json() -> Result<()> {
    with_temp_dir(|root| {
        init_min_triton_json(root);
        init_empty_vcpkg_manifest(root);
        stub_vcpkg(&root.join("bin"));

        handle_add(&[String::from("directxtex")], Some("dx12"), false)?;

        let t = read_triton(root);
        let dep = t.deps.iter().find(|d| d.name() == "directxtex");
        assert!(dep.is_some(), "directxtex dep should exist");
        match dep.unwrap() {
            DepSpec::Detailed(d) => {
                assert!(d.features.contains(&"dx12".to_string()),
                    "features should contain dx12, got: {:?}", d.features);
            }
            other => panic!("expected DepDetailed, got: {:?}", other),
        }

        Ok(())
    })
}

#[test]
#[serial]
fn add_with_features_writes_object_form_to_vcpkg_json() -> Result<()> {
    with_temp_dir(|root| {
        init_min_triton_json(root);
        init_empty_vcpkg_manifest(root);
        stub_vcpkg(&root.join("bin"));

        handle_add(&[String::from("directxtex")], Some("dx12,dx11"), false)?;

        let mani = read_vcpkg(root);
        let deps = mani["dependencies"].as_array().unwrap();

        // Should be object form with features, not a plain string
        let dtex = deps.iter().find(|v| {
            v.get("name").and_then(|n| n.as_str()) == Some("directxtex")
        });
        assert!(dtex.is_some(), "vcpkg.json should have directxtex as object, got: {:?}", deps);

        let features = dtex.unwrap()["features"].as_array().unwrap();
        let feature_strs: Vec<&str> = features.iter().filter_map(|f| f.as_str()).collect();
        assert!(feature_strs.contains(&"dx11"), "should contain dx11");
        assert!(feature_strs.contains(&"dx12"), "should contain dx12");

        Ok(())
    })
}

#[test]
#[serial]
fn add_without_features_creates_simple_dep() -> Result<()> {
    with_temp_dir(|root| {
        init_min_triton_json(root);
        init_empty_vcpkg_manifest(root);
        stub_vcpkg(&root.join("bin"));

        handle_add(&[String::from("glm")], None, false)?;

        let t = read_triton(root);
        let dep = t.deps.iter().find(|d| d.name() == "glm");
        assert!(matches!(dep, Some(DepSpec::Simple(_))), "should be Simple dep without features");

        let mani = read_vcpkg(root);
        let deps = mani["dependencies"].as_array().unwrap();
        assert!(deps.iter().any(|v| v.as_str() == Some("glm")), "vcpkg.json should have plain string 'glm'");

        Ok(())
    })
}

#[test]
#[serial]
fn add_features_to_existing_simple_dep_upgrades_to_detailed() -> Result<()> {
    with_temp_dir(|root| {
        init_min_triton_json(root);
        init_empty_vcpkg_manifest(root);
        stub_vcpkg(&root.join("bin"));

        // First add without features
        handle_add(&[String::from("directxtex")], None, false)?;
        let t = read_triton(root);
        assert!(matches!(t.deps.iter().find(|d| d.name() == "directxtex"), Some(DepSpec::Simple(_))));

        // Now add same dep with features — should upgrade to Detailed
        handle_add(&[String::from("directxtex")], Some("dx12"), false)?;
        let t2 = read_triton(root);
        match t2.deps.iter().find(|d| d.name() == "directxtex") {
            Some(DepSpec::Detailed(d)) => {
                assert!(d.features.contains(&"dx12".to_string()),
                    "features should contain dx12 after upgrade");
            }
            other => panic!("expected DepDetailed after upgrade, got: {:?}", other),
        }

        // vcpkg.json should now have object form
        let mani = read_vcpkg(root);
        let deps = mani["dependencies"].as_array().unwrap();
        let dtex = deps.iter().find(|v| {
            v.get("name").and_then(|n| n.as_str()) == Some("directxtex")
        });
        assert!(dtex.is_some(), "vcpkg.json should have object form after upgrade");

        Ok(())
    })
}

#[test]
#[serial]
fn add_features_merges_with_existing_features() -> Result<()> {
    with_temp_dir(|root| {
        init_min_triton_json(root);
        init_empty_vcpkg_manifest(root);
        stub_vcpkg(&root.join("bin"));

        // Add with dx11
        handle_add(&[String::from("directxtex")], Some("dx11"), false)?;

        // Add again with dx12 — should merge features
        handle_add(&[String::from("directxtex")], Some("dx12"), false)?;

        let t = read_triton(root);
        match t.deps.iter().find(|d| d.name() == "directxtex") {
            Some(DepSpec::Detailed(d)) => {
                assert!(d.features.contains(&"dx11".to_string()), "should still have dx11");
                assert!(d.features.contains(&"dx12".to_string()), "should also have dx12");
            }
            other => panic!("expected DepDetailed with merged features, got: {:?}", other),
        }

        // vcpkg.json should have both features
        let mani = read_vcpkg(root);
        let deps = mani["dependencies"].as_array().unwrap();
        let dtex = deps.iter().find(|v| {
            v.get("name").and_then(|n| n.as_str()) == Some("directxtex")
        }).expect("directxtex object should exist in vcpkg.json");
        let features = dtex["features"].as_array().unwrap();
        let feature_strs: Vec<&str> = features.iter().filter_map(|f| f.as_str()).collect();
        assert!(feature_strs.contains(&"dx11"), "vcpkg.json should have dx11");
        assert!(feature_strs.contains(&"dx12"), "vcpkg.json should have dx12");

        Ok(())
    })
}

#[test]
#[serial]
fn add_same_dep_twice_is_idempotent_in_triton_json() -> Result<()> {
    with_temp_dir(|root| {
        init_min_triton_json(root);
        init_empty_vcpkg_manifest(root);
        stub_vcpkg(&root.join("bin"));

        handle_add(&[String::from("directxtex")], Some("dx12"), false)?;
        handle_add(&[String::from("directxtex")], Some("dx12"), false)?;

        let t = read_triton(root);
        let count = t.deps.iter().filter(|d| d.name() == "directxtex").count();
        assert_eq!(count, 1, "should have exactly 1 directxtex dep, got {}", count);

        Ok(())
    })
}

#[test]
#[serial]
fn add_same_dep_twice_with_link_is_idempotent() -> Result<()> {
    with_temp_dir(|root| {
        init_min_triton_json(root);
        init_empty_vcpkg_manifest(root);
        stub_vcpkg(&root.join("bin"));

        handle_add(&[String::from("glm:App")], None, false)?;
        handle_add(&[String::from("glm:App")], None, false)?;

        let t = read_triton(root);
        // Only 1 dep entry
        let dep_count = t.deps.iter().filter(|d| d.name() == "glm").count();
        assert_eq!(dep_count, 1, "should have exactly 1 glm dep");

        // Only 1 link entry in App
        let app = t.components.get("App").expect("App should exist");
        let link_count = app.link.iter().filter(|e| e.normalize().0 == "glm").count();
        assert_eq!(link_count, 1, "App should have exactly 1 glm link entry");

        // vcpkg.json should have exactly 1 glm entry
        let mani = read_vcpkg(root);
        let deps = mani["dependencies"].as_array().unwrap();
        let glm_count = deps.iter().filter(|v| v.as_str() == Some("glm")).count();
        assert_eq!(glm_count, 1, "vcpkg.json should have exactly 1 glm");

        Ok(())
    })
}
