use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Serialize, Deserialize, Debug, Default, Clone)]
pub struct TritonComponent {
    /// "exe" or "lib"
    #[serde(default = "default_kind")]
    pub kind: String,
    /// vcpkg package deps
    #[serde(default)]
    pub deps: Vec<String>,
    /// linked components (targets)
    #[serde(default)]
    pub comps: Vec<String>,
}

fn default_kind() -> String { "exe".into() }

#[derive(Serialize, Deserialize, Debug, Default, Clone)]
pub struct TritonRoot {
    pub triplet: String,
    pub generator: String,
    pub cxx_std: String,
    /// Map component name -> metadata
    #[serde(default)]
    pub components: BTreeMap<String, TritonComponent>,
}

/* ---------------- vcpkg manifest ---------------- */

#[derive(Serialize, Deserialize, Debug, Default, Clone)]
pub struct VcpkgManifest {
    pub name: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub dependencies: Vec<Dependency>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(untagged)]
pub enum Dependency {
    Name(String),
    Detailed(DependencyDetail),
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct DependencyDetail {
    pub name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub features: Vec<String>,
    #[serde(
        rename = "default-features",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub default_features: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platform: Option<String>,
}

/* Equality helpers */

pub fn dep_eq(a: &Dependency, b: &Dependency) -> bool {
    match (a, b) {
        (Dependency::Name(x), Dependency::Name(y)) => x == y,
        (Dependency::Detailed(x), Dependency::Detailed(y)) => {
            x.name == y.name
                && x.features == y.features
                && x.host.unwrap_or(false) == y.host.unwrap_or(false)
                && x.default_features.unwrap_or(true) == y.default_features.unwrap_or(true)
        }
        // Name("pkg") equals Detailed{name:"pkg"} with no features and no flags
        (Dependency::Name(x), Dependency::Detailed(y)) => {
            x == &y.name
                && y.features.is_empty()
                && y.host.is_none()
                && y.default_features.is_none()
        }
        (Dependency::Detailed(x), Dependency::Name(y)) => {
            &x.name == y && x.features.is_empty() && x.host.is_none() && x.default_features.is_none()
        }
    }
}
