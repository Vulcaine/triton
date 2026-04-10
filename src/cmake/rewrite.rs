use anyhow::{Context, Result};
use std::path::Path;

use super::languages::{detect_component_languages_in_dir, detect_graph_languages};
use crate::models::{TritonComponent, TritonRoot};
use crate::templates::cmake_root_helpers;
use crate::util::write_text_if_changed;

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
    let graph_languages = detect_graph_languages(Path::new("."), root)?;
    body.push_str(&format!(
        "project({} LANGUAGES {})\n\n",
        root.app_name.replace('-', "_"),
        graph_languages.cmake_languages_clause()
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

fn generate_component_language_block(comp_dir: &Path, root: &TritonRoot, comp: &TritonComponent) -> Result<String> {
    let settings = detect_component_languages_in_dir(Path::new("."), comp_dir, root, comp)?;
    let mut lines = Vec::new();

    if let Some(c_std) = settings.c_std {
        lines.push(format!("set_property(TARGET ${{_comp_name}} PROPERTY C_STANDARD {})", c_std));
    }
    if let Some(cxx_std) = settings.cxx_std {
        lines.push(format!("set_property(TARGET ${{_comp_name}} PROPERTY CXX_STANDARD {})", cxx_std));
    }

    Ok(lines.join("\n"))
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
fn generate_managed_dep_block(name: &str, root: &TritonRoot, comp: &TritonComponent, config: Option<&str>) -> Vec<String> {
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

    let include_lines = gen_component_include_dirs_lines(root, comp);
    if !include_lines.is_empty() {
        dep_lines.extend(include_lines);
        if !dep_lines.last().unwrap().is_empty() {
            dep_lines.push("".into());
        }
    }

    let source_lines = gen_component_extra_sources_lines(root, comp);
    if !source_lines.is_empty() {
        dep_lines.extend(source_lines);
        if !dep_lines.last().unwrap().is_empty() {
            dep_lines.push("".into());
        }
    }

    let vcpkg_lines = gen_vcpkg_dep_lines(root, comp, name);
    if !vcpkg_lines.is_empty() {
        dep_lines.extend(vcpkg_lines);
        if !dep_lines.last().unwrap().is_empty() {
            dep_lines.push("".into());
        }
    }

    let component_link_lines = gen_component_link_lines(root, comp);
    if !component_link_lines.is_empty() {
        dep_lines.extend(component_link_lines);
        if !dep_lines.last().unwrap().is_empty() {
            dep_lines.push("".into());
        }
    }

    let git_lines = gen_git_dep_lines(root, name, comp);
    if !git_lines.is_empty() {
        dep_lines.extend(git_lines);
        if !dep_lines.last().unwrap().is_empty() {
            dep_lines.push("".into());
        }
    }

    let vl_lines = gen_component_vendor_libs_lines(root, comp);
    if !vl_lines.is_empty() {
        if !dep_lines.is_empty() && !dep_lines.last().unwrap().is_empty() {
            dep_lines.push("".into());
        }
        dep_lines.extend(vl_lines);
    }

    let system_lib_lines = gen_component_system_libs_lines(comp);
    if !system_lib_lines.is_empty() {
        if !dep_lines.is_empty() && !dep_lines.last().unwrap().is_empty() {
            dep_lines.push("".into());
        }
        dep_lines.extend(system_lib_lines);
    }

    let lo_lines = gen_component_link_options_lines(comp);
    if !lo_lines.is_empty() {
        if !dep_lines.is_empty() && !dep_lines.last().unwrap().is_empty() {
            dep_lines.push("".into());
        }
        dep_lines.extend(lo_lines);
    }

    let res_lines = gen_component_resources_lines(root, comp);
    if !res_lines.is_empty() {
        if !dep_lines.is_empty() && !dep_lines.last().unwrap().is_empty() {
            dep_lines.push("".into());
        }
        dep_lines.extend(res_lines);
    }

    let asset_lines = gen_component_assets_lines(root, comp, config);
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
    config: Option<&str>,
) -> Result<()> {
    let comp_dir = Path::new("components").join(name);
    if !comp_dir.is_dir() {
        return Ok(());
    }

    let path = comp_dir.join("CMakeLists.txt");
    let path_str = path.to_string_lossy().to_string();

    // Always regenerate the full component CMakeLists from the Triton template.
    // Component CMake files are Triton-owned output and should not preserve
    // ad-hoc local edits outside the generated dependency block.
    let base = crate::templates::component_cmakelists(name.eq_ignore_ascii_case("tests"));
    let base = ensure_cmake_version_header(&base, cmake_ver);
    let base_fixed = fix_target_runtime_dlls(&base);
    let language_block = generate_component_language_block(&comp_dir, root, comp)?;

    // Fill the generated placeholders in the freshly generated template.
    let lang_placeholder = "@TRITON_LANG_SETTINGS@";
    let deps_placeholder = "@TRITON_DEPS@";
    let dep_lines = generate_managed_dep_block(name, root, comp, config);
    let dep_block = dep_lines.join("\n");

    if !base_fixed.contains(lang_placeholder) || !base_fixed.contains(deps_placeholder) {
        unreachable!("component CMake template is missing a Triton placeholder");
    }

    let new_body = base_fixed
        .replace(lang_placeholder, &language_block)
        .replace(deps_placeholder, &dep_block);

    write_text_if_changed(&path_str, &new_body)
        .with_context(|| format!("writing {}", path_str))?;

    Ok(())
}

