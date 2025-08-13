use anyhow::{Context, Result};
use crate::models::{GitDep, CMakeOverride, RootDep, TritonComponent, TritonRoot};
use crate::templates::components_dir_cmakelists;
use crate::util::{read_to_string_opt, write_text_if_changed};

fn cmake_quote(val: &str) -> String {
    // Quote and escape internal quotes for safe CMake
    let mut s = val.replace('"', "\\\"");
    format!("\"{}\"", s)
}

fn infer_cmake_type(val: &str) -> &'static str {
    match val.to_ascii_uppercase().as_str() {
        "ON" | "OFF" | "TRUE" | "FALSE" | "YES" | "NO" => "BOOL",
        _ => "STRING",
    }
}

fn split_kv(raw: &str) -> (String, String) {
    if let Some(idx) = raw.find('=') {
        let (k, v) = raw.split_at(idx);
        let mut key = k.trim().to_string();
        let mut val = v[1..].trim().to_string();
        // Strip surrounding quotes if user wrote VAR="VALUE"
        if val.starts_with('"') && val.ends_with('"') && val.len() >= 2 {
            val = val[1..val.len()-1].to_string();
        }
        if key.is_empty() { key = raw.trim().to_string(); }
        if val.is_empty() { val = "ON".to_string(); } // sane default
        (key, val)
    } else {
        // No '=' provided -> treat as boolean ON
        (raw.trim().to_string(), "ON".to_string())
    }
}

/// Write/refresh components/CMakeLists.txt with managed helpers + subdirs
pub fn regenerate_root_cmake(root: &TritonRoot) -> Result<()> {
    let path = "components/CMakeLists.txt";

    let mut body = String::new();
    body.push_str(&components_dir_cmakelists());

    // ---- Triton helper block (simple & strict; de-duplicates add_subdirectory) ----
    body.push_str(r#"
# === Triton CMake helpers (simple & strict) ===
# CMake >= 3.7: use DIRECTORY properties to diff targets introduced by find_package/add_subdirectory.
# Also track a GLOBAL list (TRITON_ADDED_SUBDIRS) to avoid re-adding the same git subdir.
if(NOT COMMAND triton_find_vcpkg_and_link_strict)

  # Snapshot directory-scope targets
  function(_triton_dir_targets OUT_BS OUT_IMP)
    get_directory_property(_bs BUILDSYSTEM_TARGETS)
    get_directory_property(_imp IMPORTED_TARGETS)
    if(NOT _bs)
      set(_bs "")
    endif()
    if(NOT _imp)
      set(_imp "")
    endif()
    set(${OUT_BS} "${_bs}" PARENT_SCOPE)
    set(${OUT_IMP} "${_imp}" PARENT_SCOPE)
  endfunction()

  # Compute "new" targets since BEFORE_* snapshots (both buildsystem + imported)
  function(_triton_new_targets OUT BEFORE_BS BEFORE_IMP)
    _triton_dir_targets(_after_bs _after_imp)
    set(_new "")
    foreach(t IN LISTS _after_bs)
      list(FIND ${BEFORE_BS} "${t}" _ix)
      if(_ix EQUAL -1)
        list(APPEND _new "${t}")
      endif()
    endforeach()
    foreach(t IN LISTS _after_imp)
      list(FIND ${BEFORE_IMP} "${t}" _ix)
      if(_ix EQUAL -1)
        list(APPEND _new "${t}")
      endif()
    endforeach()
    list(REMOVE_DUPLICATES _new)
    set(${OUT} "${_new}" PARENT_SCOPE)
  endfunction()

  # Build INTERFACE target from typical Find-module variables (<PFX>_INCLUDE_DIR(S), <PFX>_LIBRARIES)
  function(_triton_make_iface_from_module_vars OUT PKG VAR_PREFIX)
    set(_incs "")
    foreach(v IN ITEMS "${VAR_PREFIX}_INCLUDE_DIRS" "${VAR_PREFIX}_INCLUDE_DIR")
      if(DEFINED ${v})
        set(_incs ${${v}})
        break()
      endif()
    endforeach()
    set(_libs "")
    foreach(v IN ITEMS "${VAR_PREFIX}_LIBRARIES" "${VAR_PREFIX}_LIBRARY")
      if(DEFINED ${v})
        set(_libs ${${v}})
        break()
      endif()
    endforeach()

    if(_incs OR _libs)
      set(_tgt "triton::${PKG}")
      if(NOT TARGET ${_tgt})
        add_library(${_tgt} INTERFACE)
        if(_incs)
          target_include_directories(${_tgt} INTERFACE ${_incs})
        endif()
        if(_libs)
          target_link_libraries(${_tgt} INTERFACE ${_libs})
        endif()
      endif()
      set(${OUT} ${_tgt} PARENT_SCOPE)
    else()
      set(${OUT} "" PARENT_SCOPE)
    endif()
  endfunction()

  # One function to rule them all
  function(triton_find_vcpkg_and_link_strict tgt pkg)
    _triton_dir_targets(_before_bs _before_imp)

    find_package(${pkg} CONFIG QUIET)
    if(NOT ${pkg}_FOUND)
      find_package(${pkg} QUIET)
    endif()

    _triton_new_targets(_new _before_bs _before_imp)
    list(LENGTH _new _n)
    if(_n EQUAL 1)
      list(GET _new 0 _t)
      target_link_libraries(${tgt} PRIVATE ${_t})
      return()
    elseif(_n GREATER 1)
      message(FATAL_ERROR
"triton: multiple targets introduced by package '${pkg}':
  ${_new}
Please specify an explicit mapping in triton.json: { \"name\": \"${pkg}\", \"package\": \"<Pkg>\", \"target\": \"<Pkg::Target>\" }")
    endif()

    string(REGEX REPLACE "[^A-Za-z0-9]" "_" _pfx "${pkg}")
    string(TOUPPER "${_pfx}" _PFX)
    _triton_make_iface_from_module_vars(_synth "${pkg}" "${_PFX}")
    if(_synth)
      target_link_libraries(${tgt} PRIVATE ${_synth})
      return()
    endif()

    message(FATAL_ERROR
"triton: could not determine a target for package '${pkg}'.
No targets appeared and no module variables like ${_PFX}_INCLUDE_DIR(S)/${_PFX}_LIBRARIES were found.")
  endfunction()

  function(triton_add_subdir_and_link_strict tgt path hint)
    get_filename_component(_abs "${path}" ABSOLUTE)
    get_filename_component(_dir "${_abs}" NAME)
    set(_bin "${CMAKE_BINARY_DIR}/third_party/${_dir}")

    get_property(_added GLOBAL PROPERTY TRITON_ADDED_SUBDIRS)
    if(NOT _added)
      set(_added "")
    endif()
    list(FIND _added "${_abs}|${_bin}" _ix)

    _triton_dir_targets(_before_bs _before_imp)
    if(_ix EQUAL -1)
      add_subdirectory("${_abs}" "${_bin}" EXCLUDE_FROM_ALL)
      set_property(GLOBAL PROPERTY TRITON_ADDED_SUBDIRS "${_added};${_abs}|${_bin}")
    endif()

    _triton_new_targets(_new _before_bs _before_imp)
    list(LENGTH _new _cnt)
    if(_cnt EQUAL 1)
      list(GET _new 0 _t)
      target_link_libraries(${tgt} PRIVATE ${_t})
      return()
    elseif(_cnt GREATER 1)
      message(FATAL_ERROR
"triton: multiple library targets were created by '${_abs}':
  ${_new}
Please set the 'target' for git dep '${hint}' in triton.json.")
    endif()

    get_property(_all GLOBAL PROPERTY TARGETS)
    set(_cand "")
    foreach(t IN LISTS _all)
      if(TARGET ${t})
        get_target_property(_src ${t} SOURCE_DIR)
        if(_src AND _src STREQUAL _abs)
          list(APPEND _cand "${t}")
        endif()
      endif()
    endforeach()
    list(REMOVE_DUPLICATES _cand)
    list(LENGTH _cand _c)
    if(_c EQUAL 0)
      message(FATAL_ERROR
"triton: no library targets were created by '${_abs}'.")
    elseif(_c EQUAL 1)
      list(GET _cand 0 _t)
      target_link_libraries(${tgt} PRIVATE ${_t})
      return()
    else()
      string(TOLOWER "${hint}" _h)
      set(_both "")
      foreach(t IN LISTS _cand)
        string(TOLOWER "${t}" _tl)
        if(_tl MATCHES "::${_h}($|::)" OR _tl MATCHES "(^|[^A-Za-z0-9])${_h}($|[^A-Za-z0-9])")
          list(APPEND _both "${t}")
        endif()
      endforeach()
      list(REMOVE_DUPLICATES _both)
      list(LENGTH _both _bh)
      if(_bh EQUAL 1)
        list(GET _both 0 _t)
        target_link_libraries(${tgt} PRIVATE ${_t})
        return()
      endif()
      message(FATAL_ERROR
"triton: multiple library targets live under '${_abs}':
  ${_cand}")
    endif()
  endfunction()

endif()
"#);

    // ---- Managed subdirectories ----
    body.push_str("\n# Subdirectories (managed)\n# ## triton:components begin\n");
    let mut names: Vec<_> = root.components.keys().cloned().collect();
    names.sort();
    for n in names { body.push_str(&format!("add_subdirectory({n})\n")); }
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
                if raw.trim().is_empty() { continue; }
                let (var, val) = split_kv(raw);
                if var.is_empty() { continue; }
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
        // We need both the name and potentially multiple explicit targets
        let (name, _pkg, maybe_first) = ent.normalize();
        let all_targets = ent.all_targets();
        if name.is_empty() { continue; }

        if let Some(RootDep::Git(g)) = root.deps.iter().find(|d| matches!(d, RootDep::Git(x) if x.name == name)) {
            if !all_targets.is_empty() || maybe_first.is_some() {
                // Explicit mapping: possibly multiple targets
                let mut req: Vec<String> = if !all_targets.is_empty() {
                    all_targets.clone()
                } else {
                    vec![maybe_first.unwrap()]
                };

                // Dedup while preserving order
                req.dedup();

                // Guarded, deduped add_subdirectory (GLOBAL)
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
                out.push("  set_property(GLOBAL PROPERTY TRITON_ADDED_SUBDIRS \"${_triton_added};${_triton_src}|${_triton_bin}\")".into());
                out.push("endif()".into());

                // For each requested target: create a unique alias and link it
                for t in req {
                    // Alias: triton::<dep>::<t>
                    out.push(format!(
                        "if(TARGET {t})\n  if(NOT TARGET triton::{dep}::{t})\n    add_library(triton::{dep}::{t} ALIAS {t})\n  endif()\n  target_link_libraries(${{_comp_name}} PRIVATE triton::{dep}::{t})\nelse()\n  message(FATAL_ERROR \"git dep '{dep}' present but target '{t}' not found\")\nendif()",
                        dep = name, t = t
                    ));
                }
                out.push(String::new());
            } else {
                // No explicit targets -> use strict auto-detection (single target)
                emit_git_dep(&mut out, g);
            }
        }
    }
    out
}

fn gen_vcpkg_dep_lines(root: &TritonRoot, comp: &TritonComponent) -> Vec<String> {
    let vcpkg_names: Vec<&str> = root.deps.iter().filter_map(|d| {
        if let RootDep::Name(n) = d { Some(n.as_str()) } else { None }
    }).collect();

    let mut lines: Vec<String> = vec![];
    for ent in &comp.link {
        let (name, pkg_hint, tgt_hint) = ent.normalize();
        if !vcpkg_names.iter().any(|n| *n == name) { continue; }

        let explicit_targets = ent.all_targets();

        if let Some(pkg) = pkg_hint.clone() {
            // Explicit mapping: REQUIRED + one or many imported targets
            lines.push(format!("find_package({} CONFIG REQUIRED)", pkg));

            let req: Vec<String> = if !explicit_targets.is_empty() {
                explicit_targets
            } else if let Some(t) = tgt_hint.clone() {
                vec![t]
            } else {
                vec![]
            };

            if req.is_empty() {
                // No explicit targets given -> fall back to strict auto discover
                lines.push(format!("# vcpkg: {}", name));
                lines.push(format!("triton_find_vcpkg_and_link_strict(${{_comp_name}} \"{}\")", name));
                lines.push(String::new());
            } else {
                for t in req {
                    lines.push(format!(
                        "if(TARGET {t})\n  target_link_libraries(${{_comp_name}} PRIVATE {t})\nelse()\n  message(FATAL_ERROR \"Package '{p}' found but target '{t}' not defined.\")\nendif()",
                        p=pkg, t=t
                    ));
                }
                lines.push(String::new());
            }
        } else {
            // Strict discovery with helpful failure
            lines.push(format!("# vcpkg: {}", name));
            lines.push(format!("triton_find_vcpkg_and_link_strict(${{_comp_name}} \"{}\")", name));
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
            // 1) Link the sibling component target
            lines.push(format!("target_link_libraries(${{_comp_name}} PRIVATE {name})"));

            // 2) Safety fallback: also add the sibling's include/ if it exists.
            //    This covers older components whose template used PRIVATE includes,
            //    and guarantees headers are visible even if the producer forgot PUBLIC.
            lines.push(format!(
                "if(EXISTS \"${{CMAKE_SOURCE_DIR}}/{n}/include\")\n  target_include_directories(${{_comp_name}} PRIVATE \"${{CMAKE_SOURCE_DIR}}/{n}/include\")\nendif()",
                n = name
            ));
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

    dep_lines.extend(gen_git_dep_lines(root, comp));
    if !dep_lines.is_empty() && !dep_lines.last().unwrap().is_empty() { dep_lines.push("".into()); }

    let vcpkg_lines = gen_vcpkg_dep_lines(root, comp);
    if !vcpkg_lines.is_empty() {
        dep_lines.extend(vcpkg_lines);
        if !dep_lines.last().unwrap().is_empty() { dep_lines.push("".into()); }
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
