use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::cmake::detect_vcpkg_triplet;
use crate::models::{DepDetailed, DepSpec, TritonRoot};
use crate::util::{
    list_dir_names, match_dep_to_packages, read_json, scan_vcpkg_share_for_configs,
    write_json_pretty_changed,
};

/// Install all vcpkg deps valid for current host, using the project-local vcpkg binary.
pub fn handle_install(_root: &TritonRoot, project: &Path, _vcpkg_exe: &PathBuf) -> Result<()> {
    // ensure vcpkg.json is up-to-date
    crate::commands::handle_generate()?;

    let triplet = detect_vcpkg_triplet();
    let share_dir = project
        .join("vcpkg")
        .join("installed")
        .join(&triplet)
        .join("share");

    // Snapshot share dir before install
    let dirs_before = list_dir_names(&share_dir);

    eprintln!("Running vcpkg install with manifest mode...");

    // Use the project-local vcpkg binary (bootstrapped by ensure_vcpkg).
    #[cfg(windows)]
    let vcpkg_bin = project.join("vcpkg").join("vcpkg.exe");
    #[cfg(not(windows))]
    let vcpkg_bin = project.join("vcpkg").join("vcpkg");

    let status = Command::new(&vcpkg_bin)
        .arg("install")
        .arg(format!("--triplet={}", triplet))
        .current_dir(project)
        .status()
        .context("failed to run vcpkg install")?;

    if !status.success() {
        anyhow::bail!("vcpkg install failed");
    }

    // Auto-detect package names for simple deps
    auto_detect_package_names(project, &share_dir, &triplet, &dirs_before)?;

    // Verify requested features actually installed
    let root: TritonRoot = read_json(project.join("triton.json"))?;
    verify_vcpkg_features(&root, &vcpkg_bin, &triplet)?;

    Ok(())
}

/// After vcpkg install, scan for new share dirs and auto-detect CMake package names
/// for any DepSpec::Simple deps whose package name differs from the dep name.
fn auto_detect_package_names(
    project: &Path,
    share_dir: &Path,
    _triplet: &str,
    dirs_before: &std::collections::HashSet<String>,
) -> Result<()> {
    if !share_dir.exists() {
        return Ok(());
    }

    let mut root: TritonRoot = read_json(project.join("triton.json"))?;
    let mut changed = false;

    // For each simple dep, check if there's a matching config in share/
    // that has a different name (case-sensitive)
    let all_packages = scan_vcpkg_share_for_configs(share_dir);

    for i in 0..root.deps.len() {
        let dep_name = match &root.deps[i] {
            DepSpec::Simple(n) => n.clone(),
            _ => continue,
        };

        let matches = match_dep_to_packages(&dep_name, &all_packages);

        if matches.len() == 1 {
            let (pkg_name, _) = &matches[0];
            // Only upgrade to Detailed if the package name actually differs
            if pkg_name != &dep_name {
                eprintln!(
                    "Auto-detected CMake package: {} (for {})",
                    pkg_name, dep_name
                );
                root.deps[i] = DepSpec::Detailed(DepDetailed {
                    name: dep_name,
                    package: Some(pkg_name.clone()),
                    os: vec![],
                    triplet: vec![],
                    features: vec![],
                });
                changed = true;
            }
        } else if matches.len() > 1 {
            // Check if any are genuinely new (not pre-existing)
            let new_matches: Vec<_> = matches
                .iter()
                .filter(|(name, _)| !dirs_before.contains(name))
                .collect();

            if new_matches.len() == 1 {
                let (pkg_name, _) = new_matches[0];
                if pkg_name != &dep_name {
                    eprintln!(
                        "Auto-detected CMake package: {} (for {})",
                        pkg_name, dep_name
                    );
                    root.deps[i] = DepSpec::Detailed(DepDetailed {
                        name: dep_name,
                        package: Some(pkg_name.clone()),
                        os: vec![],
                        triplet: vec![],
                        features: vec![],
                    });
                    changed = true;
                }
            } else if !new_matches.is_empty() {
                let names: Vec<_> = new_matches.iter().map(|(n, _)| n.as_str()).collect();
                eprintln!(
                    "Warning: multiple CMake packages found for '{}': [{}]. \
                     You may need to specify the correct one in triton.json: \
                     {{ \"name\": \"{}\", \"package\": \"<PackageName>\" }}",
                    dep_name,
                    names.join(", "),
                    dep_name
                );
            }
        }
    }

    if changed {
        write_json_pretty_changed(project.join("triton.json"), &root)?;
        // Re-run generate so vcpkg.json picks up the package overrides
        crate::commands::handle_generate()?;
    }

    Ok(())
}

/// Verify that requested vcpkg features were actually installed.
fn verify_vcpkg_features(root: &TritonRoot, vcpkg_bin: &Path, triplet: &str) -> Result<()> {
    // Collect deps that have features
    let deps_with_features: Vec<(&str, &[String])> = root
        .deps
        .iter()
        .filter_map(|d| match d {
            DepSpec::Detailed(dd) if !dd.features.is_empty() => {
                Some((dd.name.as_str(), dd.features.as_slice()))
            }
            _ => None,
        })
        .collect();

    if deps_with_features.is_empty() {
        return Ok(());
    }

    // Run vcpkg list to get installed packages + features
    let output = Command::new(vcpkg_bin)
        .args(["list", &format!("--triplet={}", triplet)])
        .output();

    let output = match output {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).to_string(),
        _ => {
            eprintln!("Warning: could not run 'vcpkg list' to verify features.");
            return Ok(());
        }
    };

    // Parse vcpkg list output. Format varies but typically:
    //   packagename:triplet    version    [feature1,feature2]
    //   or separate lines per feature:
    //   packagename[feature]:triplet    version
    for (dep_name, required_features) in &deps_with_features {
        for feature in *required_features {
            // Look for "depname[feature]:triplet" pattern
            let feature_pattern = format!("{}[{}]:{}", dep_name, feature, triplet);
            let feature_pattern_lower = feature_pattern.to_ascii_lowercase();

            let found = output
                .lines()
                .any(|line| line.to_ascii_lowercase().starts_with(&feature_pattern_lower));

            if !found {
                anyhow::bail!(
                    "vcpkg feature '{}' was requested for '{}' but was not installed.\n\
                     Check that the feature name is correct: vcpkg search {}",
                    feature, dep_name, dep_name
                );
            }
        }
    }

    Ok(())
}
