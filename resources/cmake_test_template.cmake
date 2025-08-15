add_executable(tests src/test_main.cpp)

find_package(GTest CONFIG REQUIRED)
target_link_libraries(tests PRIVATE GTest::gtest GTest::gtest_main)

set_property(TARGET tests PROPERTY CXX_STANDARD 20)

enable_testing()
add_test(NAME all_tests COMMAND tests)

# Copy any runtime DLLs (e.g. GTest built as DLL via vcpkg) next to the test exe
if(WIN32)
  add_custom_command(TARGET tests POST_BUILD
    COMMAND ${CMAKE_COMMAND} -E copy_if_different
      $<TARGET_RUNTIME_DLLS:tests>
      $<TARGET_FILE_DIR:tests>
    COMMAND_EXPAND_LISTS
  )
endif()
