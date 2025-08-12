// Templates for generated files

/// Root CMakeLists.txt scaffold with a managed components block.
pub fn root_cmakelists() -> String {
    r#"cmake_minimum_required(VERSION 3.25)
project(MyProject LANGUAGES CXX)

# Subdirectories will be (re)written by triton generate
# ## triton:components begin
add_subdirectory(components/app)
# ## triton:components end
"#
    .into()
}

/// Per-component CMakeLists.txt scaffold with a managed deps block.
pub fn component_cmakelists() -> String {
    r#"cmake_minimum_required(VERSION 3.25)

# Detect target name from directory
get_filename_component(_comp_name "${CMAKE_CURRENT_SOURCE_DIR}" NAME)

# If there's a main.cpp, assume executable; else library
file(GLOB_RECURSE COMP_SOURCES CONFIGURE_DEPENDS "src/*.cpp")
list(LENGTH COMP_SOURCES _src_len)
if(_src_len GREATER 0)
  add_executable(${_comp_name})
  target_sources(${_comp_name} PRIVATE ${COMP_SOURCES})
else()
  add_library(${_comp_name})
endif()

target_include_directories(${_comp_name} PRIVATE "include")
set_property(TARGET ${_comp_name} PROPERTY CXX_STANDARD 20)

# Dependencies (managed by triton)
# ## triton:deps begin
# (none)
# ## triton:deps end
"#
    .into()
}

/// CMakePresets.json with correct ${sourceDir} escaping.
/// NOTE: in Rust `format!`, write `${{sourceDir}}` to emit `${sourceDir}`.
pub fn cmake_presets(name: &str, generator: &str, triplet: &str) -> String {
    format!(
r#"{{
  "version": 6,
  "cmakeMinimumRequired": {{ "major": 3, "minor": 25, "patch": 0 }},
  "configurePresets": [
    {{
      "name": "default",
      "displayName": "{name} (default)",
      "generator": "{gen}",
      "binaryDir": "build/default",
      "cacheVariables": {{
        "CMAKE_TOOLCHAIN_FILE": "${{sourceDir}}/vcpkg/scripts/buildsystems/vcpkg.cmake",
        "VCPKG_TARGET_TRIPLET": "{triplet}",
        "CMAKE_EXPORT_COMPILE_COMMANDS": "ON"
      }}
    }},
    {{
      "name": "release",
      "inherits": "default",
      "cacheVariables": {{ "CMAKE_BUILD_TYPE": "Release" }},
      "binaryDir": "build/release"
    }},
    {{
      "name": "debug",
      "inherits": "default",
      "cacheVariables": {{ "CMAKE_BUILD_TYPE": "Debug" }},
      "binaryDir": "build/debug"
    }}
  ],
  "buildPresets": [
    {{ "name": "debug", "configurePreset": "debug", "configuration": "Debug" }},
    {{ "name": "release", "configurePreset": "release", "configuration": "Release" }}
  ]
}}"#,
        gen = generator,
        triplet = triplet,
        name = name
    )
}
