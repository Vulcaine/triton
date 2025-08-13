use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TritonRoot {
    pub app_name: String,
    pub triplet: String,
    pub generator: String,
    pub cxx_std: String,
    pub deps: Vec<RootDep>,
    pub components: BTreeMap<String, TritonComponent>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RootDep {
    Name(String),
    Git(GitDep),
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GitDep {
    pub repo: String,
    pub name: String,
    #[serde(default)]
    pub branch: Option<String>,

    /// Accept both old structured entries and new "VAR=VALUE" strings.
    #[serde(default)]
    pub cmake: Vec<CMakeOverride>,

    /// Kept for backward compat (not used by new flow).
    #[serde(default)]
    pub target: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum CMakeOverride {
    /// Simple "VAR=VALUE" form (preferred).
    KV(String),
    /// Backward-compatible structured form.
    Entry(CMakeCacheEntry),
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CMakeCacheEntry {
    pub var: String,
    pub val: String,
    #[serde(default = "default_cache_type")]
    pub typ: String, // Usually "BOOL" or "STRING"
}
fn default_cache_type() -> String { "STRING".into() }

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TritonComponent {
    pub kind: String, // "exe" | "lib"
    #[serde(default)]
    pub link: Vec<LinkEntry>,
}

/// Allow forms inside `components.<name>.link`:
/// 1) "sdl2"
/// 2) { "name": "rmlui", "package": "RmlUi", "target": "RmlUi::RmlUi" }
/// 3) { "rmlui": { "package": "RmlUi", "target": "RmlUi::RmlUi" } }
/// 4) { "name": "filament", "targets": ["filament","utils","math"] }
/// 5) { "filament": { "targets": ["filament","utils","math"] } }
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum LinkEntry {
    Name(String),
    Named {
        name: String,
        #[serde(default)]
        package: Option<String>,
        #[serde(default)]
        target: Option<String>,
        /// Accept "targets" and common typo "tragets"
        #[serde(default, alias = "tragets")]
        targets: Option<Vec<String>>,
    },
    Map(BTreeMap<String, LinkHint>),
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LinkHint {
    #[serde(default)]
    pub package: Option<String>,
    #[serde(default)]
    pub target: Option<String>,
    /// Accept "targets" and common typo "tragets"
    #[serde(default, alias = "tragets")]
    pub targets: Option<Vec<String>>,
}

impl LinkEntry {
    /// Canonicalize any variant into (name, package_hint, target_hint)
    pub fn normalize(&self) -> (String, Option<String>, Option<String>) {
        match self {
            LinkEntry::Name(n) => (n.clone(), None, None),
            LinkEntry::Named { name, package, target, targets } => {
                let first = target.clone().or_else(|| targets.as_ref().and_then(|v| v.get(0).cloned()));
                (name.clone(), package.clone(), first)
            }
            LinkEntry::Map(map) => {
                if let Some((k, v)) = map.iter().next() {
                    let first = v.target.clone().or_else(|| v.targets.as_ref().and_then(|vv| vv.get(0).cloned()));
                    (k.clone(), v.package.clone(), first)
                } else {
                    ("".into(), None, None)
                }
            }
        }
    }

    /// Return all explicit targets requested (single or multiple).
    pub fn all_targets(&self) -> Vec<String> {
        match self {
            LinkEntry::Name(_) => Vec::new(),
            LinkEntry::Named { target, targets, .. } => {
                let mut out = Vec::new();
                if let Some(t) = target { out.push(t.clone()); }
                if let Some(ts) = targets {
                    for t in ts {
                        if !out.iter().any(|x| x == t) { out.push(t.clone()); }
                    }
                }
                out
            }
            LinkEntry::Map(map) => {
                if let Some((_k, v)) = map.iter().next() {
                    let mut out = Vec::new();
                    if let Some(t) = &v.target { out.push(t.clone()); }
                    if let Some(ts) = &v.targets {
                        for t in ts {
                            if !out.iter().any(|x| x == t) { out.push(t.clone()); }
                        }
                    }
                    out
                } else {
                    Vec::new()
                }
            }
        }
    }
}
