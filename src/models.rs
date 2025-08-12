use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Serialize, Deserialize, Debug, Default)]
pub struct VcpkgManifest {
    pub name: String,
    #[serde(default)]
    pub version: String, // "version" is accepted; you can also use "version-string"
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
    #[serde(default)]
    pub features: Vec<String>,
    #[serde(default)]
    pub default: Option<bool>,
    #[serde(default)]
    pub host: Option<bool>, // <-- host tools go here
}

#[derive(Serialize, Deserialize, Debug, Default)]
pub struct TritonComponent {
    #[serde(default = "default_kind")]
    pub kind: String, // "exe" or "lib"
    #[serde(default)]
    pub deps: Vec<String>,
}

fn default_kind() -> String {
    "exe".into()
}

#[derive(Serialize, Deserialize, Debug, Default)]
pub struct TritonRoot {
    pub triplet: String,
    pub generator: String,
    pub cxx_std: String,
    pub components: BTreeMap<String, TritonComponent>,
}

// helper equality for deps
pub fn dep_eq(a: &Dependency, b: &Dependency) -> bool {
    match (a, b) {
        (Dependency::Name(x), Dependency::Name(y)) => x == y,
        (Dependency::Detailed(x), Dependency::Detailed(y)) => {
            x.name == y.name && x.features == y.features && x.host.unwrap_or(false) == y.host.unwrap_or(false)
        }
        (Dependency::Name(x), Dependency::Detailed(y)) => x == &y.name && !y.host.unwrap_or(false) && y.features.is_empty(),
        (Dependency::Detailed(x), Dependency::Name(y)) => &x.name == y && !x.host.unwrap_or(false) && x.features.is_empty(),
    }
}
