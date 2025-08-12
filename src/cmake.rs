use anyhow::{Context, Result};
use crate::models::{GitDep, LinkEntry, RootDep, TritonComponent, TritonRoot};
use crate::templates::components_dir_cmakelists;
use crate::util::{read_to_string_opt, write_text_if_changed};

/// Write/refresh components/CMakeLists.txt with managed helpers + subdirs
pub fn regenerate_root_cmake(root: &TritonRoot) -> Result<()> {
    let path = "components/CMakeLists.txt";

    let mut body = String::new();
    body.push_str(&components_dir_cmakelists());

    // ---- Triton helper block (strict, generic) ----
    body.push_str(r#"
# === Triton CMake helpers (strict discovery, no hardcoded targets) ===
if(NOT COMMAND triton_find_vcpkg_and_link_strict)
  function(triton__is_lib OUT TGT)
    get_target_property(_type ${TGT} TYPE)
    if(_type STREQUAL "INTERFACE_LIBRARY" OR _type STREQUAL "STATIC_LIBRARY" OR _type STREQUAL "SHARED_LIBRARY" OR _type STREQUAL "MODULE_LIBRARY")
      set(${OUT} TRUE PARENT_SCOPE)
    else()
      set(${OUT} FALSE PARENT_SCOPE)
    endif()
  endfunction()

  function(triton__list_new_lib_targets OUT BEFORE_VAR)
    get_property(_after GLOBAL PROPERTY TARGETS)
    set(_new "")
    foreach(t IN LISTS _after)
      list(FIND ${BEFORE_VAR} "${t}" _idx)
      if(_idx EQUAL -1)
        triton__is_lib(_ok ${t})
        if(_ok)
          list(APPEND _new "${t}")
        endif()
      endif()
    endforeach()
    set(${OUT} "${_new}" PARENT_SCOPE)
  endfunction()

  function(triton__collect_lib_targets_by_hint OUT HINT)
    string(TOLOWER "${HINT}" _h)
    get_property(_all GLOBAL PROPERTY TARGETS)
    set(_hit "")
    foreach(t IN LISTS _all)
      triton__is_lib(_ok ${t})
      if(NOT _ok)
        continue()
      endif()
      string(TOLOWER "${t}" _tl)
      if(_tl MATCHES "::${_h}($|::)" OR _tl MATCHES "(^|[^A-Za-z0-9])${_h}($|[^A-Za-z0-9])")
        list(APPEND _hit "${t}")
      endif()
    endforeach()
    list(REMOVE_DUPLICATES _hit)
    set(${OUT} "${_hit}" PARENT_SCOPE)
  endfunction()

  function(triton__canon_pkg_names OUT PKG)
    # Derive candidate case variants + names found in vcpkg share/<port>/*Config.cmake
    string(TOLOWER "${PKG}" _lp)
    string(TOUPPER "${PKG}" _up)
    # CamelCase
    string(REGEX REPLACE "[^A-Za-z0-9]+" ";" _parts "${PKG}")
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
    set(names "${PKG};${_lp};${_up};${_cm}")

    # Also peek vcpkg installed dir for *Config.cmake file stems
    if(DEFINED VCPKG_INSTALLED_DIR AND DEFINED VCPKG_TARGET_TRIPLET)
      set(_share "${VCPKG_INSTALLED_DIR}/${VCPKG_TARGET_TRIPLET}/share/${_lp}")
      if(EXISTS "${_share}")
        file(GLOB _cfgs "${_share}/*Config.cmake")
        foreach(f IN LISTS _cfgs)
          get_filename_component(_fn "${f}" NAME_WE)
          string(REGEX REPLACE "Config$" "" _base "${_fn}")
          list(APPEND names "${_base}")
        endforeach()
      endif()
    endif()
    list(REMOVE_DUPLICATES names)
    set(${OUT} "${names}" PARENT_SCOPE)
  endfunction()

  function(triton_find_vcpkg_and_link_strict tgt pkg)
    triton__canon_pkg_names(_names "${pkg}")
    get_property(_before GLOBAL PROPERTY TARGETS)

    # Try CONFIG mode first, then module mode (in case a Find<Pkg>.cmake exists)
    foreach(n IN LISTS _names)
      if(NOT n STREQUAL "")
        find_package(${n} CONFIG QUIET)
        if(NOT ${n}_FOUND)
          find_package(${n} QUIET)
        endif()
      endif()
    endforeach()

    # if exactly 1 NEW lib target appeared, link it
    triton__list_new_lib_targets(_new _before)
    list(LENGTH _new _cnt)
    if(_cnt EQUAL 1)
      list(GET _new 0 _t)
      target_link_libraries(${tgt} PRIVATE ${_t})
      return()
    elseif(_cnt GREATER 1)
      message(FATAL_ERROR
"triton: multiple library targets were introduced for package '${pkg}':
  ${_new}
Please specify an explicit mapping in triton.json, e.g.
  { \"name\": \"${pkg}\", \"package\": \"<CMakePackage>\", \"target\": \"<CMakeTarget>\" }
or (shorthand)
  { \"${pkg}\": { \"package\": \"<CMakePackage>\", \"target\": \"<CMakeTarget>\" } }")
    endif()

    # No NEW targets (maybe already found): try to select a unique existing match by hint
    triton__collect_lib_targets_by_hint(_cand "${pkg}")
    list(LENGTH _cand _ecnt)
    if(_ecnt EQUAL 1)
      list(GET _cand 0 _t)
      target_link_libraries(${tgt} PRIVATE ${_t})
      return()
    endif()

    # Helpful diagnostics when nothing matches
    message(FATAL_ERROR
"triton: could not determine a target for vcpkg package '${pkg}'.
No matching library targets were found after find_package().
Tips:
  * Ensure vcpkg integration is enabled (CMAKE_TOOLCHAIN_FILE points to vcpkg.cmake).
  * Ensure VCPKG_MANIFEST_DIR points to your repo root (often: '${CMAKE_SOURCE_DIR}/..').
  * Confirm the port is installed for the current triplet.
  * Otherwise add an explicit mapping in triton.json:
    { \"name\": \"${pkg}\", \"package\": \"<CMakePackage>\", \"target\": \"<CMakeTarget>\" }")
  endfunction()

  function(triton_add_subdir_and_link_strict tgt path hint)
    get_filename_component(_dir "${path}" NAME)
    get_property(_before GLOBAL PROPERTY TARGETS)
    add_subdirectory("${path}" "${CMAKE_CURRENT_BINARY_DIR}/third_party/${_dir}" EXCLUDE_FROM_ALL)
    triton__list_new_lib_targets(_new _before)
    list(LENGTH _new _cnt)
    if(_cnt EQUAL 1)
      list(GET _new 0 _t)
      target_link_libraries(${tgt} PRIVATE ${_t})
    elseif(_cnt EQUAL 0)
      message(FATAL_ERROR
"triton: no library targets were created by '${path}'.
Please specify which to link via 'target' in triton.json for git dep '${hint}'.")
    else()
      message(FATAL_ERROR
"triton: multiple library targets were created by '${path}':
  ${_new}
Please set the 'target' for git dep '${hint}' in triton.json.")
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
    for e in &g.cmake {
        lines.push(format!("set({} {} CACHE {} \"\" FORCE)", e.var, e.val, e.typ));
    }
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
        let (name, _pkg, maybe_tgt) = ent.normalize();
        if let Some(RootDep::Git(g)) = root.deps.iter().find(|d| matches!(d, RootDep::Git(x) if x.name == name)) {
            if let Some(t) = maybe_tgt.as_ref() {
                // If link entry overrides target, prefer that explicit link after subdir
                out.push(format!(
                    "add_subdirectory(\"${{CMAKE_SOURCE_DIR}}/../third_party/{n}\" \"${{CMAKE_BINARY_DIR}}/third_party/{n}\" EXCLUDE_FROM_ALL)",
                    n = g.name
                ));
                out.push(format!(
                    "if(TARGET {t})\n  target_link_libraries(${{_comp_name}} PRIVATE {t})\nelse()\n  message(FATAL_ERROR \"git dep '{n}' present but target '{t}' not found\")\nendif()",
                    n = g.name, t = t
                ));
                out.push(String::new());
            } else {
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

        if let (Some(pkg), Some(tgt)) = (pkg_hint.clone(), tgt_hint.clone()) {
            // Explicit mapping: REQUIRED + exact target name
            lines.push(format!("find_package({} CONFIG REQUIRED)", pkg));
            lines.push(format!(
                "if(TARGET {t})\n  target_link_libraries(${{_comp_name}} PRIVATE {t})\nelse()\n  message(FATAL_ERROR \"Package '{p}' found but target '{t}' not defined.\")\nendif()",
                p = pkg, t = tgt
            ));
            lines.push(String::new());
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
            lines.push(format!("target_link_libraries(${{_comp_name}} PRIVATE {name})"));
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
        "  get_filenamecomponent(_comp_name \"${CMAKE_CURRENT_SOURCE_DIR}\" NAME)".into(),
        "endif()".into(),
        "".into(),
    ];

    // Fix typo if any: ensure correct command name
    dep_lines[2] = "  get_filename_component(_comp_name \"${CMAKE_CURRENT_SOURCE_DIR}\" NAME)".into();

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
