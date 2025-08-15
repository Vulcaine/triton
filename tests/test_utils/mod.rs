use std::fs;
use std::path::{Path};

/// Copies the pre-cloned offline vcpkg tree from `tests/vcpkg-offline` into `<proj>/vcpkg`.
/// Panics with a clear error if the source tree doesn't exist.
pub fn copy_offline_vcpkg_to<P: AsRef<Path>>(proj: P) {
    let dest = proj.as_ref().join("vcpkg");
    let buildsystems = dest.join("scripts/buildsystems");
    let gtest_share = dest.join("installed/x64-windows/share/gtest");
    fs::create_dir_all(&buildsystems).unwrap();
    fs::create_dir_all(&gtest_share).unwrap();

    fs::write(
        buildsystems.join("vcpkg.cmake"),
        "# fake vcpkg.cmake for tests\n",
    ).unwrap();

    fs::write(
        gtest_share.join("GTestConfig.cmake"),
        "add_library(GTest::gtest INTERFACE IMPORTED)\n\
         add_library(GTest::gtest_main INTERFACE IMPORTED)\n",
    ).unwrap();
}
