use anyhow::{anyhow, Context, Result};
use serde::{de::DeserializeOwned, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::models::{RootDep, TritonComponent, TritonRoot};
use crate::templates::component_cmakelists;

#[derive(Debug, Clone, Copy)]
pub enum Change {
    Created,
    Modified,
    Unchanged,
}

pub fn vcpkg_exe_path() -> String {
    let mut p = PathBuf::from("vcpkg");
    p.push(if cfg!(windows) { "vcpkg.exe" } else { "vcpkg" });
    p.to_string_lossy().to_string()
}

/// Read a file to string, returning `None` if it doesn't exist or can't be read.
pub fn read_to_string_opt<P: AsRef<Path>>(p: P) -> Option<String> {
    fs::read_to_string(p.as_ref()).ok()
}

/// Write text only if absent or different; returns what happened.
pub fn write_text_if_changed<P: AsRef<Path>>(p: P, content: &str) -> Result<Change> {
    let p = p.as_ref();
    if !p.exists() {
        if let Some(parent) = p.parent() { fs::create_dir_all(parent)?; }
        fs::write(p, content)?;
        return Ok(Change::Created);
    }
    let existing = fs::read_to_string(p)?;
    if existing == content {
        Ok(Change::Unchanged)
    } else {
        fs::write(p, content)?;
        Ok(Change::Modified)
    }
}

/// JSON pretty-print writer on top of `write_text_if_changed`.
pub fn write_json_pretty_changed<P: AsRef<Path>, T: ?Sized + Serialize>(p: P, value: &T) -> Result<Change> {
    let s = serde_json::to_string_pretty(value)?;
    write_text_if_changed(p, &s)
}

pub fn read_json<P: AsRef<Path>, T: DeserializeOwned>(p: P) -> Result<T> {
    let s = fs::read_to_string(p.as_ref())
        .with_context(|| format!("reading {}", p.as_ref().display()))?;
    Ok(serde_json::from_str(&s)?)
}

pub fn run(cmd: &str, args: &[&str], cwd: &str) -> Result<()> {
    let status = Command::new(cmd)
        .args(args)
        .current_dir(cwd)
        .status()
        .with_context(|| format!("failed to spawn {cmd}"))?;
    if !status.success() {
        return Err(anyhow!("command failed: {} {:?}", cmd, args));
    }
    Ok(())
}

/* ------------------------- new shared helpers ------------------------- */

/// Ensure a component's folder layout exists and create `CMakeLists.txt` if missing.
pub fn ensure_component_scaffold(name: &str) -> Result<()> {
    let base = format!("components/{name}");
    fs::create_dir_all(format!("{base}/src"))?;
    fs::create_dir_all(format!("{base}/include"))?;
    let cm = format!("{base}/CMakeLists.txt");
    if !Path::new(&cm).exists() {
        write_text_if_changed(&cm, &component_cmakelists())
            .with_context(|| format!("writing {}", cm))?;
    }
    Ok(())
}

/// Return true if `name` is one of the project deps (vcpkg or git).
pub fn is_dep(root: &TritonRoot, name: &str) -> bool {
    root.deps.iter().any(|d| match d {
        RootDep::Name(n) => n == name,
        RootDep::Git(g) => g.name == name,
    })
}

/// Returns true if component's link list already contains an entry that normalizes to `want_name`.
pub fn has_link_to_name(comp: &TritonComponent, want_name: &str) -> bool {
    comp.link.iter().any(|e| {
        let (n, _pkg, _tgt) = e.normalize();
        n == want_name
    })
}
