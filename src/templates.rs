// src/templates.rs

// Embed resources at compile-time (independent of working dir).
const CMAKE_COMPONENT_TEMPLATE: &str =
    include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/resources/cmake_template.cmake"));

const CMAKE_ROOT_HELPERS_TEMPLATE: &str =
    include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/resources/cmake_root_template.cmake"));

const CMAKE_COMPONENTS_HEADER_TEMPLATE: &str =
    include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/resources/cmake_components_header.cmake"));

const PRESETS_TEMPLATE_JSON: &str =
    include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/resources/cmake_presets_template.json"));

/// Root components CMakeLists header (only cmake_minimum_required()/project()).
pub fn components_dir_cmakelists() -> String {
    CMAKE_COMPONENTS_HEADER_TEMPLATE.to_string()
}

/// Per-component CMakeLists template.
pub fn component_cmakelists() -> String {
    CMAKE_COMPONENT_TEMPLATE.to_string()
}

/// Helper block injected into root components CMakeLists.
pub fn cmake_root_helpers() -> &'static str {
    CMAKE_ROOT_HELPERS_TEMPLATE
}

/// Render CMakePresets.json by replacing placeholders in the JSON template.
/// Placeholders: {{APP_NAME}}, {{GENERATOR}}, {{TRIPLET}}
pub fn cmake_presets(app_name: &str, generator: &str, triplet: &str) -> String {
    PRESETS_TEMPLATE_JSON
        .replace("{{APP_NAME}}", app_name)
        .replace("{{GENERATOR}}", generator)
        .replace("{{TRIPLET}}", triplet)
}
