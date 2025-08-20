use anyhow::{Context, Result};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::models::{DepSpec, GitDep, LinkEntry, TritonComponent, TritonRoot};
use crate::util::{read_json, run};

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
    fn commit(mut self) {
        self.committed = true;
    }
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
    if let Some((p, c)) = pkg.split_once(':') {
        let p = p.trim();
        let c = c.trim();
        if !c.is_empty() { return (p, Some(c)); }
        return (p, None);
    }
    (pkg, component_opt.map(|s| s.trim()).filter(|s| !s.is_empty()))
}

pub fn handle_add(items: &[String], _features: Option<&str>, _host: bool) -> Result<()> {
    if items.is_empty() { return Ok(()); }

    let mut root: TritonRoot = read_json("triton.json")?;
    fs::create_dir_all("components")?;

    let mut txn = FsTxn::new();
    let mut touched_vcpkg_manifest = false;

    for it in items {
        let (pkg, link_to_opt) = parse_pkg_and_component(it, None);

        if pkg.contains('/') && !pkg.contains('\\') {
            // git dep
            let (repo, branch) = if let Some((r, b)) = pkg.split_once('@') {
                (r.to_string(), Some(b.to_string()))
            } else { (pkg.to_string(), None) };
            let name = repo.split('/').last().unwrap_or(&repo).to_string();

            let third = format!("third_party/{name}");
            if !Path::new(&third).exists() {
                fs::create_dir_all("third_party")?;
                eprintln!("Cloning https://github.com/{repo}.git into {third} …");
                run("git", &["clone", &format!("https://github.com/{repo}.git"), &third], ".")
                    .with_context(|| format!("git clone {repo}"))?;
                if let Some(br) = &branch {
                    run("git", &["checkout", br], &third)
                        .with_context(|| format!("git checkout {br} in {third}"))?;
                }
            }

            if !root.deps.iter().any(|d| matches!(d, DepSpec::Git(g) if g.name == name || g.repo == repo)) {
                root.deps.push(DepSpec::Git(GitDep { repo: repo.clone(), name: name.clone(), branch: branch.clone(), cmake: vec![] }));
            }

            if let Some(dest) = link_to_opt {
                let entry = root.components.entry(dest.to_string())
                    .or_insert(TritonComponent { kind: "lib".into(), link: vec![], defines: vec![], exports: vec![] });
                if !entry.link.iter().any(|e| matches!(e, LinkEntry::Name(n) if n == &name)) {
                    entry.link.push(LinkEntry::Name(name));
                }
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
                touched_vcpkg_manifest = true;
            }

            if let Some(dest) = link_to_opt {
                let entry = root.components.entry(dest.to_string())
                    .or_insert(TritonComponent { kind: "lib".into(), link: vec![], defines: vec![],  exports: vec![]  });
                if !entry.link.iter().any(|e| matches!(e, LinkEntry::Name(n) if n == pkg)) {
                    entry.link.push(LinkEntry::Name(pkg.to_string()));
                }
            }
        }
    }

    let root_json = serde_json::to_value(&root)?;
    txn_write_json_pretty(&mut txn, "triton.json", &root_json)?;

    if touched_vcpkg_manifest {
        eprintln!("Running vcpkg install (manifest mode)...");
        crate::util::run("vcpkg", &["install"], ".")?;
    }

    txn.commit();
    Ok(())
}
