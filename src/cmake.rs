use anyhow::{Context, Result};
use std::process::Command;
use std::collections::{HashSet, VecDeque};
use std::path::Path;

use crate::models::{DepSpec, GitDep, TritonComponent, TritonRoot};
use crate::templates::cmake_root_helpers;
use crate::util::{
    cmake_quote, infer_cmake_type, read_to_string_opt, split_kv, write_text_if_changed,
};

pub const MIN_CMAKE_VERSION: &str = "3.30.1";

pub fn parse_cmake_version(ver: &str) -> (u32, u32, u32) {
    let parts: Vec<_> = ver.split('.').collect();
    let major = parts.get(0).and_then(|s| s.parse().ok()).unwrap_or(0);
    let minor = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
    let patch = parts.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);
    (major, minor, patch)
}

/// Return the system's `cmake --version` string (e.g. "3.30.2"), if cmake is on PATH.
pub fn system_cmake_version() -> anyhow::Result<Option<String>> {
    let out = Command::new("cmake").arg("--version").output();
    match out {
        Ok(o) if o.status.success() => {
            let txt = String::from_utf8_lossy(&o.stdout);
            if let Some(first) = txt.lines().next() {
                let ver = first
                    .trim()
                    .strip_prefix("cmake version ")
                    .unwrap_or("")
                    .trim();
                if !ver.is_empty() {
                    return Ok(Some(ver.to_string()));
                }
            }
            Ok(None)
        }
        _ => Ok(None),
    }
}

/// Normalize OS strings from config into canonical values.
/// Only canonical OS values are allowed ("windows", "macos", "linux").
/// Rejects arch-suffixed names like "win32", "win64", "mac32", "mac64", "linux32", "linux64".
fn normalize_os(os: &str) -> Result<&'static str, String> {
    match os.to_ascii_lowercase().as_str() {
        "win" | "windows" => Ok("windows"),
        "mac" | "osx" | "darwin" | "macos" => Ok("macos"),
        "linux" => Ok("linux"),
        bad @ ("win32" | "win64" | "mac32" | "mac64" | "linux32" | "linux64") => Err(format!(
            "Invalid OS identifier '{}'. Use only 'windows', 'macos', or 'linux'.",
            bad
        )),
        other => Err(format!(
            "Unknown OS identifier '{}'. Allowed: windows, macos, linux.",
            other
        )),
    }
}

/// Check whether a dependency is active for this host OS + triplet.
pub fn dep_is_active(
    dep: &crate::models::DepSpec,
    name: &str,
    host_os: &str,
    triplet: &str,
) -> bool {
    use crate::models::DepSpec;

    match dep {
        DepSpec::Simple(s) => s.eq_ignore_ascii_case(name),

        DepSpec::Git(g) => g.name.eq_ignore_ascii_case(name),

        DepSpec::Detailed(d) => {
            if !d.name.eq_ignore_ascii_case(name) {
                return false;
            }

            let host_norm = normalize_os(host_os).unwrap_or_else(|e| {
                panic!("Invalid host OS string '{}': {}", host_os, e);
            });

            let os_ok = d.os.is_empty()
                || d.os.iter().any(|o| {
                    normalize_os(o)
                        .unwrap_or_else(|e| panic!("Invalid OS in dep '{}': {}", d.name, e))
                        == host_norm
                });

            let trip_ok =
                d.triplet.is_empty() || d.triplet.iter().any(|t| t.eq_ignore_ascii_case(triplet));

            os_ok && trip_ok
        }
    }
}

pub fn detect_vcpkg_triplet() -> String {
    if cfg!(target_os = "windows") {
        if cfg!(target_arch = "x86_64") {
            "x64-windows".into()
        } else if cfg!(target_arch = "aarch64") {
            "arm64-windows".into()
        } else {
            "x86-windows".into()
        }
    } else if cfg!(target_os = "macos") {
        if cfg!(target_arch = "aarch64") {
            "arm64-osx".into()
        } else {
            "x64-osx".into()
        }
    } else if cfg!(target_os = "linux") {
        if cfg!(target_arch = "aarch64") {
            "arm64-linux".into()
        } else {
            "x64-linux".into()
        }
    } else {
        "x64-linux".into() // fallback
    }
}

pub fn regenerate_root_cmake(root: &TritonRoot) -> Result<()> {
    let path = "components/CMakeLists.txt";

    // Header: includes exactly one `cmake_minimum_required()` and one `project()`
    let mut body = String::new();

    let (maj, min, pat) = effective_cmake_version();
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
/* ------------------------- export propagation helpers ------------------------- */

#[derive(Clone)]
struct VcpkgSpec {
    name: String,
    pkg_hint: Option<String>,
    targets: Vec<String>,
    public: bool,
}

#[derive(Clone)]
struct GitSpec {
    name: String,
    targets: Vec<String>,
    public: bool,
}

fn find_upstream_dep_spec(
    root: &TritonRoot,
    start_comp: &str,
    dep_name: &str,
) -> Option<(Option<String>, Vec<String>)> {
    let mut q = VecDeque::new();
    let mut seen = HashSet::new();
    q.push_back(start_comp.to_string());
    seen.insert(start_comp.to_string());

    while let Some(curr) = q.pop_front() {
        let c = root.components.get(&curr)?;
        for ent in &c.link {
            let (n, pkg_hint) = ent.normalize();
            if n == dep_name {
                return Some((pkg_hint, ent.all_targets()));
            }
        }
        for ent in &c.link {
            let (n, _) = ent.normalize();
            if root.components.contains_key(&n) && !seen.contains(&n) {
                seen.insert(n.clone());
                q.push_back(n);
            }
        }
    }
    None
}

/* ----------------------------- cmake cache overrides ----------------------------- */

fn push_git_cache_overrides(lines: &mut Vec<String>, g: &GitDep) {
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

fn emit_git_dep(lines: &mut Vec<String>, g: &GitDep) {
    let prefer = g.name.as_str();
    lines.push(format!(
        "triton_add_subdir_and_link_strict(${{_comp_name}} \"${{CMAKE_SOURCE_DIR}}/../third_party/{name}\" \"{hint}\")",
        name = g.name, hint = prefer
    ));
    lines.push(String::new());
}

/* ------------------------------ effective dep sets ------------------------------ */

fn build_effective_git_specs(
    root: &TritonRoot,
    comp_name: &str,
    comp: &TritonComponent,
) -> Vec<GitSpec> {
    let mut out = Vec::new();
    let mut seen = HashSet::<String>::new();

    for ent in &comp.link {
        let (name, _) = ent.normalize();
        if name.is_empty() {
            continue;
        }
        if !root
            .deps
            .iter()
            .any(|d| matches!(d, DepSpec::Git(g) if g.name == name))
        {
            continue;
        }
        let public = comp.exports.contains(&name);
        if seen.insert(name.clone()) {
            out.push(GitSpec {
                name: name.clone(),
                targets: ent.all_targets(),
                public,
            });
        }
    }

    for exp_name in &comp.exports {
        if seen.contains(exp_name) {
            continue;
        }
        if !root
            .deps
            .iter()
            .any(|d| matches!(d, DepSpec::Git(g) if g.name == *exp_name))
        {
            continue;
        }
        if let Some((_, targets)) = find_upstream_dep_spec(root, comp_name, exp_name) {
            if !targets.is_empty() && seen.insert(exp_name.clone()) {
                out.push(GitSpec {
                    name: exp_name.clone(),
                    targets,
                    public: true,
                });
            }
        }
    }
    out
}

fn build_effective_vcpkg_specs(
    root: &TritonRoot,
    comp_name: &str,
    comp: &TritonComponent,
) -> Vec<VcpkgSpec> {
    let mut out = Vec::new();
    let mut seen = HashSet::<String>::new();

    for ent in &comp.link {
        let (name, pkg_hint) = ent.normalize();
        if name.is_empty() {
            continue;
        }
        if !root.deps.iter().any(|d| match d {
            DepSpec::Simple(n) => n == &name,
            DepSpec::Detailed(dd) => dd.name == name,
            _ => false,
        }) {
            continue;
        }
        let public = comp.exports.contains(&name);

        // If your DepDetailed has an optional 'package' in your codebase, use it here.
        let pkg_override = root.deps.iter().find_map(|d| {
            if let DepSpec::Detailed(dd) = d {
                if dd.name.eq_ignore_ascii_case(&name) {
                    dd.package.clone()
                } else {
                    None
                }
            } else {
                None
            }
        });

        if seen.insert(name.clone()) {
            out.push(VcpkgSpec {
                name: name.clone(),
                pkg_hint: pkg_override.or(pkg_hint),
                targets: ent.all_targets(),
                public,
            });
        }
    }

    for exp_name in &comp.exports {
        if seen.contains(exp_name) {
            continue;
        }
        if !root
            .deps
            .iter()
            .any(|d| matches!(d, DepSpec::Simple(n) if n == exp_name))
        {
            continue;
        }
        if let Some((pkg_hint, targets)) = find_upstream_dep_spec(root, comp_name, exp_name) {
            if !targets.is_empty() && seen.insert(exp_name.clone()) {
                out.push(VcpkgSpec {
                    name: exp_name.clone(),
                    pkg_hint,
                    targets,
                    public: true,
                });
            }
        }
    }
    out
}

/* ------------------------- code generators (link blocks) ------------------------ */

fn gen_git_dep_lines(root: &TritonRoot, comp_name: &str, comp: &TritonComponent) -> Vec<String> {
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
            emit_git_dep(&mut out, g);
        }
    }

    out
}

fn gen_vcpkg_dep_lines(root: &TritonRoot, comp: &TritonComponent, comp_name: &str) -> Vec<String> {
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

fn gen_component_link_lines(root: &TritonRoot, comp: &TritonComponent) -> Vec<String> {
    let mut lines = vec![];
    for ent in &comp.link {
        let (name, _) = ent.normalize();
        if root.components.contains_key(&name) {
            lines.push(format!(
                "target_link_libraries(${{_comp_name}} PRIVATE {name})"
            ));
            // Existing: include/<…>
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

fn gen_component_resources_lines(comp: &TritonComponent) -> Vec<String> {
    let mut lines = vec![];
    for res in &comp.resources {
        let res = res.trim();
        if res.is_empty() { continue; }
        let dest_name = Path::new(res)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(res);
        lines.push(format!(
"add_custom_command(TARGET ${{_comp_name}} POST_BUILD
    COMMAND ${{CMAKE_COMMAND}} -E copy_directory
        \"${{CMAKE_CURRENT_SOURCE_DIR}}/{res}\"
        \"$<TARGET_FILE_DIR:${{_comp_name}}>/{dest_name}\"
    COMMENT \"Copying '{res}' next to executable\"
)",
            res = res, dest_name = dest_name
        ));
        lines.push(String::new());
    }
    lines
}

/// Map a triton platform name to a CMake conditional expression.
fn platform_to_cmake_condition(platform: &str) -> String {
    match platform.to_ascii_lowercase().as_str() {
        "linux"   => "UNIX AND NOT APPLE".to_string(),
        "macos"   => "APPLE".to_string(),
        "windows" => "WIN32".to_string(),
        other     => other.to_string(),
    }
}

fn gen_component_vendor_libs_lines(comp: &TritonComponent) -> Vec<String> {
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
                let condition_str = platform_to_cmake_condition(platform);
                let paths: Vec<String> = libs.iter()
                    .map(|p| format!("    \"${{CMAKE_CURRENT_SOURCE_DIR}}/{}\"", p))
                    .collect();
                lines.push(format!("if({})", condition_str));
                lines.push("  target_link_libraries(${_comp_name} PRIVATE".into());
                for path in &paths {
                    lines.push(format!("  {}", path));
                }
                lines.push("  )".into());

                // On Windows, .lib files are import libraries — copy sibling .dll
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

fn gen_component_link_options_lines(comp: &TritonComponent) -> Vec<String> {
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
                let condition_str = platform_to_cmake_condition(platform);
                lines.push(format!("if({})", condition_str));
                lines.push(format!("  target_link_options(${{_comp_name}} PRIVATE {})", joined));
                lines.push("endif()".into());
            }
            lines
        }
    }
}

fn gen_component_defines_lines(comp: &TritonComponent) -> Vec<String> {
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
fn gen_component_assets_lines(comp: &TritonComponent) -> Vec<String> {
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
        let id = make_id(a);

        // Variables unique per asset
        //   _triton_asset_src_<id>
        //   _triton_asset_dst_<id>
        //   _triton_asset_name_<id>
        //   _triton_asset_files_<id>
        //   _triton_asset_stamp_<id>
        lines.push(format!("set(_triton_asset_src_{id} \"${{CMAKE_CURRENT_SOURCE_DIR}}/{a}\")"));
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
        lines.push(format!("  message(WARNING \"triton: asset path not found for '${{_comp_name}}': ${{_triton_asset_src_{id}}}\")"));
        lines.push("endif()".into());
    }

    lines.push("if(_triton_asset_stamps)".into());
    lines.push("  add_custom_target(${_comp_name}_assets ALL DEPENDS ${_triton_asset_stamps})".into());
    lines.push("  add_dependencies(${_comp_name} ${_comp_name}_assets)".into());
    lines.push("endif()".into());
    lines.push(String::new());

    lines
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

/// For exe components, collect vendor DLLs from all transitively-linked lib components
/// and generate POST_BUILD copy commands so the DLLs end up next to the exe.
fn gen_transitive_vendor_dll_copies(_name: &str, root: &TritonRoot, comp: &TritonComponent) -> Vec<String> {
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

/// Helper: pick effective cmake version (system if >= MIN, else MIN).
pub fn effective_cmake_version() -> (u32, u32, u32) {
    let min_ver = parse_cmake_version(MIN_CMAKE_VERSION);
    system_cmake_version()
        .ok()
        .flatten()
        .map(|s| parse_cmake_version(&s))
        .map(|sys| if sys >= min_ver { sys } else { min_ver })
        .unwrap_or(min_ver)
}
