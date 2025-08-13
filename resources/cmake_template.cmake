cmake_minimum_required(VERSION 3.25)
project(triton_components LANGUAGES CXX)

# Components are added below (managed by triton)
# ## triton:components begin
# ## triton:components end
### END

### TEMPLATE:component_cmakelists
cmake_minimum_required(VERSION 3.25)

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

# Export headers: libs -> PUBLIC (so dependents see them), exe -> PRIVATE
if(_is_exe)
  target_include_directories(${_comp_name} PRIVATE "include")
else()
  target_include_directories(${_comp_name} PUBLIC "include")
endif()

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