use anyhow::{Context, Result};
use std::fs;
use std::path::Path;

use crate::cmake::{regenerate_root_cmake, rewrite_component_cmake};
use crate::models::{GitDep, RootDep, TritonComponent, TritonRoot};
use crate::templates::component_cmakelists;
use crate::util::{
    read_json, run, vcpkg_exe_path, write_json_pretty_changed, write_text_if_changed,
};

struct Plan {
    pkg: String,
    link_to: Option<String>,
}

fn is_git_token(s: &str) -> bool {
    s.contains('/') && !s.contains('\\')
}

fn make_plans(mut items: Vec<String>, root: &TritonRoot) -> Vec<Plan> {
    let mut plans = Vec::<Plan>::new();
    let mut plain = Vec::<String>::new();

    for it in items.drain(..) {
        if let Some((p, c)) = it.split_once("->") {
            let p = p.trim();
            let c = c.trim();
            if !p.is_empty() && !c.is_empty() {
                plans.push(Plan { pkg: p.to_string(), link_to: Some(c.to_string()) });
                continue;
            }
        }
        plain.push(it);
    }

    if plain.is_empty() {
        return plans;
    }

    // trailing component sugar only if last token is an *existing* component
    let mut global_comp: Option<String> = None;
    if plain.len() >= 2 {
        if let Some(last) = plain.last() {
            if root.components.contains_key(last) {
                global_comp = Some(last.clone());
                plain.pop();
            }
        }
    }

    for p in plain {
        plans.push(Plan { pkg: p, link_to: global_comp.clone() });
    }

    plans
}

fn ensure_component_scaffold(name: &str) -> Result<()> {
    let comp_dir = format!("components/{name}");
    fs::create_dir_all(format!("{comp_dir}/src"))?;
    fs::create_dir_all(format!("{comp_dir}/include"))?;
    let cm = format!("{comp_dir}/CMakeLists.txt");
    if !Path::new(&cm).exists() {
        write_text_if_changed(&cm, &crate::templates::component_cmakelists())?;
    }
    Ok(())
}

fn add_vcpkg_dep_transactional(root: &TritonRoot, pkg: &str) -> Result<bool> {
    let mani_path = Path::new("vcpkg.json");
    let existed_before = mani_path.exists();
    let prev_text = if existed_before {
        Some(std::fs::read_to_string(mani_path).unwrap_or_default())
    } else {
        None
    };

    if !existed_before {
        let empty = serde_json::json!({
            "name": root.app_name,
            "version": "0.0.0",
            "dependencies": []
        });
        write_text_if_changed("vcpkg.json", &serde_json::to_string_pretty(&empty)?)?;
    }

    let mut mani: serde_json::Value = read_json("vcpkg.json")?;
    {
        let deps = mani["dependencies"].as_array_mut().expect("dependencies array");
        if !deps.iter().any(|v| v == pkg) {
            deps.push(serde_json::Value::String(pkg.to_string()));
            write_text_if_changed("vcpkg.json", &serde_json::to_string_pretty(&mani)?)?;
        }
    }

    let vcpkg_bin = vcpkg_exe_path();
    let res = run(&vcpkg_bin, &["install"], ".");
    if let Err(e) = res {
        if let Some(text) = prev_text {
            write_text_if_changed("vcpkg.json", &text)
                .context("reverting failed vcpkg.json change")?;
        } else {
            let _ = std::fs::remove_file(mani_path);
        }
        eprintln!("{}", e);
        eprintln!("vcpkg install failed; not adding '{}' to vcpkg.json.", pkg);
        return Ok(false);
    }

    Ok(true)
}

pub fn handle_add(items: &[String], _features: Option<&str>, _host: bool) -> Result<()> {
    let mut root: TritonRoot = read_json("triton.json")?;
    fs::create_dir_all("components")?; // safe

    let plans = make_plans(items.to_vec(), &root);
    if plans.is_empty() {
        anyhow::bail!("Nothing to add. Usage: triton add <pkg...> [component] or pkg->component");
    }

    for Plan { pkg, link_to } in plans {
        let mut dep_name_for_link: Option<String> = None;

        if is_git_token(&pkg) {
            let (repo, branch) = if let Some((r, b)) = pkg.split_once('@') {
                (r.to_string(), Some(b.to_string()))
            } else {
                (pkg.to_string(), None)
            };
            let name = repo.split('/').last().unwrap_or(&repo).to_string();

            // clone first; only then persist
            let third = format!("third_party/{name}");
            if !Path::new(&third).exists() {
                fs::create_dir_all("third_party")?;
                eprintln!("Cloning https://github.com/{repo}.git into {third} …");
                run("git", &["clone", &format!("https://github.com/{repo}.git"), &third], ".")?;
                if let Some(br) = &branch {
                    run("git", &["checkout", br], &third)?;
                }
            }

            if !root.deps.iter().any(|d| matches!(d, RootDep::Git(g) if g.name == name || g.repo == repo)) {
                root.deps.push(RootDep::Git(GitDep {
                    repo,
                    name: name.clone(),
                    branch,
                    target: None,
                    cmake: vec![],
                }));
                write_json_pretty_changed("triton.json", &root)?;
            }

            dep_name_for_link = Some(name);
        } else {
            if add_vcpkg_dep_transactional(&root, &pkg)? {
                if !root.deps.iter().any(|d| matches!(d, RootDep::Name(n) if n == &pkg)) {
                    root.deps.push(RootDep::Name(pkg.clone()));
                    write_json_pretty_changed("triton.json", &root)?;
                }
                dep_name_for_link = Some(pkg.clone());
            } else {
                // vcpkg failed; skip linking
                continue;
            }
        }

        if let Some(dest_comp) = link_to.as_deref() {
            ensure_component_scaffold(dest_comp)?;
            {
                let entry = root.components.entry(dest_comp.to_string())
                    .or_insert(TritonComponent { kind: "lib".into(), link: vec![] });
                if let Some(link_key) = &dep_name_for_link {
                    if !entry.link.iter().any(|x| x == link_key) {
                        entry.link.push(link_key.clone());
                    }
                }
            }
            write_json_pretty_changed("triton.json", &root)?;
            let comp_ref = root.components.get(dest_comp).unwrap();
            rewrite_component_cmake(dest_comp, &root, comp_ref)?;
            regenerate_root_cmake(&root)?;
            eprintln!("Added '{}' and linked into component '{}'.", pkg, dest_comp);
        } else {
            regenerate_root_cmake(&root)?;
            eprintln!("Added '{}' to project dependencies (no linking).", pkg);
        }
    }

    Ok(())
}
