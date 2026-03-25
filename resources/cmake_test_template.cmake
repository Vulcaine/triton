# Collect all test sources
file(GLOB_RECURSE TEST_SOURCES CONFIGURE_DEPENDS
    "${CMAKE_CURRENT_SOURCE_DIR}/src/*.cpp"
    "${CMAKE_CURRENT_SOURCE_DIR}/src/*.cc"
    "${CMAKE_CURRENT_SOURCE_DIR}/src/*.cxx"
)

# Create the test executable
add_executable(tests ${TEST_SOURCES})

# GoogleTest
find_package(GTest CONFIG REQUIRED)
target_link_libraries(tests PRIVATE
    GTest::gtest
    GTest::gtest_main
)

# C++ standard
set_property(TARGET tests PROPERTY CXX_STANDARD 20)

# Enable testing & discovery
enable_testing()
include(GoogleTest)

# Label all project tests so we can filter with: ctest -L triton
set(_TRITON_TEST_LABEL "triton")

# On Windows we must run discovery from exe dir (where DLLs get copied)
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
    PROPERTIES LABELS ${_TRITON_TEST_LABEL}
  )

  # Helpful for Visual Studio "Debug Tests"
  set_property(TARGET tests PROPERTY
    VS_DEBUGGER_WORKING_DIRECTORY "$<TARGET_FILE_DIR:tests>")
  set_property(TARGET tests PROPERTY
    VS_DEBUGGER_ENVIRONMENT "PATH=$<TARGET_FILE_DIR:tests>;%PATH%")
else()
  gtest_discover_tests(tests
    DISCOVERY_MODE PRE_TEST
    PROPERTIES LABELS ${_TRITON_TEST_LABEL}
  )
endif()

# Wrapper so tools expecting a single test still work
add_test(NAME all_tests COMMAND tests)
set_tests_properties(all_tests PROPERTIES LABELS ${_TRITON_TEST_LABEL})
