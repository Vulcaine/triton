# Collect all test sources (any .cpp/.cc/.cxx anywhere under tests/src)
file(GLOB_RECURSE TEST_SOURCES CONFIGURE_DEPENDS
    "${CMAKE_CURRENT_SOURCE_DIR}/src/*.cpp"
    "${CMAKE_CURRENT_SOURCE_DIR}/src/*.cc"
    "${CMAKE_CURRENT_SOURCE_DIR}/src/*.cxx"
)

# Create the test executable
add_executable(tests ${TEST_SOURCES})

# Optional: nice IDE grouping
source_group(TREE "${CMAKE_CURRENT_SOURCE_DIR}/src" FILES ${TEST_SOURCES})

# GoogleTest
find_package(GTest CONFIG REQUIRED)
target_link_libraries(tests PRIVATE
    GTest::gtest
    GTest::gtest_main
    Engine
)

# C++ standard
set_property(TARGET tests PROPERTY CXX_STANDARD 20)

# Enable testing & discovery
enable_testing()
include(GoogleTest)

# On Windows, run discovery from the test exe directory (to find vcpkg DLLs)
if(WIN32)
  add_custom_command(TARGET tests POST_BUILD
    COMMAND ${CMAKE_COMMAND} -E copy_if_different
      $<TARGET_RUNTIME_DLLS:tests>
      $<TARGET_FILE_DIR:tests>
    COMMAND_EXPAND_LISTS
  )

  gtest_discover_tests(tests
    WORKING_DIRECTORY $<TARGET_FILE_DIR:tests>
    DISCOVERY_MODE PRE_TEST
  )

  # Useful for VS "Debug Tests"
  set_property(TARGET tests PROPERTY
    VS_DEBUGGER_WORKING_DIRECTORY "$<TARGET_FILE_DIR:tests>")
  set_property(TARGET tests PROPERTY
    VS_DEBUGGER_ENVIRONMENT "PATH=$<TARGET_FILE_DIR:tests>;%PATH%")
else()
  gtest_discover_tests(tests DISCOVERY_MODE PRE_TEST)
endif()

# Wrapper test so tools that expect a single test still work
# (harmless when also using gtest_discover_tests)
add_test(NAME all_tests COMMAND tests)

# ## triton:deps begin
# --- triton: resolve local target name ---
if(NOT DEFINED _comp_name)
  get_filename_component(_comp_name "${CMAKE_CURRENT_SOURCE_DIR}" NAME)
endif()

# Link to the code under test (your Engine component)
target_link_libraries(${_comp_name} PRIVATE Engine)
if(EXISTS "${CMAKE_SOURCE_DIR}/Engine/include")
  target_include_directories(${_comp_name} PRIVATE "${CMAKE_SOURCE_DIR}/Engine/include")
endif()
# ## triton:deps end
