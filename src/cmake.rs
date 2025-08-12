use anyhow::{Context, Result};
use crate::models::{GitDep, RootDep, TritonComponent, TritonRoot};
use crate::templates::components_dir_cmakelists;
use crate::util::{read_to_string_opt, write_text_if_changed};

/// Write/refresh components/CMakeLists.txt with managed subdirs
pub fn regenerate_root_cmake(root: &TritonRoot) -> Result<()> {
    let path = "components/CMakeLists.txt";
    let mut body = String::new();
    body.push_str(&components_dir_cmakelists());
    body.push_str("\n# Subdirectories (managed)\n");
    body.push_str("# ## triton:components begin\n");
    for name in root.components.keys() {
        body.push_str(&format!("add_subdirectory({})\n", name));
    }
    body.push_str("# ## triton:components end\n");
    write_text_if_changed(path, &body).with_context(|| format!("writing {}", path))?;
    Ok(())
}

fn emit_git_dep(lines: &mut Vec<String>, g: &GitDep) {
    // Cache entries BEFORE add_subdirectory so the subproject picks them up
    for e in &g.cmake {
        lines.push(format!("set({} {} CACHE {} \"\" FORCE)", e.var, e.val, e.typ));
    }
    // Paths relative to 'components' project root
    lines.push(format!(
        "add_subdirectory(\"${{PROJECT_SOURCE_DIR}}/../third_party/{name}\" \
\"${{PROJECT_BINARY_DIR}}/third_party/{name}\" EXCLUDE_FROM_ALL)",
        name = g.name
    ));
    // Convenience include if include/ exists
    lines.push(format!(
        "if(EXISTS \"${{PROJECT_SOURCE_DIR}}/../third_party/{name}/include\")\n  \
target_include_directories(${{_comp_name}} PRIVATE \"${{PROJECT_SOURCE_DIR}}/../third_party/{name}/include\")\nendif()",
        name = g.name
    ));
    // Optional target link
    if let Some(tgt) = &g.target {
        let t = tgt.trim();
        if !t.is_empty() {
            lines.push(format!("target_link_libraries(${{_comp_name}} PRIVATE {t})"));
        }
    }
    lines.push(String::new());
}

/// Only include Git deps that this component **links** by name.
fn gen_git_dep_lines(root: &TritonRoot, comp: &TritonComponent) -> Vec<String> {
    let mut lines = vec![];
    for link_key in &comp.link {
        if let Some(RootDep::Git(g)) = root
            .deps
            .iter()
            .find(|d| matches!(d, RootDep::Git(x) if &x.name == link_key))
        {
            emit_git_dep(&mut lines, g);
        }
    }
    lines
}

/// For vcpkg name deps, emit TODO hints only for those **linked** by this component.
fn gen_vcpkg_dep_lines(root: &TritonRoot, comp: &TritonComponent) -> Vec<String> {
    let mut lines: Vec<String> = vec![];
    let vcpkg_names: Vec<&str> = root
        .deps
        .iter()
        .filter_map(|d| if let RootDep::Name(n) = d { Some(n.as_str()) } else { None })
        .collect();

    for l in &comp.link {
        if vcpkg_names.iter().any(|n| *n == l) {
            lines.push(format!(
                "# TODO(triton): add find_package/targets for vcpkg dependency '{}'",
                l
            ));
        }
    }
    lines
}

fn gen_component_link_lines(root: &TritonRoot, comp: &TritonComponent) -> Vec<String> {
    let mut lines = vec![];
    for l in &comp.link {
        // Link to other known components by name
        if root.components.contains_key(l) {
            lines.push(format!("target_link_libraries(${{_comp_name}} PRIVATE {l})"));
            continue;
        }
        // Link to a Git dep explicit target if provided
        if let Some(RootDep::Git(g)) =
            root.deps.iter().find(|d| matches!(d, RootDep::Git(x) if x.name == *l))
        {
            if let Some(tgt) = &g.target {
                if !tgt.trim().is_empty() {
                    lines.push(format!("target_link_libraries(${{_comp_name}} PRIVATE {})", tgt));
                }
            }
        }
    }
    lines
}

pub fn rewrite_component_cmake(name: &str, root: &TritonRoot, comp: &TritonComponent) -> Result<()> {
    let path = format!("components/{name}/CMakeLists.txt");
    let base = read_to_string_opt(&path).unwrap_or_else(crate::templates::component_cmakelists);

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

    // Git deps that THIS component links
    dep_lines.extend(gen_git_dep_lines(root, comp));
    if !dep_lines.is_empty() && !dep_lines.last().unwrap().is_empty() {
        dep_lines.push("".into());
    }

    // vcpkg deps for THIS component (hints)
    dep_lines.extend(gen_vcpkg_dep_lines(root, comp));
    if !dep_lines.is_empty() && !dep_lines.last().unwrap().is_empty() {
        dep_lines.push("".into());
    }

    // Link to other components or Git target names
    dep_lines.extend(gen_component_link_lines(root, comp));

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

    write_text_if_changed(&path, &new_body).with_context(|| format!("writing {}", path))?;
    Ok(())
}
