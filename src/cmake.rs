use anyhow::{Context, Result};
use regex::{NoExpand, Regex};
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

use crate::models::{TritonComponent, TritonRoot};
use crate::util::read_json;

/// Replace the `# ## triton:components ...` block at repo root.
pub fn regenerate_root_cmake(_root: &TritonRoot) -> Result<()> {
    // discover existing component dirs (on disk)
    let mut existing: Vec<String> = Vec::new();
    if Path::new("components").exists() {
        for entry in WalkDir::new("components").min_depth(1).max_depth(1) {
            let e = entry?;
            if e.file_type().is_dir() {
                existing.push(e.file_name().to_string_lossy().into_owned());
            }
        }
    }
    existing.sort();

    let mut cmake = fs::read_to_string("CMakeLists.txt")?;
    let mut lines = String::new();
    for c in &existing {
        lines.push_str(&format!("add_subdirectory(components/{})\n", c));
    }
    if lines.is_empty() {
        lines = "# (no components)".into();
    }

    let re = Regex::new(r"(?s)# ## triton:components begin.*?# ## triton:components end").unwrap();
    let replacement = format!("# ## triton:components begin\n{}\n# ## triton:components end", lines);
    cmake = re.replace(&cmake, NoExpand(&replacement)).to_string();
    fs::write("CMakeLists.txt", cmake)?;
    Ok(())
}

pub fn rewrite_component_cmake(name: &str, comp: &TritonComponent) -> Result<()> {
    use regex::Regex;
    use std::path::Path;

    #[derive(Debug, Clone, serde::Deserialize)]
    struct ThirdPartyDep {
        repo: String,
        name: String,
        #[serde(default)]
        target: Option<String>,
        #[serde(default)]
        branch: Option<String>,
    }

    fn load_third_party_list(component: &str) -> Vec<ThirdPartyDep> {
        let p = format!("components/{component}/third_party.json");
        let path = std::path::Path::new(&p);
        if path.exists() {
            crate::util::read_json(path).unwrap_or_default()
        } else {
            vec![]
        }
    }

    let p = format!("components/{name}/CMakeLists.txt");
    let mut cmake = fs::read_to_string(&p).with_context(|| format!("reading {}", p))?;

    // Load root to know triplet and vcpkg installed prefix (for fallbacks)
    let root: TritonRoot = read_json("triton.json")?;
    let vcpkg_inst = Path::new("vcpkg_installed").join(&root.triplet);
    let share_dir = vcpkg_inst.join("share");
    let lib_dir = vcpkg_inst.join("lib");

    let mut lines: Vec<String> = Vec::new();

    // Resolve local target name robustly
    lines.push("# --- triton: resolve local target name ---".into());
    lines.push(r#"if(NOT DEFINED _comp_name)"#.into());
    lines.push(r#"  get_filename_component(_comp_name "${CMAKE_CURRENT_SOURCE_DIR}" NAME)"#.into());
    lines.push("endif()".into());
    lines.push("".into());

    // Inter-component deps first
    for c in &comp.comps {
        lines.push(format!("target_link_libraries(${{_comp_name}} PRIVATE {})", c));
    }

    // Third-party vendored deps
    let third_party = load_third_party_list(name);
    for tp in &third_party {
        lines.push(format!(
            "add_subdirectory(\"${{PROJECT_SOURCE_DIR}}/third_party/{}\" EXCLUDE_FROM_ALL)",
            tp.name
        ));
        if let Some(t) = &tp.target {
            lines.push(format!("target_link_libraries(${{_comp_name}} PRIVATE {})", t));
        } else {
            lines.push(format!(
                "# TODO(triton): set correct target for {} and uncomment next line",
                tp.repo
            ));
            lines.push(format!(
                "# target_link_libraries(${{_comp_name}} PRIVATE {})",
                tp.name
            ));
        }
    }

    // Partition vcpkg deps: Boost subports vs others
    let mut boost_components: Vec<String> = Vec::new();
    let mut others: Vec<String> = Vec::new();
    for pkg in &comp.deps {
        let lc = pkg.to_ascii_lowercase();
        if lc == "boost" {
            continue;
        }
        if lc.starts_with("boost-") {
            let comp_name = lc.trim_start_matches("boost-").replace('-', "_");
            boost_components.push(comp_name);
        } else {
            others.push(pkg.clone());
        }
    }
    boost_components.sort();
    boost_components.dedup();

    // Boost group
    if !boost_components.is_empty() {
        lines.push(format!(
            "find_package(Boost CONFIG COMPONENTS {} REQUIRED)",
            boost_components.join(" ")
        ));
        lines.push("target_link_libraries(${_comp_name} PRIVATE Boost::headers)".into());
        for c in &boost_components {
            lines.push(format!("target_link_libraries(${{_comp_name}} PRIVATE Boost::{} )", c));
        }
    } else if comp.deps.iter().any(|d| d.eq_ignore_ascii_case("boost")) {
        lines.push("find_package(Boost CONFIG REQUIRED)".into());
        lines.push("target_link_libraries(${_comp_name} PRIVATE Boost::headers)".into());
    }

    // Other vcpkg deps (as before)
    for pkg in &others {
        let (find_name, link_targets) = canonical_pkg(pkg);
        let share_key = pkg.to_ascii_lowercase();

        if has_config_package(&share_dir, &share_key) {
            lines.push(format!("find_package({} CONFIG REQUIRED)", find_name));
            if !link_targets.is_empty() {
                lines.push(format!(
                    "target_link_libraries(${{_comp_name}} PRIVATE {})",
                    link_targets.join(" ")
                ));
            }
            continue;
        }

        if let Some(usage) = read_usage_hint(&share_dir, &share_key) {
            lines.push(format!("# vcpkg usage for {}:", pkg));
            for ln in usage.lines() {
                lines.push(format!("# {}", ln));
            }
            lines.push("# (triton added a generic include+lib fallback below)".into());
        }

        lines.push(
            r#"target_include_directories(${_comp_name} PRIVATE "${VCPKG_INSTALLED_DIR}/${VCPKG_TARGET_TRIPLET}/include")"#.into(),
        );

        let guessed = guess_libs(&lib_dir, pkg);
        if guessed.is_empty() {
            lines.push(format!(
                "# TODO(triton): could not find config for '{}'; add concrete libs from ${{{{VCPKG_INSTALLED_DIR}}}}/${{{{VCPKG_TARGET_TRIPLET}}}}/lib if needed",
                pkg
            ));
        } else {
            let joined = guessed.iter().map(|p| cmake_path(p)).collect::<Vec<_>>().join(" ");
            lines.push(format!("target_link_libraries(${{_comp_name}} PRIVATE {})", joined));
        }
    }

    let block = if lines.is_empty() { "# (none)".to_string() } else { lines.join("\n") };
    let re = Regex::new(r"(?s)# ## triton:deps begin.*?# ## triton:deps end").unwrap();
    let replacement = format!("# ## triton:deps begin\n{}\n# ## triton:deps end", block);
    cmake = re.replace(&cmake, regex::NoExpand(&replacement)).to_string();
    fs::write(&p, cmake)?;
    Ok(())
}


fn has_config_package(share_dir: &Path, pkg_key: &str) -> bool {
    let d = share_dir.join(pkg_key);
    if !d.exists() {
        return false;
    }
    for e in WalkDir::new(&d).max_depth(2).into_iter().filter_map(|e| e.ok()) {
        let p = e.path();
        if p.is_file() {
            if let Some(name) = p.file_name().and_then(|s| s.to_str()) {
                if name.ends_with("Config.cmake") {
                    return true;
                }
            }
        }
    }
    false
}

fn read_usage_hint(share_dir: &Path, pkg_key: &str) -> Option<String> {
    let p = share_dir.join(pkg_key).join("usage");
    fs::read_to_string(p).ok()
}

fn guess_libs(lib_dir: &Path, pkg: &str) -> Vec<PathBuf> {
    if !lib_dir.exists() { return vec![]; }
    let mut out = Vec::new();
    let needle = pkg.to_ascii_lowercase();
    for e in WalkDir::new(lib_dir).min_depth(1).max_depth(1).into_iter().filter_map(|e| e.ok()) {
        let p = e.path();
        if !p.is_file() { continue; }
        if let Some(ext) = p.extension().and_then(|s| s.to_str()) {
            let ext_lc = ext.to_ascii_lowercase();
            if ext_lc == "lib" || ext_lc == "a" {
                if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
                    let stem_lc = stem.to_ascii_lowercase();
                    if stem_lc.contains(&needle) || stem_lc.starts_with(&format!("lib{}", needle)) {
                        out.push(p.to_path_buf());
                    }
                }
            }
        }
    }
    out
}

fn cmake_path(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

/// Map common vcpkg ports to their canonical CMake package + target(s).
/// Default is `<pkg>` and `<pkg>::<pkg>`.
fn canonical_pkg(pkg: &str) -> (String, Vec<String>) {
    match pkg.to_ascii_lowercase().as_str() {
        "boost" => ("Boost".into(), vec!["Boost::headers".into()]),
        "openssl" => ("OpenSSL".into(), vec!["OpenSSL::SSL".into(), "OpenSSL::Crypto".into()]),
        other => {
            let name = other.to_string();
            (name.clone(), vec![format!("{}::{}", name, name)])
        }
    }
}
