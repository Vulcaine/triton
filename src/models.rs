use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TritonRoot {
    pub app_name: String,
    pub generator: String,
    pub cxx_std: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub c_std: Option<String>,

    /// Top-level dependencies (vcpkg or git). Supports both simple and detailed forms.
    pub deps: Vec<DepSpec>,

    pub components: BTreeMap<String, TritonComponent>,

    #[serde(default)]
    pub scripts: HashMap<String, String>,
}

/// Dependency specification (hybrid form).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum DepSpec {
    /// Simple string form: `"lua"`
    Simple(String),

    /// Git dependency (structured)
    Git(GitDep),

    /// Detailed form with filters
    Detailed(DepDetailed),
}

impl DepSpec {
    /// Return the canonical name of this dependency.
    pub fn name(&self) -> &str {
        match self {
            DepSpec::Simple(n) => n,
            DepSpec::Git(g) => &g.name,
            DepSpec::Detailed(d) => &d.name,
        }
    }
}

/// Git repository dependency
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

/// More detailed dep form (with filters).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DepDetailed {
    pub name: String,
    /// Restrict to operating systems (values: "windows", "linux", "macos")
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub os: Vec<String>,
    /// Override vcpkg package name (if different from `name`)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package: Option<String>,
    /// Restrict to vcpkg triplets
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub triplet: Vec<String>,
    /// Additional vcpkg features
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub features: Vec<String>,
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
fn default_cache_type() -> String {
    "STRING".into()
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TritonComponent {
    pub kind: String, // "exe" | "lib"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cxx_std: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub c_std: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub link: Vec<LinkEntry>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub system_libs: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub defines: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exports: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub resources: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub include_dirs: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sources: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arch: Option<String>,
    #[serde(default, skip_serializing_if = "LinkOptions::is_none")]
    pub link_options: LinkOptions,
    #[serde(default, skip_serializing_if = "VendorLibs::is_none")]
    pub vendor_libs: VendorLibs,
    /// Asset paths (relative to the component root) to stage next to the target
    /// incrementally.  Directories are mirrored; files are copied if changed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub assets: Vec<String>,
}

/// Linker options — either a flat list (all platforms) or a map keyed by "linux"/"macos"/"windows".
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(untagged)]
pub enum LinkOptions {
    #[default]
    None,
    All(Vec<String>),
    PerPlatform(BTreeMap<String, Vec<String>>),
}

impl LinkOptions {
    pub fn is_none(&self) -> bool { matches!(self, LinkOptions::None) }
}

/// Vendor library paths — either a flat list (all platforms) or a map keyed by "linux"/"macos"/"windows".
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(untagged)]
pub enum VendorLibs {
    #[default]
    None,
    All(Vec<String>),
    PerPlatform(BTreeMap<String, Vec<String>>),
}

impl VendorLibs {
    pub fn is_none(&self) -> bool { matches!(self, VendorLibs::None) }
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
        #[serde(default, skip_serializing_if = "Option::is_none")]
        package: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        targets: Option<Vec<String>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        visibility: Option<String>,
    },
    Map(BTreeMap<String, LinkHint>),
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LinkHint {
    #[serde(default)]
    pub package: Option<String>,
    #[serde(default)]
    pub targets: Option<Vec<String>>,
    #[serde(default)]
    pub visibility: Option<String>,
}

impl LinkEntry {
    /// Canonicalize into (name, package_hint)
    pub fn normalize(&self) -> (String, Option<String>) {
        match self {
            LinkEntry::Name(n) => (n.clone(), None),
            LinkEntry::Named { name, package, .. } => (name.clone(), package.clone()),
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
            LinkEntry::Named { targets, .. } => targets.clone().unwrap_or_default(),
            LinkEntry::Map(map) => {
                if let Some((_k, v)) = map.iter().next() {
                    return v.targets.clone().unwrap_or_default();
                }
                vec![]
            }
            LinkEntry::Name(_) => vec![],
        }
    }

    pub fn is_public(&self) -> bool {
        let raw = match self {
            LinkEntry::Name(_) => None,
            LinkEntry::Named { visibility, .. } => visibility.as_deref(),
            LinkEntry::Map(map) => map.iter().next().and_then(|(_k, v)| v.visibility.as_deref()),
        };
        !matches!(raw.map(|s| s.trim().to_ascii_lowercase()), Some(v) if v == "private")
    }
}
