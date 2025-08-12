use anyhow::Result;
use std::fs;
use std::path::Path;

use crate::cmake::{regenerate_root_cmake, rewrite_component_cmake};
use crate::models::{GitDep, RootDep, TritonComponent, TritonRoot};
use crate::templates::component_cmakelists;
use crate::util::{
    read_json, run, vcpkg_exe_path, write_json_pretty_changed, write_text_if_changed,
};

/// Parse `"<pkg>"`, `"<pkg> <component>"`, or `"<pkg>-><component>"`.
fn parse_pkg_and_component<'a>(pkg: &'a str, component_opt: Option<&'a str>) -> (&'a str, Option<&'a str>) {
    if let Some((p, c)) = pkg.split_once("->") {
        let p = p.trim();
        let c = c.trim();
        if !c.is_empty() {
            return (p, Some(c));
        }
        return (p, None);
    }
    (pkg, component_opt.map(|s| s.trim()).filter(|s| !s.is_empty()))
}

pub fn handle_add(pkg_in: &str, component_opt: Option<&str>, _features: Option<&str>, _host: bool) -> Result<()> {
    let (pkg, link_to_opt) = parse_pkg_and_component(pkg_in, component_opt);

    let mut root: TritonRoot = read_json("triton.json")?;

    // Ensure components/ root exists when we might need to create a component
    if link_to_opt.is_some() {
        fs::create_dir_all("components")?;
    }

    // Add dependency to root.deps (either vcpkg name or git repo)
    let mut dep_name_for_link: Option<String> = None;

    if pkg.contains('/') && !pkg.contains('\\') {
        // Treat as GitHub "org/repo[@branch]"
        let (repo, branch) = if let Some((r, b)) = pkg.split_once('@') {
            (r.to_string(), Some(b.to_string()))
        } else {
            (pkg.to_string(), None)
        };
        let name = repo.split('/').last().unwrap_or(&repo).to_string();
        dep_name_for_link = Some(name.clone());

        let third = format!("third_party/{name}");
        if !Path::new(&third).exists() {
            fs::create_dir_all("third_party")?;
            eprintln!("Cloning https://github.com/{repo}.git into {third} …");
            run("git", &["clone", &format!("https://github.com/{repo}.git"), &third], ".")?;
            if let Some(br) = &branch {
                run("git", &["checkout", br], &third)?;
            }
        }

        if !root
            .deps
            .iter()
            .any(|d| matches!(d, RootDep::Git(g) if g.name == name || g.repo == repo))
        {
            root.deps.push(RootDep::Git(GitDep {
                repo,
                name: name.clone(),
                branch,
                target: None,
                cmake: vec![],
            }));
        }
    } else {
        // vcpkg package name
        if !root.deps.iter().any(|d| matches!(d, RootDep::Name(n) if n == pkg)) {
            root.deps.push(RootDep::Name(pkg.to_string()));
        }

        // Ensure vcpkg.json exists and is in sync
        let mani_path = Path::new("vcpkg.json");
        if !mani_path.exists() {
            let empty = serde_json::json!({
                "name": root.app_name,
                "version": "0.0.0",
                "dependencies": []
            });
            write_text_if_changed("vcpkg.json", &serde_json::to_string_pretty(&empty)?)?;
        }

        let mut mani: serde_json::Value = read_json("vcpkg.json")?;
        let deps = mani["dependencies"].as_array_mut().unwrap();
        if !deps.iter().any(|v| v == pkg) {
            deps.push(serde_json::Value::String(pkg.to_string()));
            write_text_if_changed("vcpkg.json", &serde_json::to_string_pretty(&mani)?)?;
        }

        // vcpkg install (manifest mode)
        let vcpkg_bin = vcpkg_exe_path();
        eprintln!("Running vcpkg install (manifest mode)...");
        run(&vcpkg_bin, &["install"], ".")?;

        // For linking, the link key is the vcpkg package name itself
        dep_name_for_link = Some(pkg.to_string());
    }

    // Persist deps update first
    write_json_pretty_changed("triton.json", &root)?;

    // If a component was specified, link it and ensure it exists on disk + metadata
    if let Some(dest_comp) = link_to_opt {
        // Ensure component folder + CMakeLists exist (non-destructive)
        let comp_dir = format!("components/{dest_comp}");
        fs::create_dir_all(format!("{comp_dir}/src"))?;
        fs::create_dir_all(format!("{comp_dir}/include"))?;
        let cm = format!("{comp_dir}/CMakeLists.txt");
        if !Path::new(&cm).exists() {
            write_text_if_changed(&cm, &component_cmakelists())?;
        }

        // --- begin mutable borrow scope
        {
            // Ensure metadata entry and push link if missing
            let entry = root
                .components
                .entry(dest_comp.to_string())
                .or_insert(TritonComponent {
                    kind: "lib".into(),
                    link: vec![],
                });

            if let Some(link_key) = &dep_name_for_link {
                if !entry.link.iter().any(|x| x == link_key) {
                    entry.link.push(link_key.clone());
                }
            }
        } // <-- mutable borrow of `root` ends here

        // Persist updated root with new link
        write_json_pretty_changed("triton.json", &root)?;

        // Re-borrow immutably for codegen
        let comp_ref = root
            .components
            .get(dest_comp)
            .expect("component should exist after insertion");

        // Regenerate cmake for that component and components root
        rewrite_component_cmake(dest_comp, &root, comp_ref)?;
        regenerate_root_cmake(&root)?;

        eprintln!("Added '{}' and linked into component '{}'.", pkg_in, dest_comp);
    } else {
        // No component specified: deps updated only
        regenerate_root_cmake(&root)?; // keep components/CMakeLists.txt in sync
        eprintln!("Added '{}' to project dependencies (no linking).", pkg_in);
    }

    Ok(())
}
