use std::collections::{HashSet, VecDeque};

use crate::models::{DepSpec, TritonComponent, TritonRoot};

#[derive(Clone)]
pub(super) struct VcpkgSpec {
    pub name: String,
    pub pkg_hint: Option<String>,
    pub targets: Vec<String>,
    pub public: bool,
}

#[derive(Clone)]
pub(super) struct GitSpec {
    pub name: String,
    pub targets: Vec<String>,
    pub public: bool,
}

pub(super) fn find_upstream_dep_spec(
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

pub(super) fn build_effective_git_specs(
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
        let public = ent.is_public() || comp.exports.contains(&name);
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

pub(super) fn build_effective_vcpkg_specs(
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
        let public = ent.is_public() || comp.exports.contains(&name);

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
