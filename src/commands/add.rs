use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

use crate::cmake::{regenerate_root_cmake, rewrite_component_cmake};
use crate::models::{
    dep_eq, Dependency, DependencyDetail, TritonComponent, TritonRoot, VcpkgManifest,
};
use crate::templates::component_cmakelists;
use crate::util::{
    read_json, run, vcpkg_exe_path, write_json_pretty_changed, write_text_if_changed,
};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct ThirdPartyDep {
    repo: String,           // "owner/repo"
    name: String,           // local folder name under third_party (usually "repo")
    #[serde(default)]
    target: Option<String>, // optional cmake target to link
    #[serde(default)]
    branch: Option<String>, // optional branch/tag
}

fn third_party_list_path(component: &str) -> String {
    format!("components/{component}/third_party.json")
}

fn load_third_party(component: &str) -> Result<Vec<ThirdPartyDep>> {
    let p = third_party_list_path(component);
    let path = Path::new(&p);
    if path.exists() {
        Ok(read_json(path)?)
    } else {
        Ok(vec![])
    }
}

fn save_third_party(component: &str, list: &Vec<ThirdPartyDep>) -> Result<()> {
    let _ = write_json_pretty_changed(third_party_list_path(component), list)?;
    Ok(())
}

fn ensure_component_scaffold(component: &str, root: &mut TritonRoot) -> Result<()> {
    if !root.components.contains_key(component) {
        fs::create_dir_all(format!("components/{component}/src"))?;
        fs::create_dir_all(format!("components/{component}/include"))?;
        write_text_if_changed(
            &format!("components/{component}/CMakeLists.txt"),
            &component_cmakelists(),
        )?;
        root.components.insert(
            component.into(),
            TritonComponent {
                kind: "lib".into(),
                deps: vec![],
                comps: vec![],
            },
        );
    }
    Ok(())
}

fn clone_github_repo(owner_repo: &str, name: &str, branch: Option<&str>) -> Result<()> {
    let tp_dir = PathBuf::from("third_party").join(name);
    if tp_dir.exists() {
        // already vendored; consider pulling in the future
        return Ok(());
    }
    fs::create_dir_all("third_party")?;
    let url = format!("https://github.com/{owner_repo}.git");
    let mut args = vec!["clone", "--depth", "1"];
    let branch_owned;
    if let Some(b) = branch {
        branch_owned = b.to_string();
        args.push("--branch");
        args.push(&branch_owned);
    }
    args.push(&url);
    let dst = tp_dir
        .to_str()
        .ok_or_else(|| anyhow!("invalid target path"))?
        .to_string();
    args.push(&dst);
    run("git", &args, ".").context("git clone third_party repo failed")?;
    Ok(())
}

pub fn handle_add(pkg: &str, component: &str, features: Option<&str>, host: bool) -> Result<()> {
    // Load root metadata & ensure component folder/files
    let mut root: TritonRoot = read_json("triton.json")?;
    ensure_component_scaffold(component, &mut root)?;

    // GitHub vendoring if the arg looks like "owner/repo"
    let is_github = pkg.contains('/') && !pkg.contains('\\');
    if is_github {
        let owner_repo = pkg.trim().trim_end_matches(".git");
        let repo_name = owner_repo
            .split('/')
            .last()
            .ok_or_else(|| anyhow!("invalid repo name: {pkg}"))?;
        clone_github_repo(owner_repo, repo_name, None)?;

        // Register in component's third_party.json
        let mut tp = load_third_party(component)?;
        if !tp.iter().any(|t| t.repo == owner_repo || t.name == repo_name) {
            tp.push(ThirdPartyDep {
                repo: owner_repo.to_string(),
                name: repo_name.to_string(),
                target: Some(repo_name.to_string()), // best guess; user can edit if needed
                branch: None,
            });
            save_third_party(component, &tp)?;
        }

        // Rewrite only this component and root
        rewrite_component_cmake(component, root.components.get(component).unwrap())?;
        regenerate_root_cmake(&root)?;
        eprintln!("Vendored GitHub repo '{}' into third_party/{} and wired to component '{}'.", owner_repo, repo_name, component);
        return Ok(());
    }

    // ----- normal vcpkg package path -----

    // Update component deps (only for vcpkg packages)
    {
        let comp = root.components.get_mut(component).unwrap();
        if !comp.deps.iter().any(|d| d == pkg) {
            comp.deps.push(pkg.to_string());
        }
    }
    write_json_pretty_changed("triton.json", &root)?;
    write_json_pretty_changed(
        &format!("components/{component}/triton.json"),
        root.components.get(component).unwrap(),
    )?;

    // Update vcpkg.json
    let mut mani: VcpkgManifest = read_json("vcpkg.json")?;
    let dep = {
        let feats: Vec<String> = features
            .unwrap_or_default()
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        if feats.is_empty() && !host {
            Dependency::Name(pkg.into())
        } else {
            Dependency::Detailed(DependencyDetail {
                name: pkg.into(),
                features: feats,
                default_features: None, // let vcpkg handle defaults
                host: if host { Some(true) } else { None },
                platform: None,
            })
        }
    };

    if !mani.dependencies.iter().any(|d| dep_eq(d, &dep)) {
        mani.dependencies.push(dep);
    }
    write_json_pretty_changed("vcpkg.json", &mani)?;

    // vcpkg install (manifest mode)
    let vcpkg_bin = vcpkg_exe_path();
    eprintln!("Running vcpkg install (manifest mode)...");
    run(&vcpkg_bin, &["install"], ".")?;

    // Rewrite CMake for this component and root
    rewrite_component_cmake(component, root.components.get(component).unwrap())?;
    regenerate_root_cmake(&root)?;

    eprintln!("Added '{}' to component '{}'.", pkg, component);
    Ok(())
}
