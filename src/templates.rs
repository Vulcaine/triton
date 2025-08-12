// src/templates.rs
pub fn components_dir_cmakelists() -> String {
    r#"cmake_minimum_required(VERSION 3.25)
project(triton_components LANGUAGES CXX)

# Components are added below (managed by triton)
# ## triton:components begin
# ## triton:components end
"#
    .to_string()
}

pub fn component_cmakelists() -> String {
    r#"cmake_minimum_required(VERSION 3.25)

# Detect target name from directory
get_filename_component(_comp_name "${CMAKE_CURRENT_SOURCE_DIR}" NAME)

# Collect sources (any C/C++ in src)
file(GLOB_RECURSE COMP_SOURCES CONFIGURE_DEPENDS
    "src/*.c" "src/*.cc" "src/*.cxx" "src/*.cpp" "src/*.ixx")

# Rule: a component is an executable ONLY if it has src/main.cpp.
if(EXISTS "${CMAKE_CURRENT_SOURCE_DIR}/src/main.cpp")
  add_executable(${_comp_name})
  set(_is_exe ON)
else()
  add_library(${_comp_name})
  set(_is_exe OFF)
endif()

if(COMP_SOURCES)
  target_sources(${_comp_name} PRIVATE ${COMP_SOURCES})
endif()

target_include_directories(${_comp_name} PRIVATE "include")
set_property(TARGET ${_comp_name} PROPERTY CXX_STANDARD 20)

# On Windows, copy runtime DLLs beside the executable after build (MSVC, vcpkg, etc.)
if(WIN32 AND _is_exe)
  add_custom_command(TARGET ${_comp_name} POST_BUILD
    COMMAND ${CMAKE_COMMAND} -E copy_if_different
      $<TARGET_RUNTIME_DLLS:${_comp_name}>
      $<TARGET_FILE_DIR:${_comp_name}>
    COMMAND_EXPAND_LISTS
  )
endif()

# Dependencies (managed by triton)
# ## triton:deps begin
# --- triton: resolve local target name ---
if(NOT DEFINED _comp_name)
  get_filename_component(_comp_name "${CMAKE_CURRENT_SOURCE_DIR}" NAME)
endif()

# (triton will inject find_package/add_subdirectory/target_link_libraries here)

# ## triton:deps end
"#
    .to_string()
}

pub fn cmake_presets(_app_name: &str, generator: &str, triplet: &str) -> String {
    // NOTE: Presets live in components/, but we still build to ../build/<cfg>
    format!(
r#"{{
  "version": 6,
  "cmakeMinimumRequired": {{ "major": 3, "minor": 25, "patch": 0 }},
  "configurePresets": [
    {{
      "name": "debug",
      "displayName": "Debug",
      "generator": "{}",
      "binaryDir": "${{sourceDir}}/../build/debug",
      "cacheVariables": {{
        "CMAKE_BUILD_TYPE": "Debug",
        "CMAKE_EXPORT_COMPILE_COMMANDS": "ON",
        "CMAKE_TOOLCHAIN_FILE": "${{sourceDir}}/../vcpkg/scripts/buildsystems/vcpkg.cmake",
        "VCPKG_TARGET_TRIPLET": "{}"
      }}
    }},
    {{
      "name": "release",
      "displayName": "Release",
      "generator": "{}",
      "binaryDir": "${{sourceDir}}/../build/release",
      "cacheVariables": {{
        "CMAKE_BUILD_TYPE": "Release",
        "CMAKE_EXPORT_COMPILE_COMMANDS": "ON",
        "CMAKE_TOOLCHAIN_FILE": "${{sourceDir}}/../vcpkg/scripts/buildsystems/vcpkg.cmake",
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
