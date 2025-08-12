use anyhow::{Context, Result};
use std::fs;
use std::path::Path;

use crate::models::{TritonComponent, TritonRoot};
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

fn gen_vcpkg_dep_lines(comp: &TritonComponent) -> Vec<String> {
    // Combine Boost components into one find_package
    let mut boost_components: Vec<String> = vec![];
    let mut lines: Vec<String> = vec![];

    for d in &comp.deps {
        if d == "boost" || d == "boost-headers" {
            lines.push("find_package(Boost CONFIG REQUIRED)".into());
            lines.push("target_link_libraries(${_comp_name} PRIVATE Boost::headers)".into());
            continue;
        }
        if let Some(rest) = d.strip_prefix("boost-") {
            // map boost-filesystem -> filesystem, etc.
            boost_components.push(rest.to_string());
            continue;
        }
        // Generic hint (non-Boost): keep it conservative to avoid breaking builds
        lines.push(format!("# TODO(triton): add CMake for dependency '{}'", d));
    }

    if !boost_components.is_empty() {
        boost_components.sort();
        boost_components.dedup();
        lines.push(format!(
            "find_package(Boost CONFIG COMPONENTS {} REQUIRED)",
            boost_components.join(" ")
        ));
        for c in &boost_components {
            lines.push(format!(
                "target_link_libraries(${{_comp_name}} PRIVATE Boost::{})",
                c
            ));
        }
    }

    lines
}

fn gen_git_dep_lines(comp: &TritonComponent) -> Vec<String> {
    let mut lines = vec![];
    for g in &comp.git {
        let name = &g.name;
        lines.push(format!(
            "add_subdirectory(\"${{PROJECT_SOURCE_DIR}}/third_party/{name}\" \"${{PROJECT_BINARY_DIR}}/third_party/{name}\" EXCLUDE_FROM_ALL)"
        ));
        lines.push(format!(
            "if(EXISTS \"${{PROJECT_SOURCE_DIR}}/third_party/{name}/include\")\n  target_include_directories(${{_comp_name}} PRIVATE \"${{PROJECT_SOURCE_DIR}}/third_party/{name}/include\")\nendif()"
        ));
        if let Some(target) = &g.target {
            if !target.trim().is_empty() {
                lines.push(format!(
                    "target_link_libraries(${{_comp_name}} PRIVATE {})",
                    target
                ));
            }
        }
    }
    lines
}

pub fn rewrite_component_cmake(name: &str, comp: &TritonComponent) -> Result<()> {
    let path = format!("components/{name}/CMakeLists.txt");
    let base = read_to_string_opt(&path).unwrap_or_else(|| {
        // Minimal scaffold if file is missing
        format!(
r#"cmake_minimum_required(VERSION 3.25)

# Detect target name from directory
get_filename_component(_comp_name "${{CMAKE_CURRENT_SOURCE_DIR}}" NAME)

# If there's a main.cpp, assume executable; else library
file(GLOB_RECURSE COMP_SOURCES CONFIGURE_DEPENDS "src/*.cpp")
list(LENGTH COMP_SOURCES _src_len)
if(_src_len GREATER 0)
  add_executable(${{_comp_name}})
  target_sources(${{_comp_name}} PRIVATE ${{COMP_SOURCES}})
else()
  add_library(${{_comp_name}})
endif()

target_include_directories(${{_comp_name}} PRIVATE "include")
set_property(TARGET ${{_comp_name}} PROPERTY CXX_STANDARD 20)

# Dependencies (managed by triton)
# ## triton:deps begin
# ## triton:deps end
"#)
    });

    // Replace only the managed region
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

    let mut dep_lines = vec![
        "# --- triton: resolve local target name ---".into(),
        "if(NOT DEFINED _comp_name)".into(),
        "  get_filename_component(_comp_name \"${CMAKE_CURRENT_SOURCE_DIR}\" NAME)".into(),
        "endif()".into(),
        "".into(),
    ];
    dep_lines.extend(gen_git_dep_lines(comp));
    if !dep_lines.is_empty() && !dep_lines.last().unwrap().is_empty() {
        dep_lines.push("".into());
    }
    dep_lines.extend(gen_vcpkg_dep_lines(comp));

    let mut new_body = String::new();
    new_body.push_str(&pre);
    new_body.push_str(begin);
    new_body.push('\n');
    for l in dep_lines {
        new_body.push_str(&l);
        new_body.push('\n');
    }
    new_body.push_str(end);
    new_body.push_str(&post);

    write_text_if_changed(&path, &new_body)
        .with_context(|| format!("writing {}", path))?;
    Ok(())
}
