use std::fs;
use tempfile::tempdir;

use serial_test::serial;

use triton::commands::init::handle_init;
use triton::commands::testcmd::handle_test;

mod test_utils;
use test_utils::copy_offline_vcpkg_to;

#[test]
#[serial]
fn testcmd_runs_build_if_missing() {
    let td = tempdir().unwrap();
    let proj = td.path().join("proj-testcmd");
    fs::create_dir_all(&proj).unwrap();
    copy_offline_vcpkg_to(&proj);

    std::env::set_current_dir(&td).unwrap();
    handle_init(Some("proj-testcmd"), "Ninja", "20").unwrap();

    let build_dir = proj.join("build");
    if build_dir.exists() {
        fs::remove_dir_all(&build_dir).unwrap();
    }

    std::env::set_current_dir(&proj).unwrap();
    std::env::set_var("TRITON_TEST_MODE", "1"); // skip real build
    handle_test(".", "debug").unwrap();
}
