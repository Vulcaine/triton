use anyhow::{Context, Result};
use crate::models::{GitDep, RootDep, TritonComponent, TritonRoot};
use crate::templates::root_cmakelists;
use crate::util::{read_to_string_opt, write_text_if_changed};

pub fn regenerate_root_cmake(root: &TritonRoot) -> Result<()> {
    let mut body = String::new();
    body.push_str(&root_cmakelists(&root.app_name));
    body.push_str("\n# Subdirectories will be (re)written by triton generate\n");
    body.push_str("# ## triton:components begin\n");
    for name in root.components.keys() {
        body.push_str(&format!("add_subdirectory(components/{name})\n"));
    }
    body.push_str("\n# ## triton:components end\n");
    write_text_if_changed("CMakeLists.txt", &body).context("writing CMakeLists.txt")?;
    Ok(())
}

fn emit_git_dep(lines: &mut Vec<String>, g: &GitDep) {
    // 1) cache entries first
    for e in &g.cmake {
        lines.push(format!("set({} {} CACHE {} \"\" FORCE)", e.var, e.val, e.typ));
    }
    // 2) add_subdirectory (source and binary dirs)
    lines.push(format!(
        "add_subdirectory(\"${{PROJECT_SOURCE_DIR}}/third_party/{n}\" \
\"${{PROJECT_BINARY_DIR}}/third_party/{n}\" EXCLUDE_FROM_ALL)",
        n = g.name
    ));
    // 3) header include convenience
    lines.push(format!(
        "if(EXISTS \"${{PROJECT_SOURCE_DIR}}/third_party/{n}/include\")\n  \
target_include_directories(${{_comp_name}} PRIVATE \"${{PROJECT_SOURCE_DIR}}/third_party/{n}/include\")\nendif()",
        n = g.name
    ));
    // 4) optional target
    if let Some(t) = &g.target {
        if !t.trim().is_empty() {
            lines.push(format!("target_link_libraries(${{_comp_name}} PRIVATE {t})"));
        }
    }
    lines.push(String::new());
}

fn emit_vcpkg_dep(lines: &mut Vec<String>, name: &str, boost_bucket: &mut Vec<String>) {
    if name == "boost" || name == "boost-headers" {
        lines.push("find_package(Boost CONFIG REQUIRED)".into());
        lines.push("target_link_libraries(${_comp_name} PRIVATE Boost::headers)".into());
    } else if let Some(rest) = name.strip_prefix("boost-") {
        boost_bucket.push(rest.to_string());
    } else {
        // Leave a conservative hint for unknown libs
        lines.push(format!("# TODO(triton): add CMake for dependency '{name}'"));
    }
}

pub fn rewrite_component_cmake(name: &str, root: &TritonRoot, comp: &TritonComponent) -> Result<()> {
    let path = format!("components/{name}/CMakeLists.txt");
    let base = read_to_string_opt(&path).unwrap_or_else(|| {
        crate::templates::component_cmakelists()
    });

    let begin = "# ## triton:deps begin";
    let end = "# ## triton:deps end";
    let (pre, post) = match (base.find(begin), base.find(end)) {
        (Some(b), Some(e)) if e >= b => {
            let pre = &base[..b];
            let post = &base[(e + end.len())..];
            (pre.to_string(), post.to_string())
        }
        _ => (base, "\n".to_string()),
    };

    // For each link, find matching dep in root.deps and emit lines
    let mut lines: Vec<String> = vec![
        "# --- triton: resolve local target name ---".into(),
        "if(NOT DEFINED _comp_name)".into(),
        "  get_filename_component(_comp_name \"${CMAKE_CURRENT_SOURCE_DIR}\" NAME)".into(),
        "endif()".into(),
        "".into(),
    ];

    let mut boost_components: Vec<String> = vec![];

    for link_name in &comp.link {
        // find in root.deps by link_name
        let resolved = root.deps.iter().find(|d| match d {
            RootDep::Name(s) => s == link_name,
            RootDep::Git(g) => g.name == *link_name || g.repo == *link_name,
        });

        match resolved {
            Some(RootDep::Git(g)) => emit_git_dep(&mut lines, g),
            Some(RootDep::Name(v)) => emit_vcpkg_dep(&mut lines, v, &mut boost_components),
            None => {
                // fallback: treat as vcpkg name
                emit_vcpkg_dep(&mut lines, link_name, &mut boost_components);
            }
        }
    }

    if !boost_components.is_empty() {
        boost_components.sort();
        boost_components.dedup();
        lines.push(format!(
            "find_package(Boost CONFIG COMPONENTS {} REQUIRED)",
            boost_components.join(" ")
        ));
        for c in &boost_components {
            lines.push(format!("target_link_libraries(${{_comp_name}} PRIVATE Boost::{c})"));
        }
    }

    let mut new_body = String::new();
    new_body.push_str(&pre);
    new_body.push_str(begin);
    new_body.push('\n');
    for l in lines {
        new_body.push_str(&l);
        new_body.push('\n');
    }
    new_body.push_str(end);
    new_body.push_str(&post);

    write_text_if_changed(&path, &new_body)
        .with_context(|| format!("writing {}", path))?;
    Ok(())
}
