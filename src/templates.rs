pub fn root_cmakelists(app_name: &str) -> String {
    format!(
r#"cmake_minimum_required(VERSION 3.25)
project({} LANGUAGES CXX)
"#,
        app_name
    )
}

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
# ## triton:deps end
"#
    .to_string()
}

pub fn cmake_presets(app_name: &str, generator: &str, triplet: &str) -> String {
    // Keep paths portable; CMake supports ${sourceDir}
    format!(
r#"{{
  "version": 6,
  "cmakeMinimumRequired": {{ "major": 3, "minor": 25, "patch": 0 }},
  "configurePresets": [
    {{
      "name": "debug",
      "displayName": "Debug",
      "generator": "{}",
      "binaryDir": "${{sourceDir}}/build/debug",
      "cacheVariables": {{
        "CMAKE_BUILD_TYPE": "Debug",
        "CMAKE_EXPORT_COMPILE_COMMANDS": "ON",
        "CMAKE_TOOLCHAIN_FILE": "${{sourceDir}}/vcpkg/scripts/buildsystems/vcpkg.cmake",
        "VCPKG_TARGET_TRIPLET": "{}"
      }}
    }},
    {{
      "name": "release",
      "displayName": "Release",
      "generator": "{}",
      "binaryDir": "${{sourceDir}}/build/release",
      "cacheVariables": {{
        "CMAKE_BUILD_TYPE": "Release",
        "CMAKE_EXPORT_COMPILE_COMMANDS": "ON",
        "CMAKE_TOOLCHAIN_FILE": "${{sourceDir}}/vcpkg/scripts/buildsystems/vcpkg.cmake",
        "VCPKG_TARGET_TRIPLET": "{}"
      }}
    }}
  ],
  "buildPresets": [
    {{ "name": "debug", "configurePreset": "debug" }},
    {{ "name": "release", "configurePreset": "release" }}
  ]
}}"#,
        generator, triplet, generator, triplet
    )
}
