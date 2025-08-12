use anyhow::Result;
use std::fs;
use std::path::Path;

use crate::cmake::{regenerate_root_cmake, rewrite_component_cmake};
use crate::models::{
    dep_eq, Dependency, DependencyDetail, GitDep, TritonComponent, TritonRoot, VcpkgManifest,
};
use crate::templates::component_cmakelists;
use crate::util::{
    read_json, run, vcpkg_exe_path, write_json_pretty_changed, write_text_if_changed,
};

fn choose_component<'a>(root: &'a TritonRoot, requested: Option<&str>) -> String {
    if let Some(r) = requested {
        if root.components.contains_key(r) {
            return r.to_string();
        }
    }
    // Prefer the exe component
    if let Some((name, _)) = root.components.iter().find(|(_, c)| c.kind == "exe") {
        return name.clone();
    }
    // Fallback: first component or "app"
    root.components
        .keys()
        .next()
        .cloned()
        .unwrap_or_else(|| "app".to_string())
}

pub fn handle_add(pkg: &str, component_opt: Option<&str>, features: Option<&str>, host: bool) -> Result<()> {
    // Load root metadata
    let mut root: TritonRoot = read_json("triton.json")?;
    let comp_name = choose_component(&root, component_opt);

    // Scaffold component if missing (rare)
    if !root.components.contains_key(&comp_name) {
        fs::create_dir_all(format!("components/{comp_name}/src"))?;
        fs::create_dir_all(format!("components/{comp_name}/include"))?;
        write_text_if_changed(
            &format!("components/{comp_name}/CMakeLists.txt"),
            &component_cmakelists(),
        )?;
        root.components.insert(
            comp_name.clone(),
            TritonComponent {
                kind: "lib".into(),
                deps: vec![],
                comps: vec![],
                git: vec![],
            },
        );
    }

    // GitHub vendoring path if the arg looks like owner/name or owner/name@branch
    if pkg.contains('/') && !pkg.contains('\\') {
        let (repo, branch) = if let Some((r, b)) = pkg.split_once('@') {
            (r.to_string(), Some(b.to_string()))
        } else {
            (pkg.to_string(), None)
        };
        let tail = repo.split('/').last().unwrap_or(pkg).to_string();
        let third = format!("third_party/{tail}");

        if !Path::new(&third).exists() {
            fs::create_dir_all("third_party")?;
            eprintln!("Cloning https://github.com/{repo}.git into {third} …");
            run(
                "git",
                &["clone", &format!("https://github.com/{repo}.git"), &third],
                ".",
            )?;
            if let Some(br) = &branch {
                run("git", &["checkout", br], &third)?;
            }
        }

        // Add to component git list if missing
        {
            let comp = root.components.get_mut(&comp_name).unwrap();
            if !comp.git.iter().any(|g| g.repo == repo) {
                comp.git.push(GitDep {
                    repo: repo.clone(),
                    name: tail.clone(),
                    target: None,
                    branch: branch.clone(),
                });
            }
        }

        // Persist component + root
        write_json_pretty_changed(
            &format!("components/{comp_name}/triton.json"),
            root.components.get(&comp_name).unwrap(),
        )?;
        write_json_pretty_changed("triton.json", &root)?;

        // Rewrite CMake for this component and root
        rewrite_component_cmake(&comp_name, root.components.get(&comp_name).unwrap())?;
        regenerate_root_cmake(&root)?;

        eprintln!("Vendored GitHub repo '{}' into {} and wired to component '{}'.", repo, third, comp_name);
        return Ok(());
    }

    // vcpkg dependency path
    {
        let comp = root.components.get_mut(&comp_name).unwrap();
        if !comp.deps.iter().any(|d| d == pkg) {
            comp.deps.push(pkg.to_string());
        }
    }
    write_json_pretty_changed("triton.json", &root)?;
    write_json_pretty_changed(
        &format!("components/{comp_name}/triton.json"),
        root.components.get(&comp_name).unwrap(),
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
    rewrite_component_cmake(&comp_name, root.components.get(&comp_name).unwrap())?;
    regenerate_root_cmake(&root)?;

    eprintln!("Added '{}' to component '{}'.", pkg, comp_name);
    Ok(())
}
