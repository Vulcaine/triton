use anyhow::{Context, Result};
use crate::models::{GitDep, LinkEntry, RootDep, TritonComponent, TritonRoot};
use crate::templates::components_dir_cmakelists;
use crate::util::{read_to_string_opt, write_text_if_changed};

/// Write/refresh components/CMakeLists.txt with managed helpers + subdirs
pub fn regenerate_root_cmake(root: &TritonRoot) -> Result<()> {
    let path = "components/CMakeLists.txt";
    let mut body = String::new();
    body.push_str(&components_dir_cmakelists());

    // ---- Triton helper block (strict, using directory properties) ----
    body.push_str(r#"
# === Triton CMake helpers (simple & strict) ===
# CMake >= 3.7: use DIRECTORY properties to diff targets introduced by find_package/add_subdirectory.
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
    # Snapshot "before"
    _triton_dir_targets(_before_bs _before_imp)

    # Try config first; if that didn't mark FOUND, try module
    find_package(${pkg} CONFIG QUIET)
    if(NOT ${pkg}_FOUND)
      find_package(${pkg} QUIET)
    endif()

    # What targets did this introduce?
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

    # No targets? Likely a Find-module with variables only (e.g. Lua, ZLIB)
    string(REGEX REPLACE "[^A-Za-z0-9]" "_" _pfx "${pkg}")
    string(TOUPPER "${_pfx}" _PFX)
    _triton_make_iface_from_module_vars(_synth "${pkg}" "${_PFX}")
    if(_synth)
      target_link_libraries(${tgt} PRIVATE ${_synth})
      return()
    endif()

    message(FATAL_ERROR
"triton: could not determine a target for package '${pkg}'.
No targets appeared and no module variables like ${_PFX}_INCLUDE_DIR(S)/${_PFX}_LIBRARIES were found.
Tips:
  * Ensure the vcpkg toolchain is active (CMAKE_TOOLCHAIN_FILE -> vcpkg.cmake), and the port is installed.
  * Set VCPKG_MANIFEST_DIR to '${CMAKE_SOURCE_DIR}/..' if your vcpkg.json is in the repo root.
  * Or add an explicit mapping in triton.json.")
  endfunction()

  # Git subdir helper (minimal): link the one new target created, otherwise ask for a mapping
  function(triton_add_subdir_and_link_strict tgt path hint)
    get_filename_component(_abs "${path}" ABSOLUTE)
    _triton_dir_targets(_before_bs _before_imp)
    add_subdirectory("${_abs}" "${CMAKE_CURRENT_BINARY_DIR}/third_party" EXCLUDE_FROM_ALL)
    _triton_new_targets(_new _before_bs _before_imp)
    list(LENGTH _new _n)
    if(_n EQUAL 1)
      list(GET _new 0 _t)
      target_link_libraries(${tgt} PRIVATE ${_t})
      return()
    elseif(_n EQUAL 0)
      message(FATAL_ERROR "triton: no library targets were created by '${_abs}'. Please set 'target' for git dep '${hint}' in triton.json.")
    else()
      message(FATAL_ERROR "triton: multiple targets from '${_abs}': ${_new}
Please set 'target' for git dep '${hint}' in triton.json.")
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
    // Forward any cache vars first
    for e in &g.cmake {
        lines.push(format!("set({} {} CACHE {} \"\" FORCE)", e.var, e.val, e.typ));
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
        let (name, _pkg, maybe_tgt) = ent.normalize();
        if let Some(RootDep::Git(g)) = root.deps.iter().find(|d| matches!(d, RootDep::Git(x) if x.name == name)) {
            if let Some(t) = maybe_tgt.as_ref() {
                // Explicit override: add subdir and link the exact target
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
                p=pkg, t=tgt
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
