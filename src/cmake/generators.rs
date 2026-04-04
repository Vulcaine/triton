use std::path::Path;

use crate::models::{DepSpec, GitDep, TritonComponent, TritonRoot};
use crate::util::{cmake_quote, infer_cmake_type, split_kv};

use super::resolution::{build_effective_git_specs, build_effective_vcpkg_specs};

fn normalize_staged_path(raw: &str) -> String {
    raw.trim().replace('\\', "/")
}

fn staged_source_expr(raw: &str, current_source_var: &str) -> (String, String) {
    let normalized = normalize_staged_path(raw);
    if let Some(rest) = normalized.strip_prefix("@root/") {
        (format!("${{CMAKE_SOURCE_DIR}}/../{}", rest), rest.to_string())
    } else {
        (format!("{}/{}", current_source_var, normalized), normalized)
    }
}

/* ----------------------------- cmake cache overrides ----------------------------- */

pub(super) fn push_git_cache_overrides(lines: &mut Vec<String>, g: &GitDep) {
    use crate::models::CMakeOverride;

    for ov in &g.cmake {
        match ov {
            CMakeOverride::Entry(e) => {
                let ty = if e.typ.is_empty() { "STRING" } else { &e.typ };
                let val_q = cmake_quote(&e.val);
                lines.push(format!(
                    "set({} {} CACHE {} \"\" FORCE)",
                    e.var, val_q, ty
                ));
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
                lines.push(format!(
                    "set({} {} CACHE {} \"\" FORCE)",
                    var, val_q, ty
                ));
            }
        }
    }
    lines.push(String::new());
}

pub(super) fn emit_git_dep(lines: &mut Vec<String>, g: &GitDep, public: bool) {
    let prefer = g.name.as_str();
    let vis = if public { "PUBLIC" } else { "PRIVATE" };
    lines.push(format!(
        "triton_add_subdir_and_link_strict(${{_comp_name}} \"${{CMAKE_SOURCE_DIR}}/../third_party/{name}\" \"{hint}\" {vis})",
        name = g.name, hint = prefer, vis = vis
    ));
    lines.push(String::new());
}

/* ------------------------- code generators (link blocks) ------------------------ */

pub(super) fn gen_git_dep_lines(root: &TritonRoot, comp_name: &str, comp: &TritonComponent) -> Vec<String> {
    let mut out = vec![];
    let specs = build_effective_git_specs(root, comp_name, comp);

    for spec in specs {
        let g = match root.deps.iter().find_map(|d| {
            if let DepSpec::Git(gg) = d {
                (gg.name == spec.name).then_some(gg)
            } else {
                None
            }
        }) {
            Some(x) => x,
            None => continue,
        };

        push_git_cache_overrides(&mut out, g);

        if !spec.targets.is_empty() {
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
            out.push(
                "  add_subdirectory(\"${_triton_src}\" \"${_triton_bin}\" EXCLUDE_FROM_ALL)".into(),
            );
            out.push("  set_property(GLOBAL PROPERTY TRITON_ADDED_SUBDIRS \"${_triton_added};${_triton_src}|${_triton_bin}\")".into());
            out.push("endif()".into());

            let vis = if spec.public { "PUBLIC" } else { "PRIVATE" };
            for t in spec.targets {
                out.push(format!(
"if(TARGET {t})
  if(NOT TARGET triton::{dep}::{t})
    add_library(triton::{dep}::{t} ALIAS {t})
  endif()
  target_link_libraries(${{_comp_name}} {vis} triton::{dep}::{t})
else()
  message(FATAL_ERROR \"git dep '{dep}' present but target '{t}' not found\")
endif()",
                    dep = g.name,
                    t = t,
                    vis = vis
                ));
            }
            out.push(String::new());
        } else {
            emit_git_dep(&mut out, g, spec.public);
        }
    }

    out
}

pub(super) fn gen_vcpkg_dep_lines(root: &TritonRoot, comp: &TritonComponent, comp_name: &str) -> Vec<String> {
    let mut lines = vec![];
    let specs = build_effective_vcpkg_specs(root, comp_name, comp);

    for spec in specs {
        if let Some(pkg) = spec.pkg_hint.clone() {
            if spec.targets.is_empty() {
                // Let the strict finder handle find_package + target detection
                lines.push(format!(
                    "# vcpkg: {} (package: {}; using strict finder)",
                    spec.name, pkg
                ));
                lines.push(format!(
                    "triton_find_vcpkg_and_link_strict(${{_comp_name}} \"{}\")",
                    pkg
                ));
                lines.push(String::new());
            } else {
                lines.push(format!("find_package({} CONFIG REQUIRED)", pkg));
                let vis = if spec.public { "PUBLIC" } else { "PRIVATE" };
                for t in spec.targets {
                    lines.push(format!(
"if(TARGET {t})
  target_link_libraries(${{_comp_name}} {vis} {t})
else()
  message(FATAL_ERROR \"Package '{p}' found but target '{t}' not defined.\")
endif()",
                        p = pkg,
                        t = t,
                        vis = vis
                    ));
                }
                lines.push(String::new());
            }
        } else {
            lines.push(format!(
                "# vcpkg: {} (no package hint; using strict finder)",
                spec.name
            ));
            lines.push(format!(
                "triton_find_vcpkg_and_link_strict(${{_comp_name}} \"{}\")",
                spec.name
            ));
            lines.push(String::new());
        }
    }

    lines
}

pub(super) fn gen_component_link_lines(root: &TritonRoot, comp: &TritonComponent) -> Vec<String> {
    let mut lines = vec![];
    for ent in &comp.link {
        let (name, _) = ent.normalize();
        if root.components.contains_key(&name) {
            let vis = if ent.is_public() { "PUBLIC" } else { "PRIVATE" };
            lines.push(format!(
                "target_link_libraries(${{_comp_name}} {vis} {name})"
            ));
            // Existing: include/<...>
            lines.push(format!(
                "if(EXISTS \"${{CMAKE_SOURCE_DIR}}/{n}/include\")
  target_include_directories(${{_comp_name}} PRIVATE \"${{CMAKE_SOURCE_DIR}}/{n}/include\")
endif()",
                n = name
            ));
            lines.push(format!(
                "if(EXISTS \"${{CMAKE_SOURCE_DIR}}/{n}\")
  target_include_directories(${{_comp_name}} PRIVATE \"${{CMAKE_SOURCE_DIR}}/{n}\")
endif()",
                n = name
            ));
        }
    }
    lines
}

pub(super) fn gen_component_resources_lines(comp: &TritonComponent) -> Vec<String> {
    let mut lines = vec![];
    for raw in &comp.resources {
        let raw = raw.trim();
        if raw.is_empty() { continue; }
        let (source_expr, display_path) = staged_source_expr(raw, "${CMAKE_CURRENT_SOURCE_DIR}");
        let dest_name = Path::new(&display_path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(&display_path);
        lines.push(format!(
"add_custom_command(TARGET ${{_comp_name}} POST_BUILD
    COMMAND ${{CMAKE_COMMAND}} -E copy_directory
        \"{source_expr}\"
        \"$<TARGET_FILE_DIR:${{_comp_name}}>/{dest_name}\"
    COMMENT \"Copying '{display_path}' next to executable\"
)",
            source_expr = source_expr, dest_name = dest_name, display_path = display_path
        ));
        lines.push(String::new());
    }
    lines
}

pub(super) fn gen_component_include_dirs_lines(comp: &TritonComponent) -> Vec<String> {
    if comp.include_dirs.is_empty() {
        return vec![];
    }

    let vis = if comp.kind == "exe" { "PRIVATE" } else { "PUBLIC" };
    let mut lines = vec!["set_property(TARGET ${_comp_name} PROPERTY TRITON_MANUAL_INCLUDE_DIRS TRUE)".into()];
    for raw in &comp.include_dirs {
        let raw = raw.trim();
        if raw.is_empty() {
            continue;
        }
        let (source_expr, _) = staged_source_expr(raw, "${CMAKE_CURRENT_SOURCE_DIR}");
        lines.push(format!(
            "target_include_directories(${{_comp_name}} {vis} \"{source_expr}\")",
            vis = vis,
            source_expr = source_expr,
        ));
    }
    lines
}

pub(super) fn gen_component_extra_sources_lines(comp: &TritonComponent) -> Vec<String> {
    if comp.sources.is_empty() {
        return vec![];
    }

    let mut lines = vec!["set_property(TARGET ${_comp_name} PROPERTY TRITON_MANUAL_SOURCES TRUE)".into()];
    for raw in &comp.sources {
        let raw = raw.trim();
        if raw.is_empty() {
            continue;
        }
        let (source_expr, display_path) = staged_source_expr(raw, "${CMAKE_CURRENT_SOURCE_DIR}");
        lines.push(format!("if(EXISTS \"{source_expr}\")", source_expr = source_expr));
        lines.push(format!(
            "  target_sources(${{_comp_name}} PRIVATE \"{source_expr}\")",
            source_expr = source_expr,
        ));
        lines.push("else()".into());
        lines.push(format!(
            "  message(WARNING \"triton: source path not found for '${{_comp_name}}': {display_path}\")",
            display_path = display_path,
        ));
        lines.push("endif()".into());
    }
    lines
}

pub(super) fn gen_component_vendor_libs_lines(comp: &TritonComponent) -> Vec<String> {
    use crate::models::VendorLibs;
    match &comp.vendor_libs {
        VendorLibs::None => vec![],
        VendorLibs::All(libs) => {
            if libs.is_empty() { return vec![]; }
            let paths: Vec<String> = libs.iter()
                .map(|p| format!("    \"${{CMAKE_CURRENT_SOURCE_DIR}}/{}\"", p))
                .collect();
            let mut lines = vec!["target_link_libraries(${_comp_name} PRIVATE".into()];
            lines.extend(paths);
            lines.push(")".into());
            lines
        }
        VendorLibs::PerPlatform(map) => {
            let mut lines = vec![];
            for (platform, libs) in map {
                if libs.is_empty() { continue; }
                let condition_str = super::platform_to_cmake_condition(platform);
                let paths: Vec<String> = libs.iter()
                    .map(|p| format!("    \"${{CMAKE_CURRENT_SOURCE_DIR}}/{}\"", p))
                    .collect();
                lines.push(format!("if({})", condition_str));
                lines.push("  target_link_libraries(${_comp_name} PRIVATE".into());
                for path in &paths {
                    lines.push(format!("  {}", path));
                }
                lines.push("  )".into());

                // On Windows, .lib files are import libraries -- copy sibling .dll
                // files next to the executable so they're found at runtime.
                if platform.to_ascii_lowercase() == "windows" {
                    for lib_path in libs {
                        if lib_path.ends_with(".lib") {
                            let dll_path = format!("{}dll", &lib_path[..lib_path.len() - 3]);
                            lines.push(format!(
                                "  if(EXISTS \"${{CMAKE_CURRENT_SOURCE_DIR}}/{}\")",
                                dll_path
                            ));
                            lines.push(format!(
                                "    add_custom_command(TARGET ${{_comp_name}} POST_BUILD COMMAND ${{CMAKE_COMMAND}} -E copy_if_different \"${{CMAKE_CURRENT_SOURCE_DIR}}/{}\" \"$<TARGET_FILE_DIR:${{_comp_name}}>\")",
                                dll_path
                            ));
                            lines.push("  endif()".into());
                        }
                    }
                }

                lines.push("endif()".into());
            }
            lines
        }
    }
}

pub(super) fn gen_component_link_options_lines(comp: &TritonComponent) -> Vec<String> {
    use crate::models::LinkOptions;
    match &comp.link_options {
        LinkOptions::None => vec![],
        LinkOptions::All(opts) => {
            if opts.is_empty() { return vec![]; }
            let joined = opts.iter().map(|o| cmake_quote(o)).collect::<Vec<_>>().join(" ");
            vec![format!("target_link_options(${{_comp_name}} PRIVATE {})", joined)]
        }
        LinkOptions::PerPlatform(map) => {
            let mut lines = vec![];
            for (platform, opts) in map {
                if opts.is_empty() { continue; }
                let joined = opts.iter().map(|o| cmake_quote(o)).collect::<Vec<_>>().join(" ");
                let condition_str = super::platform_to_cmake_condition(platform);
                lines.push(format!("if({})", condition_str));
                lines.push(format!("  target_link_options(${{_comp_name}} PRIVATE {})", joined));
                lines.push("endif()".into());
            }
            lines
        }
    }
}

pub(super) fn gen_component_system_libs_lines(comp: &TritonComponent) -> Vec<String> {
    if comp.system_libs.is_empty() {
        return vec![];
    }

    let libs = comp
        .system_libs
        .iter()
        .filter(|lib| !lib.trim().is_empty())
        .map(|lib| format!("    {}", cmake_quote(lib)))
        .collect::<Vec<_>>();

    if libs.is_empty() {
        return vec![];
    }

    let mut lines = vec!["target_link_libraries(${_comp_name} PRIVATE".into()];
    lines.extend(libs);
    lines.push(")".into());
    lines
}

pub(super) fn gen_component_defines_lines(comp: &TritonComponent) -> Vec<String> {
    if comp.defines.is_empty() {
        return vec![];
    }
    let mut parts = vec![];
    for d in &comp.defines {
        if !d.trim().is_empty() {
            parts.push(cmake_quote(d));
        }
    }
    if parts.is_empty() {
        return vec![];
    }
    vec![format!(
        "target_compile_definitions(${{_comp_name}} PRIVATE {})",
        parts.join(" ")
    )]
}

/// Generate CMake lines that copy declared assets next to the produced exe/lib,
/// incrementally. For each entry in `component.assets` (relative to component dir):
///  - Directory: mirror to `$<TARGET_FILE_DIR:...>/<basename>`; deletions handled.
///  - File: copy into `$<TARGET_FILE_DIR:...>` if changed.
/// Each copy rule produces a stamp file in the binary dir and we depend on all
/// stamps via a `${_comp_name}_assets` target wired into the component.
pub(super) fn gen_component_assets_lines(comp: &TritonComponent) -> Vec<String> {
    if comp.assets.is_empty() {
        return vec![];
    }

    // Helper to turn an asset path into a CMake-variable-safe id
    let make_id = |s: &str| -> String {
        s.chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect()
    };

    let mut lines = vec![];
    lines.push("# --- triton: stage component assets next to target (incremental) ---".into());
    lines.push("set(_triton_asset_stamps)".into());

    for raw in &comp.assets {
        let a = raw.trim();
        if a.is_empty() {
            continue;
        }
        let (source_expr, display_path) = staged_source_expr(a, "${CMAKE_CURRENT_SOURCE_DIR}");
        let id = make_id(&display_path);

        // Variables unique per asset
        //   _triton_asset_src_<id>
        //   _triton_asset_dst_<id>
        //   _triton_asset_name_<id>
        //   _triton_asset_files_<id>
        //   _triton_asset_stamp_<id>
        lines.push(format!("set(_triton_asset_src_{id} \"{source_expr}\")"));
        lines.push(format!("if(EXISTS \"${{_triton_asset_src_{id}}}\")"));
        lines.push(format!("  if(IS_DIRECTORY \"${{_triton_asset_src_{id}}}\")"));
        lines.push(format!("    get_filename_component(_triton_asset_name_{id} \"${{_triton_asset_src_{id}}}\" NAME)"));
        lines.push(format!("    set(_triton_asset_dst_{id} \"$<TARGET_FILE_DIR:${{_comp_name}}>/${{_triton_asset_name_{id}}}\")"));
        lines.push(format!("    set(_triton_asset_stamp_{id} \"${{CMAKE_CURRENT_BINARY_DIR}}/${{_comp_name}}_assets_{id}.stamp\")"));
        lines.push(format!("    file(GLOB_RECURSE _triton_asset_files_{id} CONFIGURE_DEPENDS \"${{_triton_asset_src_{id}}}/*\")"));
        lines.push("    add_custom_command(".into());
        lines.push(format!("      OUTPUT \"${{_triton_asset_stamp_{id}}}\""));
        lines.push(format!("      COMMAND \"${{CMAKE_COMMAND}}\" -E make_directory \"${{_triton_asset_dst_{id}}}\""));
        lines.push(format!("      COMMAND \"${{CMAKE_COMMAND}}\" -E remove_directory \"${{_triton_asset_dst_{id}}}\""));
        lines.push(format!("      COMMAND \"${{CMAKE_COMMAND}}\" -E make_directory \"${{_triton_asset_dst_{id}}}\""));
        lines.push(format!("      COMMAND \"${{CMAKE_COMMAND}}\" -E copy_directory \"${{_triton_asset_src_{id}}}\" \"${{_triton_asset_dst_{id}}}\""));
        lines.push(format!("      COMMAND \"${{CMAKE_COMMAND}}\" -E touch \"${{_triton_asset_stamp_{id}}}\""));
        lines.push(format!("      DEPENDS ${{_triton_asset_files_{id}}} \"${{CMAKE_BINARY_DIR}}/CMakeCache.txt\""));
        lines.push(format!("      COMMENT \"Syncing assets (dir): ${{_triton_asset_src_{id}}} -> ${{_triton_asset_dst_{id}}}\""));
        lines.push("      VERBATIM".into());
        lines.push("    )".into());
        lines.push(format!("    list(APPEND _triton_asset_stamps \"${{_triton_asset_stamp_{id}}}\")"));
        lines.push("  else()".into());
        lines.push(format!("    set(_triton_asset_dst_{id} \"$<TARGET_FILE_DIR:${{_comp_name}}>\")"));
        lines.push(format!("    set(_triton_asset_stamp_{id} \"${{CMAKE_CURRENT_BINARY_DIR}}/${{_comp_name}}_assets_{id}.stamp\")"));
        lines.push("    add_custom_command(".into());
        lines.push(format!("      OUTPUT \"${{_triton_asset_stamp_{id}}}\""));
        lines.push(format!("      COMMAND \"${{CMAKE_COMMAND}}\" -E make_directory \"${{_triton_asset_dst_{id}}}\""));
        lines.push(format!("      COMMAND \"${{CMAKE_COMMAND}}\" -E copy_if_different \"${{_triton_asset_src_{id}}}\" \"${{_triton_asset_dst_{id}}}\""));
        lines.push(format!("      COMMAND \"${{CMAKE_COMMAND}}\" -E touch \"${{_triton_asset_stamp_{id}}}\""));
        lines.push(format!("      DEPENDS \"${{_triton_asset_src_{id}}}\" \"${{CMAKE_BINARY_DIR}}/CMakeCache.txt\""));
        lines.push(format!("      COMMENT \"Copy asset file: ${{_triton_asset_src_{id}}} -> ${{_triton_asset_dst_{id}}}\""));
        lines.push("      VERBATIM".into());
        lines.push("    )".into());
        lines.push(format!("    list(APPEND _triton_asset_stamps \"${{_triton_asset_stamp_{id}}}\")"));
        lines.push("  endif()".into());
        lines.push("else()".into());
        lines.push(format!("  message(WARNING \"triton: asset path not found for '${{_comp_name}}': {display_path}\")"));
        lines.push("endif()".into());
    }

    lines.push("if(_triton_asset_stamps)".into());
    lines.push("  add_custom_target(${_comp_name}_assets ALL DEPENDS ${_triton_asset_stamps})".into());
    lines.push("  add_dependencies(${_comp_name} ${_comp_name}_assets)".into());
    lines.push("endif()".into());
    lines.push(String::new());

    lines
}

/// For exe components, collect vendor DLLs from all transitively-linked lib components
/// and generate POST_BUILD copy commands so the DLLs end up next to the exe.
pub(super) fn gen_transitive_vendor_dll_copies(_name: &str, root: &TritonRoot, comp: &TritonComponent) -> Vec<String> {
    use crate::models::VendorLibs;

    // Only exe components need DLLs copied next to them
    if comp.kind != "exe" { return vec![]; }

    // BFS: collect all transitively-linked component names
    let mut queue = std::collections::VecDeque::new();
    let mut visited = std::collections::HashSet::new();
    for ent in &comp.link {
        let (n, _) = ent.normalize();
        if root.components.contains_key(&n) {
            queue.push_back(n);
        }
    }
    while let Some(cn) = queue.pop_front() {
        if !visited.insert(cn.clone()) { continue; }
        if let Some(c) = root.components.get(&cn) {
            for ent in &c.link {
                let (n, _) = ent.normalize();
                if root.components.contains_key(&n) && !visited.contains(&n) {
                    queue.push_back(n);
                }
            }
        }
    }

    // Collect Windows vendor DLLs from all visited lib components
    let mut lines = vec![];
    for comp_name in &visited {
        let c = match root.components.get(comp_name) {
            Some(c) => c,
            None => continue,
        };
        let win_libs = match &c.vendor_libs {
            VendorLibs::PerPlatform(map) => {
                map.get("windows").or_else(|| map.get("Windows")).cloned().unwrap_or_default()
            }
            VendorLibs::All(libs) => libs.clone(),
            VendorLibs::None => continue,
        };
        for lib_path in &win_libs {
            if lib_path.ends_with(".lib") {
                let dll_path = format!("{}dll", &lib_path[..lib_path.len() - 3]);
                let full_dll = format!("${{CMAKE_SOURCE_DIR}}/{}/{}",  comp_name, dll_path);
                lines.push(format!("if(WIN32)"));
                lines.push(format!(
                    "  if(EXISTS \"{}\")", full_dll
                ));
                lines.push(format!(
                    "    add_custom_command(TARGET ${{_comp_name}} POST_BUILD COMMAND ${{CMAKE_COMMAND}} -E copy_if_different \"{}\" \"$<TARGET_FILE_DIR:${{_comp_name}}>\")",
                    full_dll
                ));
                lines.push("  endif()".into());
                lines.push("endif()".into());
            }
        }
    }
    lines
}
