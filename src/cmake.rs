use anyhow::{Context, Result};
use std::collections::{HashSet, VecDeque};

use crate::models::{GitDep, RootDep, TritonComponent, TritonRoot};
use crate::templates::{cmake_root_helpers, components_dir_cmakelists};
use crate::util::{
    cmake_quote, infer_cmake_type, read_to_string_opt, split_kv, write_text_if_changed,
};

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

    // 3) managed subdirectories only (simple alphabetical; upgrade to topo if you want)
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

/* ------------------------- export propagation helpers ------------------------- */

/// Information we need to link a vcpkg dep.
#[derive(Clone)]
struct VcpkgSpec {
    name: String,
    pkg_hint: Option<String>,
    targets: Vec<String>, // empty => no explicit targets (can't PUBLIC-link using strict helper)
    public: bool,
}

/// Information we need to link a git dep.
#[derive(Clone)]
struct GitSpec {
    name: String,
    targets: Vec<String>, // empty => fallback helper, which is PRIVATE only
    public: bool,
}

/// BFS over upstream components (those this component depends on) to find a dep spec.
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
        // Does this component link the dep directly? If so, extract details.
        for ent in &c.link {
            let (n, pkg_hint) = ent.normalize();
            if n == dep_name {
                let targets = ent.all_targets();
                return Some((pkg_hint, targets));
            }
        }
        // Otherwise enqueue upstream components
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

/// Build the set of GIT deps the component must link, including propagated
/// exports (PUBLIC) that come from upstream components.
fn build_effective_git_specs(root: &TritonRoot, comp_name: &str, comp: &TritonComponent) -> Vec<GitSpec> {
    let mut out: Vec<GitSpec> = Vec::new();
    let mut seen = HashSet::<String>::new();

    // 1) Direct git deps
    for ent in &comp.link {
        let (name, _pkg) = ent.normalize();
        if name.is_empty() { continue; }
        if !root.deps.iter().any(|d| matches!(d, RootDep::Git(g) if g.name == name)) {
            continue;
        }
        let public = comp.exports.iter().any(|e| e == &name);
        let spec = GitSpec { name: name.clone(), targets: ent.all_targets(), public };
        if seen.insert(name) { out.push(spec); }
    }

    // 2) Propagated exports: for each exported name that is a git dep, if not directly linked,
    //    try to locate upstream component that links it (and copy its targets).
    for exp_name in &comp.exports {
        if seen.contains(exp_name) { continue; }
        if !root.deps.iter().any(|d| matches!(d, RootDep::Git(g) if g.name == *exp_name)) {
            continue;
        }
        if let Some((_, targets)) = find_upstream_dep_spec(root, comp_name, exp_name) {
            // Only propagate if targets are known; otherwise we'd fall back to the strict helper
            // which links PRIVATE. If you need PUBLIC here, declare explicit targets upstream.
            if !targets.is_empty() && seen.insert(exp_name.clone()) {
                out.push(GitSpec { name: exp_name.clone(), targets, public: true });
            }
        }
    }

    out
}

/// Build the set of VCPKG deps the component must link, including propagated PUBLIC exports.
fn build_effective_vcpkg_specs(root: &TritonRoot, comp_name: &str, comp: &TritonComponent) -> Vec<VcpkgSpec> {
    let mut out: Vec<VcpkgSpec> = Vec::new();
    let mut seen = HashSet::<String>::new();

    // 1) Direct vcpkg deps
    for ent in &comp.link {
        let (name, pkg_hint) = ent.normalize();
        if name.is_empty() { continue; }
        if !root.deps.iter().any(|d| matches!(d, RootDep::Name(n) if n == &name)) {
            continue;
        }
        let public = comp.exports.iter().any(|e| e == &name);
        let spec = VcpkgSpec { name: name.clone(), pkg_hint, targets: ent.all_targets(), public };
        if seen.insert(name) { out.push(spec); }
    }

    // 2) Propagated exports
    for exp_name in &comp.exports {
        if seen.contains(exp_name) { continue; }
        if !root.deps.iter().any(|d| matches!(d, RootDep::Name(n) if n == exp_name)) {
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
        // Find matching git root dep (to push cache overrides & source dir)
        let g = match root.deps.iter().find_map(|d| {
            if let RootDep::Git(gg) = d { (gg.name == spec.name).then_some(gg) } else { None }
        }) {
            Some(x) => x,
            None => continue,
        };

        // Always forward cache overrides before add_subdirectory
        push_git_cache_overrides(&mut out, g);

        if !spec.targets.is_empty() {
            // Deduped add_subdirectory for this repo
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
            // No explicit targets known -> fallback helper (PRIVATE). We don't force PUBLIC here.
            emit_git_dep(&mut out, g);
        }
    }

    out
}

fn gen_vcpkg_dep_lines(root: &TritonRoot, comp: &TritonComponent, comp_name: &str) -> Vec<String> {
    let mut lines: Vec<String> = vec![];
    let specs = build_effective_vcpkg_specs(root, comp_name, comp);

    for spec in specs {
        // We need a package hint to call find_package with CONFIG REQUIRED
        if let Some(pkg) = spec.pkg_hint.clone() {
            lines.push(format!("find_package({} CONFIG REQUIRED)", pkg));
            if spec.targets.is_empty() {
                // Without explicit targets, our strict helper links PRIVATE; keep behavior consistent.
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
            // No pkg hint -> strict helper; will be PRIVATE. To re-export PUBLIC, add explicit targets upstream.
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
        let (name, _pkg) = ent.normalize();
        if root.components.contains_key(&name) {
            // Always PRIVATE for component->component; exports propagate via explicit PUBLIC links to deps (above).
            lines.push(format!("target_link_libraries(${{_comp_name}} PRIVATE {name})"));
            // Make consumer see provider's headers
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
    let mut parts: Vec<String> = Vec::new();
    for d in &comp.defines {
        if d.trim().is_empty() { continue; }
        parts.push(cmake_quote(d));
    }
    if parts.is_empty() { return vec![]; }
    vec![format!("target_compile_definitions(${{_comp_name}} PRIVATE {})", parts.join(" "))]
}

pub fn rewrite_component_cmake(name: &str, root: &TritonRoot, comp: &TritonComponent) -> Result<()> {
    let path = format!("components/{name}/CMakeLists.txt");
    let base_raw = read_to_string_opt(&path).unwrap_or_else(crate::templates::component_cmakelists);

    // Idempotent include visibility migration
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

    // Per-component compile definitions (GLM_ENABLE_EXPERIMENTAL, etc.)
    dep_lines.extend(gen_component_defines_lines(comp));
    if !dep_lines.is_empty() && !dep_lines.last().unwrap().is_empty() { dep_lines.push("".into()); }

    // External deps (git + vcpkg), including propagated PUBLIC exports
    dep_lines.extend(gen_git_dep_lines(root, name, comp));
    if !dep_lines.is_empty() && !dep_lines.last().unwrap().is_empty() { dep_lines.push("".into()); }

    let vcpkg_lines = gen_vcpkg_dep_lines(root, comp, name);
    if !vcpkg_lines.is_empty() {
        dep_lines.extend(vcpkg_lines);
        if !dep_lines.last().unwrap().is_empty() { dep_lines.push("".into()); }
    }

    // Internal component links (PRIVATE + add include dir)
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
