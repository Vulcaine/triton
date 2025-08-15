use anyhow::{anyhow, Context, Result};
use serde::{de::DeserializeOwned, Serialize};
use std::fs;
use std::{env, path::{Path, PathBuf}};
use std::process::Command;

use crate::models::{RootDep, TritonComponent, TritonRoot};
use crate::templates::component_cmakelists;

#[derive(Debug, Clone, Copy)]
pub enum Change {
    Created,
    Modified,
    Unchanged,
}

fn which_in_path(candidates: &[&str]) -> Option<PathBuf> {
    let path = env::var_os("PATH")?;
    for dir in env::split_paths(&path) {
        for name in candidates {
            let p = dir.join(name);
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

/// Locate vcpkg:
/// 1) TRITON_VCPKG_EXE
/// 2) VCPKG_EXE
/// 3) PATH (vcpkg.exe / vcpkg.bat / vcpkg.cmd on Windows, vcpkg on *nix)
pub fn vcpkg_exe_path() -> Result<PathBuf> {
    if let Some(p) = env::var_os("TRITON_VCPKG_EXE").filter(|v| !v.is_empty()) {
        let pb = PathBuf::from(p);
        if pb.is_file() { return Ok(pb); }
    }
    if let Some(p) = env::var_os("VCPKG_EXE").filter(|v| !v.is_empty()) {
        let pb = PathBuf::from(p);
        if pb.is_file() { return Ok(pb); }
    }
    if cfg!(windows) {
        if let Some(p) = which_in_path(&["vcpkg.exe", "vcpkg.bat", "vcpkg.cmd"]) {
            return Ok(p);
        }
    } else if let Some(p) = which_in_path(&["vcpkg"]) {
        return Ok(p);
    }
    Err(anyhow!("Could not find vcpkg in TRITON_VCPKG_EXE / VCPKG_EXE / PATH"))
}

pub fn read_to_string_opt<P: AsRef<Path>>(p: P) -> Option<String> {
    fs::read_to_string(p.as_ref()).ok()
}

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

pub fn write_json_pretty_changed<P: AsRef<Path>, T: ?Sized + Serialize>(p: P, value: &T) -> Result<Change> {
    let s = serde_json::to_string_pretty(value)?;
    write_text_if_changed(p, &s)
}

pub fn read_json<P: AsRef<Path>, T: DeserializeOwned>(p: P) -> Result<T> {
    let s = fs::read_to_string(p.as_ref())
        .with_context(|| format!("reading {}", p.as_ref().display()))?;
    Ok(serde_json::from_str(&s)?)
}

pub fn run(exe: impl AsRef<Path>, args: &[&str], cwd: impl AsRef<Path>) -> Result<()> {
    let status = Command::new(exe.as_ref())
        .current_dir(cwd)
        .args(args)
        .status()
        .map_err(|e| anyhow::anyhow!("failed to spawn {}: {e}", exe.as_ref().display()))?;
    if !status.success() {
        return Err(anyhow::anyhow!("command exited with {}", status));
    }
    Ok(())
}

pub fn ensure_component_scaffold(name: &str) -> Result<()> {
    let base = format!("components/{name}");
    fs::create_dir_all(format!("{base}/src"))?;
    fs::create_dir_all(format!("{base}/include"))?;

    let cm = format!("{base}/CMakeLists.txt");
    if !Path::new(&cm).exists() {
        let is_test = name.eq_ignore_ascii_case("tests");
        write_text_if_changed(&cm, &component_cmakelists(is_test))
            .with_context(|| format!("writing {}", cm))?;
    }

    Ok(())
}

pub fn is_dep(root: &TritonRoot, name: &str) -> bool {
    root.deps.iter().any(|d| match d {
        RootDep::Name(n) => n == name,
        RootDep::Git(g) => g.name == name,
    })
}

pub fn has_link_to_name(comp: &TritonComponent, want: &str) -> bool {
    comp.link.iter().any(|e| {
        let (n, _pkg) = e.normalize();
        n == want
    })
}

pub fn cmake_quote(val: &str) -> String {
    let s = val.trim().replace('"', "\\\"");
    format!("\"{}\"", s)
}

pub fn infer_cmake_type(val: &str) -> &'static str {
    match val.to_ascii_uppercase().as_str() {
        "ON" | "OFF" | "TRUE" | "FALSE" | "YES" | "NO" => "BOOL",
        _ => "STRING",
    }
}

pub fn split_kv(raw: &str) -> (String, String) {
    if let Some(idx) = raw.find('=') {
        let (k, v) = raw.split_at(idx);
        let key = k.trim().to_string();
        let mut val = v[1..].trim().to_string();
        if val.starts_with('"') && val.ends_with('"') && val.len() >= 2 {
            val = val[1..val.len() - 1].to_string();
        }
        (key, if val.is_empty() { "ON".into() } else { val })
    } else {
        (raw.trim().to_string(), "ON".to_string())
    }
}
