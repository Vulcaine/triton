use std::fs;
use std::io::Write;
use std::path::Path;

pub fn ensure_component_scaffold(name: &str) -> anyhow::Result<()> {
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
