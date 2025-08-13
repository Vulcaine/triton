use anyhow::{Context, Result};
use crate::models::{CMakeOverride, GitDep, LinkEntry, RootDep, TritonComponent, TritonRoot};
use crate::templates::{cmake_root_helpers, components_dir_cmakelists};
use crate::util::{
    cmake_quote, ensure_component_scaffold, has_link_to_name, infer_cmake_type, is_dep,
    read_to_string_opt, split_kv, write_text_if_changed,
};

/// Write/refresh components/CMakeLists.txt with helpers + subdirs (no build logic here)
pub fn regenerate_root_cmake(root: &TritonRoot) -> Result<()> {
    let path = "components/CMakeLists.txt";

    let mut body = String::new();

    // 1) header (tiny)
    body.push_str(&components_dir_cmakelists());
    body.push('\n');

    // 2) helper functions block
    body.push_str(cmake_root_helpers());
    body.push('\n');

    // 3) managed subdirectories only
    body.push_str("\n# Subdirectories (managed)\n# ## triton:components begin\n");
    let mut names: Vec<_> = root.components.keys().cloned().collect();
    names.sort();
    for n in names {
        body.push_str(&format!("add_subdirectory({n})\n"));
    }
    body.push_str("# ## triton:components end\n");

    write_text_if_changed(path, &body).with_context(|| format!("writing {}", path))?;
    Ok(())
}

fn emit_git_dep(lines: &mut Vec<String>, g: &GitDep) {
    // Forward any cache overrides first
    for ov in &g.cmake {
        match ov {
            CMakeOverride::Entry(e) => {
                let ty = if e.typ.is_empty() { "STRING" } else { e.typ.as_str() };
                let val_q = cmake_quote(&e.val);
                lines.push(format!("set({} {} CACHE {} \"\" FORCE)", e.var, val_q, ty));
            }
            CMakeOverride::KV(raw) => {
                if raw.trim().is_empty() {
                    continue;
                }
                let (var, val) = split_kv(raw);
                if var.is_empty() {
                    continue;
                }
                let ty = infer_cmake_type(&val);
                let val_q = cmake_quote(&val);
                lines.push(format!("set({} {} CACHE {} \"\" FORCE)", var, val_q, ty));
            }
        }
    }

    // Always resolve against repo root: components/.. == root/
    let prefer = g.name.as_str();
    lines.push(format!(
        "triton_add_subdir_and_link_strict(${{_comp_name}} \"${{CMAKE_SOURCE_DIR}}/../third_party/{name}\" \"{hint}\")",
        name = g.name, hint = prefer
    ));
    lines.push(String::new());
}

fn gen_git_dep_lines(root: &TritonRoot, comp: &TritonComponent) -> Vec<String> {
    let mut out = vec![];
    for ent in &comp.link {
        let (name, _pkg, maybe_first) = ent.normalize();
        let all_targets = ent.all_targets();
        if name.is_empty() {
            continue;
        }

        if let Some(RootDep::Git(g)) =
            root.deps.iter().find(|d| matches!(d, RootDep::Git(x) if x.name == name))
        {
            if !all_targets.is_empty() || maybe_first.is_some() {
                let mut req: Vec<String> = if !all_targets.is_empty() {
                    all_targets.clone()
                } else {
                    vec![maybe_first.unwrap()]
                };
                req.dedup();

                out.push(format!(
                    "set(_triton_src \"${{CMAKE_SOURCE_DIR}}/../third_party/{n}\")",
                    n = g.name
                ));
                out.push(format!(
                    "set(_triton_bin \"${{CMAKE_BINARY_DIR}}/third_party/{n}\")",
                    n = g.name
                ));
                out.push("get_property(_triton_added GLOBAL PROPERTY TRITON_ADDED_SUBDIRS)".into());
                out.push("if(NOT _triton_added)".into());
                out.push("  set(_triton_added \"\")".into());
                out.push("endif()".into());
                out.push("list(FIND _triton_added \"${_triton_src}|${_triton_bin}\" _ix)".into());
                out.push("if(_ix EQUAL -1)".into());
                out.push("  add_subdirectory(\"${_triton_src}\" \"${_triton_bin}\" EXCLUDE_FROM_ALL)".into());
                out.push(
                    "  set_property(GLOBAL PROPERTY TRITON_ADDED_SUBDIRS \"${_triton_added};${_triton_src}|${_triton_bin}\")"
                        .into(),
                );
                out.push("endif()".into());

                for t in req {
                    out.push(format!(
                        "if(TARGET {t})
  if(NOT TARGET triton::{dep}::{t})
    add_library(triton::{dep}::{t} ALIAS {t})
  endif()
  target_link_libraries(${{_comp_name}} PRIVATE triton::{dep}::{t})
else()
  message(FATAL_ERROR \"git dep '{dep}' present but target '{t}' not found\")
endif()",
                        dep = name,
                        t = t
                    ));
                }
                out.push(String::new());
            } else {
                emit_git_dep(&mut out, g);
            }
        }
    }
    out
}

fn gen_vcpkg_dep_lines(root: &TritonRoot, comp: &TritonComponent) -> Vec<String> {
    let vcpkg_names: Vec<&str> = root
        .deps
        .iter()
        .filter_map(|d| if let RootDep::Name(n) = d { Some(n.as_str()) } else { None })
        .collect();

    let mut lines: Vec<String> = vec![];
    for ent in &comp.link {
        let (name, pkg_hint, tgt_hint) = ent.normalize();
        if !vcpkg_names.iter().any(|n| *n == name) {
            continue;
        }

        let explicit_targets = ent.all_targets();

        if let Some(pkg) = pkg_hint.clone() {
            lines.push(format!("find_package({} CONFIG REQUIRED)", pkg));
            let req: Vec<String> = if !explicit_targets.is_empty() {
                explicit_targets
            } else if let Some(t) = tgt_hint.clone() {
                vec![t]
            } else {
                vec![]
            };

            if req.is_empty() {
                lines.push(format!("# vcpkg: {}", name));
                lines.push(format!(
                    "triton_find_vcpkg_and_link_strict(${{_comp_name}} \"{}\")",
                    name
                ));
                lines.push(String::new());
            } else {
                for t in req {
                    lines.push(format!(
                        "if(TARGET {t})
  target_link_libraries(${{_comp_name}} PRIVATE {t})
else()
  message(FATAL_ERROR \"Package '{p}' found but target '{t}' not defined.\")
endif()",
                        p = pkg,
                        t = t
                    ));
                }
                lines.push(String::new());
            }
        } else {
            lines.push(format!("# vcpkg: {}", name));
            lines.push(format!(
                "triton_find_vcpkg_and_link_strict(${{_comp_name}} \"{}\")",
                name
            ));
            lines.push(String::new());
        }
    }
    lines
}

fn gen_component_link_lines(root: &TritonRoot, comp: &TritonComponent) -> Vec<String> {
    let mut lines = vec![];
    for ent in &comp.link {
        let (name, _pkg, _tgt) = ent.normalize();
        if root.components.contains_key(&name) {
            lines.push(format!(
                "target_link_libraries(${{_comp_name}} PRIVATE {name})"
            ));
            lines.push(format!(
                "if(EXISTS \"${{CMAKE_SOURCE_DIR}}/{n}/include\")
  target_include_directories(${{_comp_name}} PRIVATE \"${{CMAKE_SOURCE_DIR}}/{n}/include\")
endif()",
                n = name
            ));
        }
    }
    lines
}

fn gen_component_defines_lines(comp: &TritonComponent) -> Vec<String> {
    if comp.defines.is_empty() {
        return vec![];
    }
    let mut parts: Vec<String> = Vec::new();
    for d in &comp.defines {
        if d.trim().is_empty() {
            continue;
        }
        parts.push(cmake_quote(d));
    }
    if parts.is_empty() {
        return vec![];
    }
    vec![format!(
        "target_compile_definitions(${{_comp_name}} PRIVATE {})",
        parts.join(" ")
    )]
}

pub fn rewrite_component_cmake(name: &str, root: &TritonRoot, comp: &TritonComponent) -> Result<()> {
    let path = format!("components/{name}/CMakeLists.txt");
    let base_raw = read_to_string_opt(&path).unwrap_or_else(crate::templates::component_cmakelists);

    // Fix up include visibility: exe -> PRIVATE, lib -> PUBLIC
    let canonical = r#"if(_is_exe)
  target_include_directories(${_comp_name} PRIVATE "include")
else()
  target_include_directories(${_comp_name} PUBLIC "include")
endif()"#;

    let duplicated = r#"if(_is_exe)
  if(_is_exe)
  target_include_directories(${_comp_name} PRIVATE "include")
else()
  target_include_directories(${_comp_name} PUBLIC "include")
endif()
else()
  target_include_directories(${_comp_name} PUBLIC "include")
endif()"#;

    let mut base = base_raw.clone();
    let had_conditional = base_raw.contains("if(_is_exe)");
    if !had_conditional
        && base_raw.contains(r#"target_include_directories(${_comp_name} PRIVATE "include")"#)
    {
        base = base_raw.replace(
            r#"target_include_directories(${_comp_name} PRIVATE "include")"#,
            canonical,
        );
    } else if base_raw.contains(duplicated) {
        base = base_raw.replace(duplicated, canonical);
    }

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

    // Component-local compile definitions
    dep_lines.extend(gen_component_defines_lines(comp));
    if !dep_lines.is_empty() && !dep_lines.last().unwrap().is_empty() {
        dep_lines.push("".into());
    }

    // Deps
    dep_lines.extend(gen_git_dep_lines(root, comp));
    if !dep_lines.is_empty() && !dep_lines.last().unwrap().is_empty() {
        dep_lines.push("".into());
    }

    let vcpkg_lines = gen_vcpkg_dep_lines(root, comp);
    if !vcpkg_lines.is_empty() {
        dep_lines.extend(vcpkg_lines);
        if !dep_lines.last().unwrap().is_empty() {
            dep_lines.push("".into());
        }
    }

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
