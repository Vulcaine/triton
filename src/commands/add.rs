use anyhow::{Context, Result};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::models::{DepSpec, GitDep, LinkEntry, TritonComponent, TritonRoot};
use crate::util::read_json;

/// Simple transactional FS
struct FsTxn {
    backups: Vec<Backup>,
    created_files: Vec<PathBuf>,
    committed: bool,
}
struct Backup {
    path: PathBuf,
    existed: bool,
    original: Vec<u8>,
}
impl FsTxn {
    fn new() -> Self {
        Self { backups: Vec::new(), created_files: Vec::new(), committed: false }
    }
    fn backup_if_needed(&mut self, path: &Path) -> io::Result<()> {
        if self.backups.iter().any(|b| b.path == path) {
            return Ok(());
        }
        let existed = path.exists();
        let original = if existed { fs::read(path)? } else { Vec::new() };
        self.backups.push(Backup { path: path.to_path_buf(), existed, original });
        Ok(())
    }
    fn write_text(&mut self, path: impl AsRef<Path>, content: &str) -> io::Result<()> {
        let path = path.as_ref();
        self.backup_if_needed(path)?;
        let already = if path.exists() { Some(fs::read_to_string(path)?) } else { None };
        if already.as_deref() != Some(content) {
            if !path.exists() {
                self.created_files.push(path.to_path_buf());
            }
            if let Some(parent) = path.parent() { fs::create_dir_all(parent)?; }
            fs::write(path, content)?;
        }
        Ok(())
    }
    fn commit(mut self) { self.committed = true; }
}
impl Drop for FsTxn {
    fn drop(&mut self) {
        if self.committed { return; }
        for b in self.backups.iter().rev() {
            let _ = if b.existed {
                fs::write(&b.path, &b.original)
            } else {
                fs::remove_file(&b.path)
            };
        }
        for p in self.created_files.iter().rev() {
            let _ = fs::remove_file(p);
        }
    }
}

fn txn_write_json_pretty(txn: &mut FsTxn, path: &str, v: &serde_json::Value) -> Result<()> {
    let s = serde_json::to_string_pretty(v)?;
    txn.write_text(path, &s).with_context(|| format!("writing {}", path))?;
    Ok(())
}

fn parse_pkg_and_component<'a>(pkg: &'a str, component_opt: Option<&'a str>) -> (&'a str, Option<&'a str>) {
    // Support both "pkg->Component" and "pkg:Component" syntax
    if let Some((p, c)) = pkg.split_once("->") {
        let p = p.trim();
        let c = c.trim();
        if !c.is_empty() { return (p, Some(c)); }
        return (p, None);
    }
    // Don't split on ':' if it looks like a URL scheme (https:, git:, ssh:, http:)
    if pkg.starts_with("https:") || pkg.starts_with("http:")
        || pkg.starts_with("git:") || pkg.starts_with("ssh:")
        || pkg.starts_with("git@")
    {
        // For full URLs, component comes after @branch:Component
        // e.g. "https://github.com/org/repo.git@docking:UI"
        if let Some(at_idx) = pkg.find('@') {
            let after_at = &pkg[at_idx + 1..];
            if let Some(colon_idx) = after_at.find(':') {
                let url_with_branch = &pkg[..at_idx + 1 + colon_idx];
                let comp = after_at[colon_idx + 1..].trim();
                if !comp.is_empty() {
                    return (url_with_branch, Some(comp));
                }
            }
        }
        return (pkg, component_opt.map(|s| s.trim()).filter(|s| !s.is_empty()));
    }

    if let Some((p, c)) = pkg.split_once(':') {
        let p = p.trim();
        let c = c.trim();
        if !c.is_empty() { return (p, Some(c)); }
        return (p, None);
    }
    (pkg, component_opt.map(|s| s.trim()).filter(|s| !s.is_empty()))
}

/// Ensure component dirs and CMakeLists exist.
fn ensure_component_scaffold(name: &str, txn: &mut FsTxn) -> Result<()> {
    let base = format!("components/{name}");
    fs::create_dir_all(format!("{base}/src"))?;
    fs::create_dir_all(format!("{base}/include"))?;

    let cm = format!("{base}/CMakeLists.txt");
    if !Path::new(&cm).exists() {
        // minimal template
        let body = r#"cmake_minimum_required(VERSION 3.25)
get_filename_component(_comp_name "${CMAKE_CURRENT_SOURCE_DIR}" NAME)
add_library(${_comp_name})
target_include_directories(${_comp_name} PUBLIC "include")
# ## triton:deps begin
# ## triton:deps end
"#;
        txn.write_text(&cm, body)?;
    }
    Ok(())
}

/// Check if a string looks like a git dependency (contains `/`, or is a full URL).
fn is_git_dep(s: &str) -> bool {
    // Full URLs: https://..., git://..., ssh://...
    if s.starts_with("https://") || s.starts_with("http://")
        || s.starts_with("git://") || s.starts_with("ssh://")
        || s.starts_with("git@")
    {
        return true;
    }
    // Shorthand: org/repo or org/repo.git (but not a Windows path with backslashes)
    s.contains('/') && !s.contains('\\')
}

/// Given user input, extract (repo_field, clone_url, short_name, branch).
///
/// `repo_field` — stored in triton.json (shorthand like "ConfettiFX/The-Forge")
/// `clone_url`  — full HTTPS URL for git clone
///
/// Accepts:
///   - `https://github.com/ConfettiFX/The-Forge.git`
///   - `https://github.com/ConfettiFX/The-Forge.git@v1.63`
///   - `ConfettiFX/The-Forge.git`
///   - `ConfettiFX/The-Forge.git@v1.63`
///   - `ConfettiFX/The-Forge`
///   - `ConfettiFX/The-Forge@v1.63`
fn parse_git_dep(raw: &str) -> (String, String, String, Option<String>) {
    // Split off @branch first
    let (repo_part, branch) = if let Some((r, b)) = raw.split_once('@') {
        (r.to_string(), Some(b.to_string()))
    } else {
        (raw.to_string(), None)
    };

    // Determine if it's already a full URL
    let is_full_url = repo_part.starts_with("https://") || repo_part.starts_with("http://")
        || repo_part.starts_with("git://") || repo_part.starts_with("ssh://")
        || repo_part.starts_with("git@");

    let clone_url = if is_full_url {
        repo_part.clone()
    } else {
        // Shorthand: org/repo or org/repo.git → expand to GitHub HTTPS URL
        let normalized = if repo_part.ends_with(".git") {
            repo_part.clone()
        } else {
            format!("{}.git", repo_part)
        };
        format!("https://github.com/{}", normalized)
    };

    // Extract short name from the last path segment, stripping .git suffix
    let last_segment = repo_part
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or(&repo_part);
    let name = last_segment.strip_suffix(".git").unwrap_or(last_segment).to_string();

    // For triton.json "repo" field, use the shorthand form (org/name) if possible
    let repo_field = if is_full_url {
        extract_github_shorthand(&repo_part).unwrap_or(repo_part)
    } else {
        repo_part.strip_suffix(".git").unwrap_or(&repo_part).to_string()
    };

    (repo_field, clone_url, name, branch)
}

/// Try to extract "org/repo" from a full GitHub URL.
fn extract_github_shorthand(url: &str) -> Option<String> {
    // https://github.com/org/repo.git → org/repo
    let stripped = url
        .strip_prefix("https://github.com/")?
        .strip_suffix(".git")
        .unwrap_or(url.strip_prefix("https://github.com/")?);
    if stripped.contains('/') {
        Some(stripped.to_string())
    } else {
        None
    }
}

/// Clone a git repo into `third_party/<name>/`. If the directory already exists, skip.
fn git_clone(clone_url: &str, name: &str, branch: &Option<String>) -> Result<()> {
    let dest = format!("third_party/{name}");
    if Path::new(&dest).exists() {
        // Check if it has contents (not just a stub dir)
        let is_populated = Path::new(&dest).join(".git").exists()
            || fs::read_dir(&dest).map(|mut d| d.next().is_some()).unwrap_or(false);
        if is_populated {
            eprintln!("  third_party/{name} already exists, skipping clone.");
            return Ok(());
        }
        // Remove empty stub directory before cloning
        fs::remove_dir(&dest).ok();
    }

    fs::create_dir_all("third_party")?;

    let mut cmd = Command::new("git");
    cmd.arg("clone").arg("--depth").arg("1");
    if let Some(b) = branch {
        cmd.arg("--branch").arg(b);
    }
    cmd.arg(clone_url).arg(&dest);

    eprintln!("Cloning {} into third_party/{} ...", clone_url, name);
    let status = cmd.status().context("failed to run git clone")?;
    if !status.success() {
        anyhow::bail!("git clone failed for {}", clone_url);
    }

    Ok(())
}

pub fn handle_add(items: &[String], _features: Option<&str>, _host: bool) -> Result<()> {
    if items.is_empty() { return Ok(()); }

    let mut root: TritonRoot = read_json("triton.json")?;
    fs::create_dir_all("components")?;

    let mut txn = FsTxn::new();

    for it in items {
        let (pkg, link_to_opt) = parse_pkg_and_component(it, None);

        if is_git_dep(pkg) {
            // git dep — parse URL/shorthand, clone, and register
            let (repo, clone_url, name, branch) = parse_git_dep(pkg);

            git_clone(&clone_url, &name, &branch)?;

            if !root.deps.iter().any(|d| matches!(d, DepSpec::Git(g) if g.name == name || g.repo == repo)) {
                root.deps.push(DepSpec::Git(GitDep { repo: repo.clone(), name: name.clone(), branch: branch.clone(), cmake: vec![] }));
            }

            if let Some(dest) = link_to_opt {
                let entry = root.components.entry(dest.to_string())
                    .or_insert(TritonComponent { kind: "lib".into(), ..Default::default() });
                if !entry.link.iter().any(|e| matches!(e, LinkEntry::Name(n) if n == &name)) {
                    entry.link.push(LinkEntry::Name(name.clone()));
                }
                ensure_component_scaffold(dest, &mut txn)?;
            }
        } else {
            // vcpkg dep
            if !root.deps.iter().any(|d| matches!(d, DepSpec::Simple(n) if n == pkg)) {
                root.deps.push(DepSpec::Simple(pkg.to_string()));
            }

            if !Path::new("vcpkg.json").exists() {
                let empty = serde_json::json!({ "name": root.app_name, "version":"0.0.0", "dependencies": [] });
                txn_write_json_pretty(&mut txn, "vcpkg.json", &empty)?;
            }
            let mut mani: serde_json::Value = crate::util::read_json("vcpkg.json")?;
            let deps = mani["dependencies"].as_array_mut()
                .ok_or_else(|| anyhow::anyhow!("vcpkg.json: 'dependencies' must be an array"))?;
            if !deps.iter().any(|v| v == pkg) {
                deps.push(serde_json::Value::String(pkg.to_string()));
                txn_write_json_pretty(&mut txn, "vcpkg.json", &mani)?;
            }

            if let Some(dest) = link_to_opt {
                let entry = root.components.entry(dest.to_string())
                    .or_insert(TritonComponent { kind: "lib".into(), ..Default::default() });
                if !entry.link.iter().any(|e| matches!(e, LinkEntry::Name(n) if n == pkg)) {
                    entry.link.push(LinkEntry::Name(pkg.to_string()));
                }
                ensure_component_scaffold(dest, &mut txn)?;
            }
        }
    }

    let root_json = serde_json::to_value(&root)?;
    txn_write_json_pretty(&mut txn, "triton.json", &root_json)?;

    txn.commit();
    Ok(())
}
