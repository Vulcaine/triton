use serde_json::json;
use std::borrow::Cow;

// Embed resources at compile-time (independent of working dir).
const COMPONENT_CMAKE_TEMPLATE: &str =
    include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/resources/component_cmake_template.cmake"));

const TEST_COMPONENT_CMAKE_TEMPLATE: &str =
    include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/resources/cmake_test_template.cmake"));

const CMAKE_ROOT_HELPERS_TEMPLATE: &str =
    include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/resources/cmake_root_template.cmake"));

const CMAKE_COMPONENTS_HEADER_TEMPLATE: &str =
    include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/resources/components_dir_template.cmake"));

/// Root components CMakeLists header (only cmake_minimum_required()/project()).
pub fn components_dir_cmakelists() -> String {
    CMAKE_COMPONENTS_HEADER_TEMPLATE.to_string()
}

/// Per-component CMakeLists template.
/// Pass `true` for the tests component to use the test-specific template.
pub fn component_cmakelists(is_test: bool) -> String {
    let tpl: Cow<'static, str> = if is_test {
        Cow::from(TEST_COMPONENT_CMAKE_TEMPLATE)
    } else {
        Cow::from(COMPONENT_CMAKE_TEMPLATE)
    };
    tpl.to_string()
}

/// Helper block injected into root components CMakeLists.
pub fn cmake_root_helpers() -> &'static str {
    CMAKE_ROOT_HELPERS_TEMPLATE
}

/// Render CMakePresets.json for the selected triplet.
pub fn cmake_presets(
    _app_name: &str,
    generator: &str,
    triplet: &str,
    cmake_ver: (u32, u32, u32),
) -> String {
    let arch = crate::cmake::arch_label_for_triplet(triplet);
    let (maj, min, pat) = cmake_ver;

    let mut debug = json!({
        "name": "debug",
        "displayName": format!("Debug ({})", arch),
        "generator": generator,
        "binaryDir": format!("${{sourceDir}}/../build/{}/debug", arch),
        "cacheVariables": {
            "CMAKE_BUILD_TYPE": "Debug",
            "CMAKE_EXPORT_COMPILE_COMMANDS": "ON",
            "CMAKE_TOOLCHAIN_FILE": "${sourceDir}/../vcpkg/scripts/buildsystems/vcpkg.cmake",
            "VCPKG_TARGET_TRIPLET": triplet,
            "VCPKG_MANIFEST_DIR": "${sourceDir}/.."
        }
    });

    let mut release = json!({
        "name": "release",
        "displayName": format!("Release ({})", arch),
        "generator": generator,
        "binaryDir": format!("${{sourceDir}}/../build/{}/release", arch),
        "cacheVariables": {
            "CMAKE_BUILD_TYPE": "Release",
            "CMAKE_EXPORT_COMPILE_COMMANDS": "ON",
            "CMAKE_TOOLCHAIN_FILE": "${sourceDir}/../vcpkg/scripts/buildsystems/vcpkg.cmake",
            "VCPKG_TARGET_TRIPLET": triplet,
            "VCPKG_MANIFEST_DIR": "${sourceDir}/.."
        }
    });

    if cfg!(windows) && generator.to_ascii_lowercase().contains("visual studio") {
        if let Some(platform) = crate::cmake::cmake_generator_platform_for_triplet(triplet) {
            let architecture = json!({ "value": platform, "strategy": "set" });
            debug["architecture"] = architecture.clone();
            release["architecture"] = architecture;
        }
    }

    serde_json::to_string_pretty(&json!({
        "version": 6,
        "cmakeMinimumRequired": {
            "major": maj,
            "minor": min,
            "patch": pat
        },
        "configurePresets": [debug, release],
        "buildPresets": [
            { "name": "debug", "configurePreset": "debug" },
            { "name": "release", "configurePreset": "release" }
        ]
    }))
    .expect("serializing CMakePresets.json should succeed")
}
