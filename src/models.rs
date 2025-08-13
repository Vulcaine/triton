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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitDep {
    pub repo: String,
    pub name: String,
    #[serde(default)]
    pub branch: Option<String>,
    /// CMake cache overrides before add_subdirectory.
    /// You can use either a structured entry or a raw `VAR=VAL` string.
    #[serde(default)]
    pub cmake: Vec<CMakeOverride>,
}

impl Default for GitDep {
    fn default() -> Self {
        Self {
            repo: String::new(),
            name: String::new(),
            branch: None,
            cmake: vec![],
        }
    }
}

/// Support either a structured cache entry or a raw `VAR=VAL`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum CMakeOverride {
    Entry(CMakeCacheEntry),
    KV(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CMakeCacheEntry {
    pub var: String,
    pub val: String,
    /// Optional CMake cache type; default "STRING".
    #[serde(default = "default_cache_type")]
    pub typ: String,
}
fn default_cache_type() -> String { "STRING".into() }

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TritonComponent {
    pub kind: String, // "exe" | "lib"
    #[serde(default)]
    pub link: Vec<LinkEntry>,
    /// Preprocessor defs applied to this component (e.g. "GLM_ENABLE_EXPERIMENTAL").
    #[serde(default)]
    pub defines: Vec<String>,
    /// Names of deps (as they appear in this component's `link`) to **re-export** PUBLICly.
    /// Any component that depends on this one will inherit these usage requirements.
    #[serde(default)]
    pub exports: Vec<String>,
}

/// Allow three forms inside `components.<name>.link`:
/// 1) "sdl2"
/// 2) { "name": "rmlui", "package": "RmlUi", "target": "RmlUi::RmlUi" }
/// 3) { "name": "filament", "targets": ["filament","utils","math"] }
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum LinkEntry {
    Name(String),
    Named {
        name: String,
        #[serde(default)]
        package: Option<String>,
        #[serde(default)]
        targets: Option<Vec<String>>,
    },
    // Kept for backwards compatibility if you ever used a map form earlier
    Map(BTreeMap<String, LinkHint>),
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LinkHint {
    #[serde(default)]
    pub package: Option<String>,
    #[serde(default)]
    pub targets: Option<Vec<String>>,
}

impl LinkEntry {
    /// Canonicalize into (name, package_hint, first_target_hint)
    pub fn normalize(&self) -> (String, Option<String>) {
        match self {
            LinkEntry::Name(n) => (n.clone(), None),
            LinkEntry::Named { name, package, .. } =>
                (name.clone(), package.clone()),
            LinkEntry::Map(map) => {
                if let Some((k, v)) = map.iter().next() {
                    (k.clone(), v.package.clone())
                } else {
                    ("".into(), None)
                }
            }
        }
    }

    /// Return all explicit targets if provided (for multi-target git/vcpkg entries).
    pub fn all_targets(&self) -> Vec<String> {
        match self {
            LinkEntry::Named { targets, .. } => {
                if let Some(ts) = targets {
                    ts.clone()
                } else {
                    vec![]
                }
            }
            LinkEntry::Map(map) => {
                if let Some((_k, v)) = map.iter().next() {
                    if let Some(ts) = &v.targets {
                        return ts.clone();
                    }
                }
                vec![]
            }
            LinkEntry::Name(_) => vec![],
        }
    }
}
