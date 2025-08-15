// Embed resources at compile-time (independent of working dir).
const COMPONENT_CMAKE_TEMPLATE: &str =
    include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/resources/component_cmake_template.cmake"));

const TEST_COMPONENT_CMAKE_TEMPLATE: &str =
    include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/resources/cmake_test_template.cmake"));

const CMAKE_ROOT_HELPERS_TEMPLATE: &str =
    include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/resources/cmake_root_template.cmake"));

const CMAKE_COMPONENTS_HEADER_TEMPLATE: &str =
    include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/resources/components_dir_template.cmake"));

const PRESETS_TEMPLATE_JSON: &str =
    include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/resources/cmake_presets_template.json"));

/// Root components CMakeLists header (only cmake_minimum_required()/project()).
pub fn components_dir_cmakelists() -> String {
    CMAKE_COMPONENTS_HEADER_TEMPLATE.to_string()
}

/// Per-component CMakeLists template.
/// Pass `true` for the tests component to use the test-specific template.
pub fn component_cmakelists(is_test: bool) -> String {
    if is_test {
        TEST_COMPONENT_CMAKE_TEMPLATE.to_string()
    } else {
        COMPONENT_CMAKE_TEMPLATE.to_string()
    }
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
