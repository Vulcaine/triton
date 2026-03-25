use anyhow::Result;
use std::path::Path;

use crate::cmake::detect_vcpkg_triplet;
use crate::util::{match_dep_to_packages, scan_vcpkg_share_for_configs};

pub fn handle_find_target(dep: &str) -> Result<()> {
    let triplet = detect_vcpkg_triplet();
    let share_dir = Path::new("vcpkg")
        .join("installed")
        .join(&triplet)
        .join("share");

    if !share_dir.exists() {
        eprintln!(
            "No vcpkg share directory found at '{}'.",
            share_dir.display()
        );
        eprintln!("Make sure dependencies are installed: triton build .");
        eprintln!("Then check vcpkg/installed/{}/share/ for available packages.", triplet);
        return Ok(());
    }

    eprintln!("Searching for CMake package for '{}'...", dep);

    let all_packages = scan_vcpkg_share_for_configs(&share_dir);

    if all_packages.is_empty() {
        eprintln!("No CMake packages found in '{}'.", share_dir.display());
        return Ok(());
    }

    let matches = match_dep_to_packages(dep, &all_packages);

    if matches.is_empty() {
        eprintln!("No CMake package found matching '{}'.", dep);
        eprintln!();
        eprintln!("Available packages:");
        for (name, _) in &all_packages {
            eprintln!("  {}", name);
        }
        eprintln!();
        eprintln!(
            "Specify the correct one in triton.json:\n  \
             {{ \"name\": \"{}\", \"package\": \"<PackageName>\" }}",
            dep
        );
    } else if matches.len() == 1 {
        let (pkg_name, config_path) = &matches[0];
        eprintln!("Found: {}", pkg_name);
        eprintln!("  Config: {}", config_path.display());
        eprintln!();
        if pkg_name.to_ascii_lowercase() != dep.to_ascii_lowercase() {
            eprintln!(
                "Use in triton.json:\n  \
                 {{ \"name\": \"{}\", \"package\": \"{}\" }}",
                dep, pkg_name
            );
        } else {
            eprintln!("Package name matches dep name — no override needed.");
        }
    } else {
        eprintln!("Found multiple candidates:");
        for (i, (pkg_name, config_path)) in matches.iter().enumerate() {
            eprintln!(
                "  {}. {}  ({})",
                i + 1,
                pkg_name,
                config_path.display()
            );
        }
        eprintln!();
        eprintln!(
            "Specify the correct one in triton.json:\n  \
             {{ \"name\": \"{}\", \"package\": \"<PackageName>\" }}",
            dep
        );
    }

    Ok(())
}
