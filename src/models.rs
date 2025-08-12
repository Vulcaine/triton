use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TritonRoot {
    pub app_name: String,
    pub triplet: String,
    pub generator: String,
    pub cxx_std: String,
    pub deps: Vec<RootDep>,
    pub components: std::collections::BTreeMap<String, TritonComponent>,
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
    #[serde(default)]
    pub target: Option<String>,
    #[serde(default)]
    pub cmake: Vec<CMakeCacheEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CMakeCacheEntry {
    pub var: String,
    pub val: String,
    #[serde(default = "default_cache_type")]
    pub typ: String,
}
fn default_cache_type() -> String { "STRING".into() }

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TritonComponent {
    pub kind: String, // "exe" | "lib"
    #[serde(default)]
    pub link: Vec<LinkEntry>,
}

/// Allow three forms inside `components.<name>.link`:
/// 1) "sdl2"
/// 2) { "name": "rmlui", "package": "RmlUi", "target": "RmlUi::RmlUi" }
/// 3) { "rmlui": { "package": "RmlUi", "target": "RmlUi::RmlUi" } }  (shorthand)
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
    },
    Map(BTreeMap<String, LinkHint>),
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LinkHint {
    #[serde(default)]
    pub package: Option<String>,
    #[serde(default)]
    pub target: Option<String>,
}

impl LinkEntry {
    /// Canonicalize any variant into (name, package_hint, target_hint)
    pub fn normalize(&self) -> (String, Option<String>, Option<String>) {
        match self {
            LinkEntry::Name(n) => (n.clone(), None, None),
            LinkEntry::Named { name, package, target } =>
                (name.clone(), package.clone(), target.clone()),
            LinkEntry::Map(map) => {
                if let Some((k, v)) = map.iter().next() {
                    (k.clone(), v.package.clone(), v.target.clone())
                } else {
                    ("".into(), None, None)
                }
            }
        }
    }
}
