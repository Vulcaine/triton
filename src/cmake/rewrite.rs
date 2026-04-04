use anyhow::{Context, Result};
use std::path::Path;

use crate::models::{TritonComponent, TritonRoot};
use crate::templates::cmake_root_helpers;
use crate::util::{read_to_string_opt, write_text_if_changed};

use super::generators::*;

pub fn regenerate_root_cmake(root: &TritonRoot) -> Result<()> {
    let path = "components/CMakeLists.txt";

    // Header: includes exactly one `cmake_minimum_required()` and one `project()`
    let mut body = String::new();

    let (maj, min, pat) = super::effective_cmake_version();
    body.push_str(&format!(
        "cmake_minimum_required(VERSION {}.{}.{})\n",
        maj, min, pat
    ));
    body.push_str(&format!(
        "project({} LANGUAGES CXX)\n\n",
        root.app_name.replace('-', "_")
    ));

    // Helpers: strip any accidental prologue lines so we don't duplicate project()
    let helpers = {
        // Remove any leading cmake_minimum_required / project lines from helper text
        let s = cmake_root_helpers();
        let mut out = Vec::new();
        for line in s.lines() {
            let l = line.trim_start();
            if l.starts_with("cmake_minimum_required(") || l.starts_with("project(") {
                continue;
            }
            out.push(line);
        }
        let mut cleaned = String::new();
        let mut last_blank = false;
        for line in out {
            let is_blank = line.trim().is_empty();
            if is_blank && last_blank {
                continue;
            }
            cleaned.push_str(line);
            cleaned.push('\n');
            last_blank = is_blank;
        }
        cleaned
    };

    if !helpers.trim().is_empty() {
        body.push_str(&helpers);
        if !helpers.ends_with('\n') {
            body.push('\n');
        }
    }

    // Managed subdirs
    body.push_str("\n# Subdirectories (managed)\n# ## triton:components begin\n");

    let mut names: Vec<_> = root.components.keys().cloned().collect();
    names.sort();
    for n in names {
        let dir = Path::new("components").join(&n);
        if dir.is_dir() {
            body.push_str(&format!("add_subdirectory({n})\n"));
        }
    }

    body.push_str("# ## triton:components end\n");

    write_text_if_changed(path, &body).with_context(|| format!("writing {}", path))?;
    Ok(())
}

/// Ensure the first line is a `cmake_minimum_required(VERSION ...)` directive.
fn ensure_cmake_version_header(base: &str, cmake_ver: (u32, u32, u32)) -> String {
    let (maj, min, pat) = cmake_ver;
    let required_line = format!("cmake_minimum_required(VERSION {}.{}.{})", maj, min, pat);

    let mut lines: Vec<String> = base.lines().map(|l| l.to_string()).collect();
    if lines.is_empty() {
        lines.push(required_line.clone());
        lines.push(String::new());
    } else if lines[0].trim_start().starts_with("cmake_minimum_required") {
        lines[0] = required_line.clone();
    } else {
        lines.insert(0, required_line.clone());
        lines.insert(1, String::new());
    }
    lines.join("\n")
}

/// Normalize include-directory blocks so both exe and lib targets work correctly.
fn normalize_include_dirs(base: &str) -> String {
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

    if !base.contains("if(_is_exe)")
        && base.contains(r#"target_include_directories(${_comp_name} PRIVATE "include")"#)
    {
        base.replace(
            r#"target_include_directories(${_comp_name} PRIVATE "include")"#,
            canonical,
        )
    } else if base.contains(duplicated) {
        base.replace(duplicated, canonical)
    } else {
        base.to_string()
    }
}

/// Replace the naive TARGET_RUNTIME_DLLS copy command with one that is a no-op
/// when the generator expression expands to an empty list.
fn fix_target_runtime_dlls(base: &str) -> String {
    let old_dll_copy = r#"  add_custom_command(TARGET ${_comp_name} POST_BUILD
    COMMAND ${CMAKE_COMMAND} -E copy_if_different
      $<TARGET_RUNTIME_DLLS:${_comp_name}>
      $<TARGET_FILE_DIR:${_comp_name}>
    COMMAND_EXPAND_LISTS
  )"#;
    let new_dll_copy = r#"  add_custom_command(TARGET ${_comp_name} POST_BUILD
    COMMAND ${CMAKE_COMMAND} -E
      $<IF:$<BOOL:$<TARGET_RUNTIME_DLLS:${_comp_name}>>,copy_if_different,true>
      $<TARGET_RUNTIME_DLLS:${_comp_name}>
      $<$<BOOL:$<TARGET_RUNTIME_DLLS:${_comp_name}>>:$<TARGET_FILE_DIR:${_comp_name}>>
    COMMAND_EXPAND_LISTS
  )"#;
    if base.contains(old_dll_copy) {
        base.replace(old_dll_copy, new_dll_copy)
    } else {
        base.to_string()
    }
}

/// Build the managed dependency block that goes between the triton:deps begin/end markers.
fn generate_managed_dep_block(name: &str, root: &TritonRoot, comp: &TritonComponent) -> Vec<String> {
    let mut dep_lines: Vec<String> = vec![
        "# --- triton: resolve local target name ---".into(),
        "if(NOT DEFINED _comp_name)".into(),
        "  get_filename_component(_comp_name \"${CMAKE_CURRENT_SOURCE_DIR}\" NAME)".into(),
        "endif()".into(),
        "".into(),
    ];

    dep_lines.extend(gen_component_defines_lines(comp));
    if !dep_lines.is_empty() && !dep_lines.last().unwrap().is_empty() {
        dep_lines.push("".into());
    }

    dep_lines.extend(gen_git_dep_lines(root, name, comp));
    if !dep_lines.is_empty() && !dep_lines.last().unwrap().is_empty() {
        dep_lines.push("".into());
    }

    let vcpkg_lines = gen_vcpkg_dep_lines(root, comp, name);
    if !vcpkg_lines.is_empty() {
        dep_lines.extend(vcpkg_lines);
        if !dep_lines.last().unwrap().is_empty() {
            dep_lines.push("".into());
        }
    }

    dep_lines.extend(gen_component_link_lines(root, comp));

    let vl_lines = gen_component_vendor_libs_lines(comp);
    if !vl_lines.is_empty() {
        if !dep_lines.is_empty() && !dep_lines.last().unwrap().is_empty() {
            dep_lines.push("".into());
        }
        dep_lines.extend(vl_lines);
    }

    let lo_lines = gen_component_link_options_lines(comp);
    if !lo_lines.is_empty() {
        if !dep_lines.is_empty() && !dep_lines.last().unwrap().is_empty() {
            dep_lines.push("".into());
        }
        dep_lines.extend(lo_lines);
    }

    let res_lines = gen_component_resources_lines(comp);
    if !res_lines.is_empty() {
        if !dep_lines.is_empty() && !dep_lines.last().unwrap().is_empty() {
            dep_lines.push("".into());
        }
        dep_lines.extend(res_lines);
    }

    let asset_lines = gen_component_assets_lines(comp);
    if !asset_lines.is_empty() {
        if !dep_lines.is_empty() && !dep_lines.last().unwrap().is_empty() {
            dep_lines.push("".into());
        }
        dep_lines.extend(asset_lines);
    }

    // For exe components: copy vendor DLLs from transitive lib dependencies
    let dll_lines = gen_transitive_vendor_dll_copies(name, root, comp);
    if !dll_lines.is_empty() {
        if !dep_lines.is_empty() && !dep_lines.last().unwrap().is_empty() {
            dep_lines.push("".into());
        }
        dep_lines.push("# --- triton: copy vendor DLLs from lib dependencies ---".into());
        dep_lines.extend(dll_lines);
    }

    dep_lines
}

pub fn rewrite_component_cmake(
    name: &str,
    root: &TritonRoot,
    comp: &TritonComponent,
    cmake_ver: (u32, u32, u32),
) -> Result<()> {
    let comp_dir = Path::new("components").join(name);
    if !comp_dir.is_dir() {
        return Ok(());
    }

    let path = comp_dir.join("CMakeLists.txt");
    let path_str = path.to_string_lossy().to_string();

    // Load file or create from template
    let base = read_to_string_opt(&path_str).unwrap_or_else(|| {
        crate::templates::component_cmakelists(name.eq_ignore_ascii_case("tests"))
    });

    let base = ensure_cmake_version_header(&base, cmake_ver);
    let base_fixed = fix_target_runtime_dlls(&normalize_include_dirs(&base));

    // --- Replace triton deps block ---
    let begin = "# ## triton:deps begin";
    let end = "# ## triton:deps end";

    let (pre, post) = match (base_fixed.find(begin), base_fixed.find(end)) {
        (Some(b), Some(e)) if e > b => {
            (base_fixed[..b].to_string(), base_fixed[(e + end.len())..].to_string())
        }
        _ => (base_fixed, "\n".to_string()),
    };

    let dep_lines = generate_managed_dep_block(name, root, comp);

    let mut new_body = String::new();
    new_body.push_str(&pre);
    if !pre.ends_with('\n') {
        new_body.push('\n');
    }
    new_body.push_str(begin);
    new_body.push('\n');
    for l in dep_lines {
        new_body.push_str(&l);
        new_body.push('\n');
    }
    new_body.push_str(end);
    if !post.starts_with('\n') {
        new_body.push('\n');
    }
    new_body.push_str(&post);

    write_text_if_changed(&path_str, &new_body)
        .with_context(|| format!("writing {}", path_str))?;

    Ok(())
}
