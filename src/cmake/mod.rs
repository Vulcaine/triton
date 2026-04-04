mod generators;
mod languages;
mod resolution;
mod rewrite;

pub use languages::detect_graph_languages;
pub use rewrite::{regenerate_root_cmake, rewrite_component_cmake};

use anyhow::Result;
use std::process::Command;

pub const MIN_CMAKE_VERSION: &str = "3.30.1";
pub const DEPS_BEGIN: &str = "# ## triton:deps begin";
pub const DEPS_END: &str = "# ## triton:deps end";

pub fn parse_cmake_version(ver: &str) -> (u32, u32, u32) {
    let parts: Vec<_> = ver.split('.').collect();
    let major = parts.get(0).and_then(|s| s.parse().ok()).unwrap_or(0);
    let minor = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
    let patch = parts.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);
    (major, minor, patch)
}

/// Return the system's `cmake --version` string (e.g. "3.30.2"), if cmake is on PATH.
pub fn system_cmake_version() -> anyhow::Result<Option<String>> {
    let out = Command::new("cmake").arg("--version").output();
    match out {
        Ok(o) if o.status.success() => {
            let txt = String::from_utf8_lossy(&o.stdout);
            if let Some(first) = txt.lines().next() {
                let ver = first
                    .trim()
                    .strip_prefix("cmake version ")
                    .unwrap_or("")
                    .trim();
                if !ver.is_empty() {
                    return Ok(Some(ver.to_string()));
                }
            }
            Ok(None)
        }
        _ => Ok(None),
    }
}

/// Normalize OS strings from config into canonical values.
/// Only canonical OS values are allowed ("windows", "macos", "linux").
/// Rejects arch-suffixed names like "win32", "win64", "mac32", "mac64", "linux32", "linux64".
fn normalize_os(os: &str) -> Result<&'static str, String> {
    match os.to_ascii_lowercase().as_str() {
        "win" | "windows" => Ok("windows"),
        "mac" | "osx" | "darwin" | "macos" => Ok("macos"),
        "linux" => Ok("linux"),
        bad @ ("win32" | "win64" | "mac32" | "mac64" | "linux32" | "linux64") => Err(format!(
            "Invalid OS identifier '{}'. Use only 'windows', 'macos', or 'linux'.",
            bad
        )),
        other => Err(format!(
            "Unknown OS identifier '{}'. Allowed: windows, macos, linux.",
            other
        )),
    }
}

/// Check whether a dependency is active for this host OS + triplet.
pub fn dep_is_active(
    dep: &crate::models::DepSpec,
    name: &str,
    host_os: &str,
    triplet: &str,
) -> bool {
    use crate::models::DepSpec;

    match dep {
        DepSpec::Simple(s) => s.eq_ignore_ascii_case(name),

        DepSpec::Git(g) => g.name.eq_ignore_ascii_case(name),

        DepSpec::Detailed(d) => {
            if !d.name.eq_ignore_ascii_case(name) {
                return false;
            }

            let host_norm = normalize_os(host_os).unwrap_or_else(|e| {
                panic!("Invalid host OS string '{}': {}", host_os, e);
            });

            let os_ok = d.os.is_empty()
                || d.os.iter().any(|o| {
                    normalize_os(o)
                        .unwrap_or_else(|e| panic!("Invalid OS in dep '{}': {}", d.name, e))
                        == host_norm
                });

            let trip_ok =
                d.triplet.is_empty() || d.triplet.iter().any(|t| t.eq_ignore_ascii_case(triplet));

            os_ok && trip_ok
        }
    }
}

pub fn detect_vcpkg_triplet() -> String {
    detect_vcpkg_triplet_for_arch(host_default_arch())
        .expect("host architecture should map to a valid vcpkg triplet")
}

pub fn host_default_arch() -> &'static str {
    if cfg!(target_os = "windows") {
        if cfg!(target_arch = "x86_64") {
            "x64"
        } else if cfg!(target_arch = "aarch64") {
            "arm64"
        } else {
            "x86"
        }
    } else if cfg!(target_os = "macos") {
        if cfg!(target_arch = "aarch64") {
            "arm64"
        } else {
            "x64"
        }
    } else if cfg!(target_os = "linux") {
        if cfg!(target_arch = "aarch64") {
            "arm64"
        } else {
            "x64"
        }
    } else {
        "x64"
    }
}

pub fn normalize_target_arch(arch: Option<&str>) -> Result<&'static str> {
    match arch.unwrap_or(host_default_arch()).trim().to_ascii_lowercase().as_str() {
        "x86" | "win32" | "i686" | "32" => Ok("x86"),
        "x64" | "amd64" | "64" => Ok("x64"),
        "arm64" | "aarch64" => Ok("arm64"),
        other => Err(anyhow::anyhow!(
            "Unsupported target architecture '{}'. Allowed values: x86, x64, arm64.",
            other
        )),
    }
}

pub fn detect_vcpkg_triplet_for_arch(arch: &str) -> Result<String> {
    let arch = normalize_target_arch(Some(arch))?;
    let triplet = if cfg!(target_os = "windows") {
        match arch {
            "x86" => "x86-windows",
            "x64" => "x64-windows",
            "arm64" => "arm64-windows",
            _ => unreachable!(),
        }
    } else if cfg!(target_os = "macos") {
        match arch {
            "x64" => "x64-osx",
            "arm64" => "arm64-osx",
            "x86" => anyhow::bail!("x86 builds are not supported on macOS"),
            _ => unreachable!(),
        }
    } else if cfg!(target_os = "linux") {
        match arch {
            "x86" => "x86-linux",
            "x64" => "x64-linux",
            "arm64" => "arm64-linux",
            _ => unreachable!(),
        }
    } else {
        match arch {
            "x86" => "x86-linux",
            "x64" => "x64-linux",
            "arm64" => "arm64-linux",
            _ => unreachable!(),
        }
    };
    Ok(triplet.into())
}

pub fn arch_label_for_triplet(triplet: &str) -> &'static str {
    let lower = triplet.to_ascii_lowercase();
    if lower.starts_with("x86-") {
        "x86"
    } else if lower.starts_with("arm64-") {
        "arm64"
    } else {
        "x64"
    }
}

pub fn cmake_generator_platform_for_triplet(triplet: &str) -> Option<&'static str> {
    if !cfg!(target_os = "windows") {
        return None;
    }

    match arch_label_for_triplet(triplet) {
        "x86" => Some("Win32"),
        "x64" => Some("x64"),
        "arm64" => Some("ARM64"),
        _ => None,
    }
}

/// Helper: pick effective cmake version (system if >= MIN, else MIN).
pub fn effective_cmake_version() -> (u32, u32, u32) {
    let min_ver = parse_cmake_version(MIN_CMAKE_VERSION);
    system_cmake_version()
        .ok()
        .flatten()
        .map(|s| parse_cmake_version(&s))
        .map(|sys| if sys >= min_ver { sys } else { min_ver })
        .unwrap_or(min_ver)
}

/// Map a triton platform name to a CMake conditional expression.
fn platform_to_cmake_condition(platform: &str) -> String {
    match platform.to_ascii_lowercase().as_str() {
        "linux"   => "UNIX AND NOT APPLE".to_string(),
        "macos"   => "APPLE".to_string(),
        "windows" => "WIN32".to_string(),
        other     => other.to_string(),
    }
}
