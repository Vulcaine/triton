use anyhow::{Context, Result};
use std::fs;
use std::path::Path;

use crate::cmake::{regenerate_root_cmake, rewrite_component_cmake};
use crate::models::{LinkEntry, TritonComponent, TritonRoot};
use crate::templates::component_cmakelists;
use crate::util::{read_json, write_json_pretty_changed, write_text_if_changed};

/// Link one component to another (adds `to` as a dependency of `from`).
/// Also used for dep->component links at CLI level elsewhere.
pub fn handle_link(from: &str, to: &str) -> Result<()> {
    for name in [from, to] {
        let base = format!("components/{name}");
        fs::create_dir_all(format!("{base}/src"))?;
        fs::create_dir_all(format!("{base}/include"))?;
        let cm = format!("{base}/CMakeLists.txt");
        if !Path::new(&cm).exists() {
            write_text_if_changed(&cm, &component_cmakelists())
                .with_context(|| format!("writing {}", cm))?;
        }
    }

    let mut root: TritonRoot = read_json("triton.json")?;
    root.components.entry(from.into()).or_insert(TritonComponent {
        kind: "lib".into(),
        link: vec![],
    });
    root.components.entry(to.into()).or_insert(TritonComponent {
        kind: "lib".into(),
        link: vec![],
    });

    {
        let cfrom = root.components.get_mut(from).unwrap();
        if !cfrom.link.iter().any(|e| matches!(e, LinkEntry::Name(n) if n == to)) {
            cfrom.link.push(LinkEntry::Name(to.into()));
        }
    }

    write_json_pretty_changed("triton.json", &root)?;
    if let Some(cf) = root.components.get(from) { rewrite_component_cmake(from, &root, cf)?; }
    if let Some(ct) = root.components.get(to) { rewrite_component_cmake(to, &root, ct)?; }
    regenerate_root_cmake(&root)?;

    eprintln!("Linked component '{}' : '{}'.", from, to);
    Ok(())
}
