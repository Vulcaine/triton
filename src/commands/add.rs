use anyhow::Result;
use std::fs;
use std::path::Path;

use crate::cmake::{regenerate_root_cmake, rewrite_component_cmake};
use crate::models::{RootDep, GitDep, TritonComponent, TritonRoot};
use crate::util::{read_json, run, vcpkg_exe_path, write_json_pretty_changed, write_text_if_changed};

fn choose_component<'a>(root: &'a TritonRoot, requested: Option<&str>) -> String {
    if let Some(r) = requested {
        if root.components.contains_key(r) { return r.to_string(); }
    }
    if let Some((name, _)) = root.components.iter().find(|(_, c)| c.kind == "exe") {
        return name.clone();
    }
    root.components.keys().next().cloned().unwrap_or_else(|| root.app_name.clone())
}

pub fn handle_add(pkg: &str, component_opt: Option<&str>, features: Option<&str>, host: bool) -> Result<()> {
    let mut root: TritonRoot = read_json("triton.json")?;
    let comp_name = choose_component(&root, component_opt);

    // Ensure component exists
    root.components.entry(comp_name.clone()).or_insert(TritonComponent { kind: "lib".into(), link: vec![] });

    let mut dep_name_for_link: String;

    if pkg.contains('/') && !pkg.contains('\\') {
        // GIT path
        let (repo, branch) = if let Some((r, b)) = pkg.split_once('@') { (r.to_string(), Some(b.to_string())) } else { (pkg.to_string(), None) };
        let name = repo.split('/').last().unwrap_or(&repo).to_string();
        dep_name_for_link = name.clone();

        let third = format!("third_party/{name}");
        if !Path::new(&third).exists() {
            fs::create_dir_all("third_party")?;
            eprintln!("Cloning https://github.com/{repo}.git into {third} …");
            run("git", &["clone", &format!("https://github.com/{repo}.git"), &third], ".")?;
            if let Some(br) = &branch { run("git", &["checkout", br], &third)?; }
        }

        if !root.deps.iter().any(|d| matches!(d, RootDep::Git(g) if g.name == name || g.repo == repo)) {
            root.deps.push(RootDep::Git(GitDep { repo, name: name.clone(), branch, target: None, cmake: vec![] }));
        }
    } else {
        // vcpkg package name
        dep_name_for_link = pkg.to_string();

        if !root.deps.iter().any(|d| matches!(d, RootDep::Name(n) if n == pkg)) {
            root.deps.push(RootDep::Name(pkg.to_string()));
        }

        // keep vcpkg.json in sync and install
        let mut mani: serde_json::Value = read_json("vcpkg.json")?;
        let deps = mani["dependencies"].as_array_mut().unwrap();
        if !deps.iter().any(|v| v == pkg) {
            deps.push(serde_json::Value::String(pkg.to_string()));
            write_text_if_changed("vcpkg.json", &serde_json::to_string_pretty(&mani)?)?;
        }
        let vcpkg_bin = vcpkg_exe_path();
        eprintln!("Running vcpkg install (manifest mode)...");
        run(&vcpkg_bin, &["install"], ".")?;
    }

    // auto-link into chosen component if not already linked
    if let Some(c) = root.components.get_mut(&comp_name) {
        if !c.link.iter().any(|x| x == &dep_name_for_link) {
            c.link.push(dep_name_for_link);
        }
    }

    write_json_pretty_changed("triton.json", &root)?;

    // regenerate cmake
    let comp = root.components.get(&comp_name).unwrap();
    rewrite_component_cmake(&comp_name, &root, comp)?;
    regenerate_root_cmake(&root)?;
    eprintln!("Added '{}' and linked into component '{}'.", pkg, comp_name);
    Ok(())
}
