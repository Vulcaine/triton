use anyhow::Result;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

use crate::models::{TritonComponent, TritonRoot};

#[derive(Debug, Clone, Default)]
pub struct ComponentLanguageSettings {
    pub uses_c: bool,
    pub uses_cxx: bool,
    pub c_std: Option<String>,
    pub cxx_std: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct GraphLanguageSettings {
    pub uses_c: bool,
    pub uses_cxx: bool,
}

impl GraphLanguageSettings {
    pub fn cmake_languages_clause(&self) -> &'static str {
        match (self.uses_c, self.uses_cxx) {
            (true, true) => "C CXX",
            (true, false) => "C",
            (false, true) => "CXX",
            (false, false) => "CXX",
        }
    }
}

fn normalize_source_path(raw: &str) -> String {
    raw.trim().replace('\\', "/")
}

fn resolve_component_source_path(project_root: &Path, component_dir: &Path, raw: &str) -> PathBuf {
    let normalized = normalize_source_path(raw);
    if let Some(rest) = normalized.strip_prefix("@root/") {
        project_root.join(rest)
    } else {
        component_dir.join(normalized)
    }
}

fn classify_path(path: &Path, uses_c: &mut bool, uses_cxx: &mut bool) {
    let Some(ext) = path.extension().and_then(|value| value.to_str()) else {
        return;
    };

    match ext.to_ascii_lowercase().as_str() {
        "c" => *uses_c = true,
        "cc" | "cpp" | "cxx" | "ixx" => *uses_cxx = true,
        _ => {}
    }
}

pub fn detect_component_languages(
    project_root: &Path,
    component_name: &str,
    root: &TritonRoot,
    comp: &TritonComponent,
) -> Result<ComponentLanguageSettings> {
    let component_dir = project_root.join("components").join(component_name);
    detect_component_languages_in_dir(project_root, &component_dir, root, comp)
}

pub fn detect_component_languages_in_dir(
    project_root: &Path,
    component_dir: &Path,
    root: &TritonRoot,
    comp: &TritonComponent,
) -> Result<ComponentLanguageSettings> {
    let mut uses_c = false;
    let mut uses_cxx = false;

    let auto_src_dir = component_dir.join("src");
    if auto_src_dir.is_dir() {
        for entry in WalkDir::new(&auto_src_dir).follow_links(true).into_iter().filter_map(|e| e.ok()) {
            if entry.file_type().is_file() {
                classify_path(entry.path(), &mut uses_c, &mut uses_cxx);
            }
        }
    }

    for raw in &comp.sources {
        if raw.trim().is_empty() {
            continue;
        }
        let source_path = resolve_component_source_path(project_root, component_dir, raw);
        classify_path(&source_path, &mut uses_c, &mut uses_cxx);
    }

    Ok(ComponentLanguageSettings {
        uses_c,
        uses_cxx,
        c_std: if uses_c {
            comp.c_std.clone().or_else(|| root.c_std.clone())
        } else {
            None
        },
        cxx_std: if uses_cxx {
            comp.cxx_std.clone().or_else(|| Some(root.cxx_std.clone()))
        } else {
            None
        },
    })
}

pub fn detect_graph_languages(project_root: &Path, root: &TritonRoot) -> Result<GraphLanguageSettings> {
    let mut settings = GraphLanguageSettings::default();
    for (name, comp) in &root.components {
        let component_lang = detect_component_languages(project_root, name, root, comp)?;
        settings.uses_c |= component_lang.uses_c;
        settings.uses_cxx |= component_lang.uses_cxx;
    }
    Ok(settings)
}
