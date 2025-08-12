use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitDep {
    pub repo: String,             // e.g. "nlohmann/json"
    pub name: String,             // folder name under third_party (e.g. "json")
    pub target: Option<String>,   // optional cmake target to link
    pub branch: Option<String>,   // optional branch/tag
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TritonComponent {
    pub kind: String,       // "exe" or "lib"
    pub deps: Vec<String>,  // vcpkg package names
    pub comps: Vec<String>, // component links
    pub git: Vec<GitDep>,   // vendored git deps
}
impl Default for TritonComponent {
    fn default() -> Self {
        Self { kind: "lib".into(), deps: vec![], comps: vec![], git: vec![] }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TritonRoot {
    pub app_name: String,
    pub triplet: String,
    pub generator: String,
    pub cxx_std: String,
    pub components: BTreeMap<String, TritonComponent>,
}

// vcpkg manifest types
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VcpkgManifest {
    pub name: String,
    pub version: String,
    pub dependencies: Vec<Dependency>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Dependency {
    Name(String),
    Detailed(DependencyDetail),
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DependencyDetail {
    pub name: String,
    #[serde(default)]
    pub features: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_features: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub platform: Option<String>,
}

// helper to compare deps
pub fn dep_eq(a: &Dependency, b: &Dependency) -> bool {
    match (a, b) {
        (Dependency::Name(x), Dependency::Name(y)) => x == y,
        (Dependency::Detailed(x), Dependency::Detailed(y)) => {
            x.name == y.name && x.features == y.features && x.host == y.host
        }
        _ => false,
    }
}
