use anyhow::{Context, Result};
use std::collections::HashSet;

use crate::models::{GitDep, RootDep, TritonComponent, TritonRoot};
use crate::templates::components_dir_cmakelists;
use crate::util::{read_to_string_opt, write_text_if_changed};

/// Write/refresh components/CMakeLists.txt with managed subdirs
pub fn regenerate_root_cmake(root: &TritonRoot) -> Result<()> {
    let path = "components/CMakeLists.txt";

    let mut body = String::new();
    body.push_str(&components_dir_cmakelists());

    // ---- Triton helper block (once) ----
    body.push_str(r#"
# === Triton CMake helpers ===
if(NOT COMMAND triton_try_link_first_new_library_target)
  function(triton_try_link_first_new_library_target tgt before_list_var)
    # Link the first NEW library target introduced since BEFORE list
    get_property(_after GLOBAL PROPERTY TARGETS)
    set(_linked FALSE)
    foreach(t IN LISTS _after)
      list(FIND ${before_list_var} "${t}" _idx)
      if(_idx EQUAL -1)
        get_target_property(_type ${t} TYPE)
        if(_type STREQUAL "STATIC_LIBRARY" OR _type STREQUAL "SHARED_LIBRARY" OR _type STREQUAL "INTERFACE_LIBRARY")
          target_link_libraries(${tgt} PRIVATE ${t})
          set(_linked TRUE)
          break()
        endif()
      endif()
    endforeach()
    set(_TRITON_LINKED ${_linked} PARENT_SCOPE)
  endfunction()

  function(triton_find_vcpkg_and_link tgt pkg)
    # Try multiple casings; diff targets before/after; link the best one.
    string(TOLOWER "${pkg}" _lp)
    string(TOUPPER "${pkg}" _up)

    # CamelCase
    set(_cm "${pkg}")
    string(REGEX REPLACE "[^A-Za-z0-9]+" ";" _parts "${pkg}")
    if(_parts)
      set(_cm "")
      foreach(p IN LISTS _parts)
        if(p STREQUAL "") 
          continue()
        endif()
        string(SUBSTRING "${p}" 0 1 _c1)
        string(TOUPPER "${_c1}" _c1)
        string(SUBSTRING "${p}" 1 -1 _rest)
        string(TOLOWER "${_rest}" _rest)
        set(_cm "${_cm}${_c1}${_rest}")
      endforeach()
    endif()

    get_property(_before GLOBAL PROPERTY TARGETS)
    foreach(_name IN ITEMS "${pkg}" "${_lp}" "${_up}" "${_cm}")
      if(NOT _name STREQUAL "")
        find_package(${_name} CONFIG QUIET)
      endif()
    endforeach()

    # Prefer canonical imported names, else fall back to first new lib target
    foreach(_cand IN ITEMS "${pkg}::${pkg}" "${_lp}::${_lp}" "${_up}::${_up}" "${_cm}::${_cm}")
      if(TARGET ${_cand})
        target_link_libraries(${tgt} PRIVATE ${_cand})
        return()
      endif()
    endforeach()

    triton_try_link_first_new_library_target(${tgt} _before)
  endfunction()

  function(triton_add_subdir_and_link tgt path prefer_name)
    # Add subdir; link prefer_name if it exists; else link first new lib target.
    get_filename_component(_dir "${path}" NAME)
    get_property(_before GLOBAL PROPERTY TARGETS)
    add_subdirectory("${path}" "${CMAKE_CURRENT_BINARY_DIR}/third_party/${_dir}" EXCLUDE_FROM_ALL)

    if(NOT "${prefer_name}" STREQUAL "")
      if(TARGET ${prefer_name})
        target_link_libraries(${tgt} PRIVATE ${prefer_name})
        return()
      endif()
      set(_ns "${prefer_name}::${prefer_name}")
      if(TARGET ${_ns})
        target_link_libraries(${tgt} PRIVATE ${_ns})
        return()
      endif()
    endif()

    triton_try_link_first_new_library_target(${tgt} _before)
  endfunction()
endif()
"#);

    // ---- Managed subdirectories ----
    body.push_str("\n# Subdirectories (managed)\n");
    body.push_str("# ## triton:components begin\n");

    let mut names: Vec<_> = root.components.keys().cloned().collect();
    names.sort();
    for name in names {
        body.push_str(&format!("add_subdirectory({})\n", name));
    }
    body.push_str("# ## triton:components end\n");

    write_text_if_changed(path, &body).with_context(|| format!("writing {}", path))?;
    Ok(())
}

fn emit_git_dep(lines: &mut Vec<String>, g: &GitDep) {
    for e in &g.cmake {
        lines.push(format!("set({} {} CACHE {} \"\" FORCE)", e.var, e.val, e.typ));
    }

    // Add the subdir and auto-link best new library target (or explicit g.target if you set it)
    let prefer = g.target.as_deref().unwrap_or(&g.name);
    lines.push(format!(
        "triton_add_subdir_and_link(${{_comp_name}} \"${{PROJECT_SOURCE_DIR}}/../third_party/{name}\" \"{prefer}\")",
        name = g.name, prefer = prefer
    ));

    // (Optional) extra include roots for projects that don’t propagate include dirs correctly:
    lines.push("set(_triton_git_inc_roots".into());
    lines.push(format!("  \"${{PROJECT_SOURCE_DIR}}/../third_party/{}/include\"", g.name));
    lines.push(format!("  \"${{PROJECT_SOURCE_DIR}}/../third_party/{}/filament/include\"", g.name));
    lines.push(format!("  \"${{PROJECT_SOURCE_DIR}}/../third_party/{}/libs/filament/include\"", g.name));
    lines.push(")".into());
    lines.push("foreach(_inc ${_triton_git_inc_roots})".into());
    lines.push("  if(EXISTS ${_inc})".into());
    lines.push("    target_include_directories(${_comp_name} PRIVATE ${_inc})".into());
    lines.push("  endif()".into());
    lines.push("endforeach()".into());
    lines.push("unset(_triton_git_inc_roots)".into());

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

/// Build a unique-in-order vector.
fn uniq_in_order(items: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut seen = HashSet::<String>::new();
    let mut out = Vec::new();
    for s in items {
        if seen.insert(s.clone()) {
            out.push(s);
        }
    }
    out
}

fn gen_vcpkg_dep_lines(root: &TritonRoot, comp: &TritonComponent) -> Vec<String> {
    let vcpkg_names: Vec<&str> = root.deps.iter().filter_map(|d| {
        if let RootDep::Name(n) = d { Some(n.as_str()) } else { None }
    }).collect();

    let mut lines: Vec<String> = vec![];
    for l in &comp.link {
        if vcpkg_names.iter().any(|n| *n == l) {
            lines.push(format!("# vcpkg: {}", l));
            lines.push(format!("triton_find_vcpkg_and_link(${{_comp_name}} \"{}\")", l));
            lines.push(String::new());
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

    // vcpkg deps for THIS component (quiet, deduped)
    let vcpkg_lines = gen_vcpkg_dep_lines(root, comp);
    if !vcpkg_lines.is_empty() {
        dep_lines.extend(vcpkg_lines);
        if !dep_lines.last().unwrap().is_empty() {
            dep_lines.push("".into());
        }
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
