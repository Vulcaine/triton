# Detect target name from directory
get_filename_component(_comp_name "${CMAKE_CURRENT_SOURCE_DIR}" NAME)

# Collect sources (any C/C++ in src)
file(GLOB_RECURSE COMP_SOURCES CONFIGURE_DEPENDS
    "src/*.c" "src/*.cc" "src/*.cxx" "src/*.cpp" "src/*.ixx")

# Rule: a component is an executable only if it has a recognized src/main.* entrypoint.
if(
  EXISTS "${CMAKE_CURRENT_SOURCE_DIR}/src/main.c" OR
  EXISTS "${CMAKE_CURRENT_SOURCE_DIR}/src/main.cc" OR
  EXISTS "${CMAKE_CURRENT_SOURCE_DIR}/src/main.cpp" OR
  EXISTS "${CMAKE_CURRENT_SOURCE_DIR}/src/main.cxx" OR
  EXISTS "${CMAKE_CURRENT_SOURCE_DIR}/src/main.ixx"
)
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

@TRITON_LANG_SETTINGS@

# On Windows, copy runtime DLLs beside the executable after build (MSVC, vcpkg, etc.)
if(WIN32 AND _is_exe)
  add_custom_command(TARGET ${_comp_name} POST_BUILD
    COMMAND ${CMAKE_COMMAND} -E
      $<IF:$<BOOL:$<TARGET_RUNTIME_DLLS:${_comp_name}>>,copy_if_different,true>
      $<TARGET_RUNTIME_DLLS:${_comp_name}>
      $<$<BOOL:$<TARGET_RUNTIME_DLLS:${_comp_name}>>:$<TARGET_FILE_DIR:${_comp_name}>>
    COMMAND_EXPAND_LISTS
  )

  # Ensure VS debugger / cmake launchers run inside exe folder
  set_property(TARGET ${_comp_name} PROPERTY
    VS_DEBUGGER_WORKING_DIRECTORY "$<TARGET_FILE_DIR:${_comp_name}>")
  if(DEFINED VCPKG_TARGET_TRIPLET)
    set_property(TARGET ${_comp_name} PROPERTY
      VS_DEBUGGER_ENVIRONMENT "PATH=$<TARGET_FILE_DIR:${_comp_name}>;${CMAKE_BINARY_DIR}/vcpkg_installed/${VCPKG_TARGET_TRIPLET}/debug/bin;${CMAKE_BINARY_DIR}/vcpkg_installed/${VCPKG_TARGET_TRIPLET}/bin;%PATH%")
  else()
    set_property(TARGET ${_comp_name} PROPERTY
      VS_DEBUGGER_ENVIRONMENT "PATH=$<TARGET_FILE_DIR:${_comp_name}>;%PATH%")
  endif()
endif()

# Dependencies
@TRITON_DEPS@
