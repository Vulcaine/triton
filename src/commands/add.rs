use anyhow::Result;
use std::fs;
use std::path::Path;

use crate::models::{GitDep, LinkEntry, RootDep, TritonComponent, TritonRoot};
use crate::util::{read_json, run, write_json_pretty_changed, write_text_if_changed};

/// Parse `"<pkg>"`, `"<pkg> <component>"`, or `"<pkg>:<component>"`.
fn parse_pkg_and_component<'a>(pkg: &'a str, component_opt: Option<&'a str>) -> (&'a str, Option<&'a str>) {
    if let Some((p, c)) = pkg.split_once(':') {
        let p = p.trim();
        let c = c.trim();
        if !c.is_empty() { return (p, Some(c)); }
        return (p, None);
    }
    (pkg, component_opt.map(|s| s.trim()).filter(|s| !s.is_empty()))
}

pub fn handle_add(items: &[String], _features: Option<&str>, _host: bool) -> Result<()> {
    if items.is_empty() { return Ok(()); }

    let mut root: TritonRoot = read_json("triton.json")?;
    fs::create_dir_all("components")?;

    // First: update deps (vcpkg or git), run vcpkg manifest install only when we actually changed vcpkg.json
    let mut touched_vcpkg_manifest = false;

    for it in items {
        let (pkg, link_to_opt) = parse_pkg_and_component(it, None);

        if pkg.contains('/') && !pkg.contains('\\') {
            // git dep
            let (repo, branch) = if let Some((r, b)) = pkg.split_once('@') { (r.to_string(), Some(b.to_string())) } else { (pkg.to_string(), None) };
            let name = repo.split('/').last().unwrap_or(&repo).to_string();

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

            if let Some(dest) = link_to_opt {
                // ensure component dir & cmakelists
                let base = format!("components/{dest}");
                fs::create_dir_all(format!("{base}/src"))?;
                fs::create_dir_all(format!("{base}/include"))?;
                let cm = format!("{base}/CMakeLists.txt");
                if !Path::new(&cm).exists() {
                    write_text_if_changed(&cm, &crate::templates::component_cmakelists())?;
                }
                let entry = root.components.entry(dest.to_string())
                    .or_insert(TritonComponent { kind: "lib".into(), link: vec![], defines: vec![] });
                if !entry.link.iter().any(|e| matches!(e, LinkEntry::Name(n) if n == &name)) {
                    entry.link.push(LinkEntry::Name(name));
                }
            }
        } else {
            // vcpkg dep
            if !root.deps.iter().any(|d| matches!(d, RootDep::Name(n) if n == pkg)) {
                root.deps.push(RootDep::Name(pkg.to_string()));
            }

            let manifest_path = Path::new("vcpkg.json");
            if !manifest_path.exists() {
                let empty = serde_json::json!({ "name": root.app_name, "version":"0.0.0", "dependencies": [] });
                write_text_if_changed("vcpkg.json", &serde_json::to_string_pretty(&empty)?)?;
            }
            // Update manifest only if missing
            let mut mani: serde_json::Value = crate::util::read_json("vcpkg.json")?;
            let deps = mani["dependencies"].as_array_mut().unwrap();
            if !deps.iter().any(|v| v == pkg) {
                deps.push(serde_json::Value::String(pkg.to_string()));
                write_text_if_changed("vcpkg.json", &serde_json::to_string_pretty(&mani)?)?;
                touched_vcpkg_manifest = true;
            }

            if let Some(dest) = link_to_opt {
                let base = format!("components/{dest}");
                fs::create_dir_all(format!("{base}/src"))?;
                fs::create_dir_all(format!("{base}/include"))?;
                let cm = format!("{base}/CMakeLists.txt");
                if !Path::new(&cm).exists() {
                    write_text_if_changed(&cm, &crate::templates::component_cmakelists())?;
                }
                let entry = root.components.entry(dest.to_string())
                    .or_insert(TritonComponent { kind: "lib".into(), link: vec![], defines: vec![]  });
                if !entry.link.iter().any(|e| matches!(e, LinkEntry::Name(n) if n == pkg)) {
                    entry.link.push(LinkEntry::Name(pkg.to_string()));
                }
            }
        }
    }

    write_json_pretty_changed("triton.json", &root)?;

    // vcpkg install after updating manifest
    if touched_vcpkg_manifest {
        let vcpkg_bin = crate::util::vcpkg_exe_path();
        eprintln!("Running vcpkg install (manifest mode)...");
        // If this fails, revert manifest changes? We keep it simple: the error aborts `add`.
        crate::util::run(&vcpkg_bin, &["install"], ".")?;
    }

    // Regenerate cmake
    for (name, comp) in &root.components { crate::cmake::rewrite_component_cmake(name, &root, comp)?; }
    crate::cmake::regenerate_root_cmake(&root)?;
    Ok(())
}
