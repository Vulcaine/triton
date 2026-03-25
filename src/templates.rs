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

const PRESETS_TEMPLATE_JSON: &str =
    include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/resources/cmake_presets_template.json"));

fn ver_parts(ver: (u32, u32, u32)) -> (String, String, String) {
    (ver.0.to_string(), ver.1.to_string(), ver.2.to_string())
}

/// Root components CMakeLists header (only cmake_minimum_required()/project()).
pub fn components_dir_cmakelists() -> String {
    CMAKE_COMPONENTS_HEADER_TEMPLATE.to_string()
}

/// Per-component CMakeLists template.
/// Pass `true` for the tests component to use the test-specific template.
/// `cmake_ver` is (major, minor, patch) to be placed in `cmake_minimum_required(...)`.
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
/// Render CMakePresets.json by replacing placeholders in the JSON template.
/// Placeholders supported:
///   {{APP_NAME}}, {{GENERATOR}}, {{TRIPLET}},
///   {{CMAKE_MAJOR}}, {{CMAKE_MINOR}}, {{CMAKE_PATCH}},
///   {{CMAKE_MIN_MAJOR}}, {{CMAKE_MIN_MINOR}}, {{CMAKE_MIN_PATCH}}, {{CMAKE_MIN_VERSION}}
pub fn cmake_presets(
    app_name: &str,
    generator: &str,
    triplet: &str,
    cmake_ver: (u32, u32, u32),
) -> String {
    let (maj, min, pat) = ver_parts(cmake_ver);
    let maj_str = maj.to_string();
    let min_str = min.to_string();
    let pat_str = pat.to_string();

    PRESETS_TEMPLATE_JSON
        .replace("{{APP_NAME}}", app_name)
        .replace("{{GENERATOR}}", generator)
        .replace("{{TRIPLET}}", triplet)
        // Match both quoted and unquoted placeholders
        .replace("\"{{CMAKE_MAJOR}}\"", &maj_str)
        .replace("{{CMAKE_MAJOR}}", &maj_str)
        .replace("\"{{CMAKE_MINOR}}\"", &min_str)
        .replace("{{CMAKE_MINOR}}", &min_str)
        .replace("\"{{CMAKE_PATCH}}\"", &pat_str)
        .replace("{{CMAKE_PATCH}}", &pat_str)
        // Back-compat names
        .replace("\"{{CMAKE_MIN_MAJOR}}\"", &maj_str)
        .replace("{{CMAKE_MIN_MAJOR}}", &maj_str)
        .replace("\"{{CMAKE_MIN_MINOR}}\"", &min_str)
        .replace("{{CMAKE_MIN_MINOR}}", &min_str)
        .replace("\"{{CMAKE_MIN_PATCH}}\"", &pat_str)
        .replace("{{CMAKE_MIN_PATCH}}", &pat_str)
        .replace("{{CMAKE_MIN_VERSION}}", &format!("{}.{}.{}", maj, min, pat))
}
