use anyhow::Result;

use crate::cmake::{regenerate_root_cmake, rewrite_component_cmake};
use crate::models::TritonRoot;
use crate::util::read_json;

pub fn handle_generate() -> Result<()> {
    let root: TritonRoot = read_json("triton.json")?;
    for (name, comp) in &root.components {
        rewrite_component_cmake(name, comp)?;
    }
    regenerate_root_cmake(&root)?;
    eprintln!("Regenerated CMake files.");
    Ok(())
}
