use anyhow::{Context, Result};
use serde::{de::DeserializeOwned, Serialize};
use std::fs;
use std::path::Path;
use std::process::Command;

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
