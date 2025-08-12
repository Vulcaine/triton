use anyhow::{Context, Result};
use regex::Regex;
use std::fs;
use walkdir::WalkDir;

use crate::models::{TritonComponent, TritonRoot};

pub fn pkg_case(s: &str) -> String {
    // MVP: return as-is (many vcpkg config packages are lowercase)
    s.to_string()
}

pub fn rewrite_component_cmake(name: &str, comp: &TritonComponent) -> Result<()> {
    let p = format!("components/{name}/CMakeLists.txt");
    let cmake = fs::read_to_string(&p)
        .with_context(|| format!("reading {}", p))?;

    let mut lines = Vec::new();
    for pkg in &comp.deps {
        let pcase = pkg_case(pkg);
        lines.push(format!("find_package({} CONFIG REQUIRED)", pcase));
        lines.push(format!("target_link_libraries(${{_comp_name}} PRIVATE {}::{} )", pcase, pcase));
    }
    let block = if lines.is_empty() { "# (none)".into() } else { lines.join("\n") };

    let re = Regex::new(r"(?s)# ## triton:deps begin.*?# ## triton:deps end").unwrap();
    let replacement = format!("# ## triton:deps begin\n{}\n# ## triton:deps end", block);
    let updated = if re.is_match(&cmake) {
        re.replace(&cmake, replacement.as_str()).to_string()
    } else {
        format!("{cmake}\n{replacement}\n")
    };
    fs::write(&p, updated)?;
    Ok(())
}

pub fn regenerate_root_cmake(_root: &TritonRoot) -> Result<()> {
    let mut existing: Vec<String> = Vec::new();
    for entry in WalkDir::new("components").min_depth(1).max_depth(1) {
        let e = entry?;
        if e.file_type().is_dir() {
            existing.push(e.file_name().to_string_lossy().into_owned());
        }
    }
    existing.sort();

    let cmake = fs::read_to_string("CMakeLists.txt")?;
    let mut lines = String::new();
    for c in &existing {
        lines.push_str(&format!("add_subdirectory(components/{})\n", c));
    }
    if lines.is_empty() { lines = "# (no components)".into(); }

    let re = Regex::new(r"(?s)# ## triton:components begin.*?# ## triton:components end").unwrap();
    let replacement = format!("# ## triton:components begin\n{}\n# ## triton:components end", lines);
    let updated = if re.is_match(&cmake) {
        re.replace(&cmake, replacement.as_str()).to_string()
    } else {
        format!("{cmake}\n{replacement}\n")
    };
    fs::write("CMakeLists.txt", updated)?;
    Ok(())
}
