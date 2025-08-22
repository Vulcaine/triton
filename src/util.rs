use anyhow::{ Context, Result};
use serde::{de::DeserializeOwned, Serialize};
use std::fs;
use std::{path::{Path}};
use std::process::Command;

use crate::models::{DepSpec, TritonComponent, TritonRoot};
use crate::templates::component_cmakelists;

#[derive(Debug, Clone, Copy)]
pub enum Change {
    Created,
    Modified,
    Unchanged,
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

/// Convert paths to a form that plays nicely with CMake and Windows shells.
/// - Strip leading verbatim prefix (`\\?\` or `//?/`) if present (CMake 4.1+ often uses this).
/// - On Windows, return backslashes `\`.
/// - On non-Windows, return forward slashes `/`.
pub fn normalize_path<P: AsRef<Path>>(p: P) -> String {
    let mut s = p.as_ref().to_string_lossy().into_owned();

    // Strip Windows verbatim prefixes if present
    if s.starts_with(r"\\?\") {
        // remove the leading \\?\
        s = s.replacen(r"\\?\", "", 1);
    } else if s.starts_with("//?/") {
        // remove the leading //?/
        s = s.replacen("//?/", "", 1);
    }

    // Normalize separators per-platform
    if cfg!(windows) {
        // Use backslashes on Windows
        s = s.replace('/', r"\");
    } else {
        // Use forward slashes elsewhere
        s = s.replace('\\', "/");
    }

    s
}

pub fn ensure_component_scaffold(name: &str) -> anyhow::Result<()> {
    use std::fs;
    use std::io::Write;
    use std::path::Path;

    // components/<name>/
    let base = Path::new("components").join(name);
    fs::create_dir_all(&base)?;

    // components/<name>/src/<name> and components/<name>/include/<name>
    let src_dir = base.join("src").join(name);
    let inc_dir = base.join("include").join(name);
    fs::create_dir_all(&src_dir)?;
    fs::create_dir_all(&inc_dir)?;

    // Minimal placeholder header so includes like <Name/Name.hpp> resolve.
    let header_path = inc_dir.join(format!("{name}.hpp"));
    if !header_path.exists() {
        let mut f = fs::File::create(&header_path)?;
        writeln!(f, "#pragma once")?;
        writeln!(f, "// {} public headers live under this folder.", name)?;
    }

    // Minimal placeholder source (no main()).
    let source_path = src_dir.join(format!("{name}.cpp"));
    if !source_path.exists() {
        let mut f = fs::File::create(&source_path)?;
        writeln!(f, "#include <{0}/{0}.hpp>", name)?;
        writeln!(f, "// Implementation files for {} live here.", name)?;
    }

    Ok(())
}


pub fn is_dep(root: &TritonRoot, name: &str) -> bool {
    root.deps.iter().any(|d| match d {
        DepSpec::Simple(n) => n == name,
        DepSpec::Git(g) => g.name == name,
        DepSpec::Detailed(dd) => dd.name == name,
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
