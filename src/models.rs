use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CMakeCacheEntry {
    pub var: String,     // e.g. "BUILD_SHARED_LIBS"
    pub val: String,     // e.g. "ON"
    pub typ: String,     // e.g. "BOOL"
}

// A global dependency can be a plain vcpkg name or a vendored git dep
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RootDep {
    Name(String),
    Git(GitDep),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitDep {
    pub repo: String,             // "owner/name"
    pub name: String,             // local folder under third_party and dep name for linking
    #[serde(default)]
    pub branch: Option<String>,
    // Optional: allow a known CMake target name to link to automatically (still optional)
    #[serde(default)]
    pub target: Option<String>,
    #[serde(default)]
    pub cmake: Vec<CMakeCacheEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TritonComponent {
    pub kind: String,           // "exe" or "lib"
    #[serde(default)]
    pub link: Vec<String>,      // names that must be resolved from root.deps
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TritonRoot {
    pub app_name: String,
    pub triplet: String,
    pub generator: String,
    pub cxx_std: String,
    #[serde(default)]
    pub deps: Vec<RootDep>,
    #[serde(default)]
    pub components: HashMap<String, TritonComponent>,
}
