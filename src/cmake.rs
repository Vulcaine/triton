use anyhow::{Context, Result};
use std::collections::{HashSet, VecDeque};
use std::path::Path;

use crate::models::{DepSpec, GitDep, TritonComponent, TritonRoot};
use crate::templates::{cmake_root_helpers, components_dir_cmakelists};
use crate::util::{
    cmake_quote, infer_cmake_type, read_to_string_opt, split_kv, write_text_if_changed,
};

pub fn dep_is_active(dep: &crate::models::DepSpec, name: &str, host_os: &str, triplet: &str) -> bool {
    use crate::models::DepSpec;
    match dep {
        DepSpec::Simple(s) => s.eq_ignore_ascii_case(name),
        DepSpec::Git(g)    => g.name.eq_ignore_ascii_case(name),
        DepSpec::Detailed(d) => {
            if !d.name.eq_ignore_ascii_case(name) {
                return false;
            }
            let os_ok = d.os.is_empty() || d.os.iter().any(|o| o.eq_ignore_ascii_case(host_os));
            let trip_ok = d.triplet.is_empty() || d.triplet.iter().any(|t| t.eq_ignore_ascii_case(triplet));
            os_ok && trip_ok
        }
    }
}

pub fn detect_vcpkg_triplet() -> String {
    if cfg!(target_os = "windows") {
        if cfg!(target_arch = "x86_64") { "x64-windows".into() }
        else if cfg!(target_arch = "aarch64") { "arm64-windows".into() }
        else { "x86-windows".into() }
    } else if cfg!(target_os = "macos") {
        if cfg!(target_arch = "aarch64") { "arm64-osx".into() }
        else { "x64-osx".into() }
    } else if cfg!(target_os = "linux") {
        if cfg!(target_arch = "aarch64") { "arm64-linux".into() }
        else { "x64-linux".into() }
    } else {
        "x64-linux".into() // fallback
    }
}

/// Write/refresh components/CMakeLists.txt with helpers + subdirs (no build logic here)
pub fn regenerate_root_cmake(root: &TritonRoot) -> Result<()> {
    let path = "components/CMakeLists.txt";

    let mut body = String::new();

    // 1) header (tiny)
    body.push_str(&components_dir_cmakelists());
    body.push('\n');

    // 2) helper functions block (from resources/cmake_root_template.cmake)
    body.push_str(cmake_root_helpers());
    body.push('\n');

    // 3) managed subdirectories (only if components/<name> exists)
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

fn build_effective_git_specs(root: &TritonRoot, comp_name: &str, comp: &TritonComponent) -> Vec<GitSpec> {
    let mut out = Vec::new();
    let mut seen = HashSet::<String>::new();

    for ent in &comp.link {
        let (name, _) = ent.normalize();
        if name.is_empty() { continue; }
        if !root.deps.iter().any(|d| matches!(d, DepSpec::Git(g) if g.name == name)) {
            continue;
        }
        let public = comp.exports.contains(&name);
        if seen.insert(name.clone()) {
            out.push(GitSpec { name: name.clone(), targets: ent.all_targets(), public });
        }
    }

    for exp_name in &comp.exports {
        if seen.contains(exp_name) { continue; }
        if !root.deps.iter().any(|d| matches!(d, DepSpec::Git(g) if g.name == *exp_name)) {
            continue;
        }
        if let Some((_, targets)) = find_upstream_dep_spec(root, comp_name, exp_name) {
            if !targets.is_empty() && seen.insert(exp_name.clone()) {
                out.push(GitSpec { name: exp_name.clone(), targets, public: true });
            }
        }
    }
    out
}

fn build_effective_vcpkg_specs(root: &TritonRoot, comp_name: &str, comp: &TritonComponent) -> Vec<VcpkgSpec> {
    let mut out = Vec::new();
    let mut seen = HashSet::<String>::new();

    for ent in &comp.link {
        let (name, pkg_hint) = ent.normalize();
        if name.is_empty() { continue; }
        if !root.deps.iter().any(|d| matches!(d, DepSpec::Simple(n) if n == &name)) {
            continue;
        }
        let public = comp.exports.contains(&name);
        if seen.insert(name.clone()) {
            out.push(VcpkgSpec { name: name.clone(), pkg_hint, targets: ent.all_targets(), public });
        }
    }

    for exp_name in &comp.exports {
        if seen.contains(exp_name) { continue; }
        if !root.deps.iter().any(|d| matches!(d, DepSpec::Simple(n) if n == exp_name)) {
            continue;
        }
        if let Some((pkg_hint, targets)) = find_upstream_dep_spec(root, comp_name, exp_name) {
            if !targets.is_empty() && seen.insert(exp_name.clone()) {
                out.push(VcpkgSpec { name: exp_name.clone(), pkg_hint, targets, public: true });
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
            if let DepSpec::Git(gg) = d { (gg.name == spec.name).then_some(gg) } else { None }
        }) {
            Some(x) => x,
            None => continue,
        };

        push_git_cache_overrides(&mut out, g);

        if !spec.targets.is_empty() {
            out.push(format!("set(_triton_src \"${{CMAKE_SOURCE_DIR}}/../third_party/{n}\")", n = g.name));
            out.push(format!("set(_triton_bin \"${{CMAKE_BINARY_DIR}}/third_party/{n}\")", n = g.name));
            out.push("get_property(_triton_added GLOBAL PROPERTY TRITON_ADDED_SUBDIRS)".into());
            out.push("if(NOT _triton_added)".into());
            out.push("  set(_triton_added \"\")".into());
            out.push("endif()".into());
            out.push("list(FIND _triton_added \"${_triton_src}|${_triton_bin}\" _ix)".into());
            out.push("if(_ix EQUAL -1)".into());
            out.push("  add_subdirectory(\"${_triton_src}\" \"${_triton_bin}\" EXCLUDE_FROM_ALL)".into());
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
                    dep = g.name, t = t, vis = vis
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
            lines.push(format!("find_package({} CONFIG REQUIRED)", pkg));
            if spec.targets.is_empty() {
                lines.push(format!("# vcpkg: {} (no explicit targets; using strict finder)", spec.name));
                lines.push(format!("triton_find_vcpkg_and_link_strict(${{_comp_name}} \"{}\")", spec.name));
                lines.push(String::new());
            } else {
                let vis = if spec.public { "PUBLIC" } else { "PRIVATE" };
                for t in spec.targets {
                    lines.push(format!(
"if(TARGET {t})
  target_link_libraries(${{_comp_name}} {vis} {t})
else()
  message(FATAL_ERROR \"Package '{p}' found but target '{t}' not defined.\")
endif()",
                        p = pkg, t = t, vis = vis
                    ));
                }
                lines.push(String::new());
            }
        } else {
            lines.push(format!("# vcpkg: {} (no package hint; using strict finder)", spec.name));
            lines.push(format!("triton_find_vcpkg_and_link_strict(${{_comp_name}} \"{}\")", spec.name));
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
            lines.push(format!("target_link_libraries(${{_comp_name}} PRIVATE {name})"));
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
    if comp.defines.is_empty() { return vec![]; }
    let mut parts = vec![];
    for d in &comp.defines {
        if !d.trim().is_empty() {
            parts.push(cmake_quote(d));
        }
    }
    if parts.is_empty() { return vec![]; }
    vec![format!("target_compile_definitions(${{_comp_name}} PRIVATE {})", parts.join(" "))]
}

pub fn rewrite_component_cmake(name: &str, root: &TritonRoot, comp: &TritonComponent) -> Result<()> {
    let comp_dir = Path::new("components").join(name);
    if !comp_dir.is_dir() { return Ok(()); }

    let path = comp_dir.join("CMakeLists.txt");
    let path_str = path.to_string_lossy().to_string();

    let base_raw = read_to_string_opt(&path_str).unwrap_or_else(|| {
        crate::templates::component_cmakelists(name.eq_ignore_ascii_case("tests"))
    });

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
    if !had_conditional && base_raw.contains(r#"target_include_directories(${_comp_name} PRIVATE "include")"#) {
        base = base_raw.replace(
            r#"target_include_directories(${_comp_name} PRIVATE "include")"#,
            canonical,
        );
    } else if base_raw.contains(duplicated) {
        base = base_raw.replace(duplicated, canonical);
    }

    let begin = "# ## triton:deps begin";
    let end = "# ## triton:deps end";
    let (pre, post) = match (base.find(begin), base.find(end)) {
        (Some(b), Some(e)) if e >= b => (base[..b].to_string(), base[(e + end.len())..].to_string()),
        _ => (base, "\n".to_string()),
    };

    let mut dep_lines = vec![
        "# --- triton: resolve local target name ---".into(),
        "if(NOT DEFINED _comp_name)".into(),
        "  get_filename_component(_comp_name \"${CMAKE_CURRENT_SOURCE_DIR}\" NAME)".into(),
        "endif()".into(),
        "".into(),
    ];

    dep_lines.extend(gen_component_defines_lines(comp));
    if !dep_lines.is_empty() && !dep_lines.last().unwrap().is_empty() { dep_lines.push("".into()); }

    dep_lines.extend(gen_git_dep_lines(root, name, comp));
    if !dep_lines.is_empty() && !dep_lines.last().unwrap().is_empty() { dep_lines.push("".into()); }

    let vcpkg_lines = gen_vcpkg_dep_lines(root, comp, name);
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

    write_text_if_changed(&path_str, &new_body).with_context(|| format!("writing {}", path_str))?;
    Ok(())
}
