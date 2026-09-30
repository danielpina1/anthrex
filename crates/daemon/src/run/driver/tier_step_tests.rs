//! Ruling C-27 (M-2): a step directory that cannot be removed is left, with the
//! "Risks" section's warning.

use std::fs::Permissions;
use std::os::unix::fs::PermissionsExt;

use super::remove_dir;

#[test]
fn a_step_directory_that_cannot_go_is_left_for_cleanup() {
    // SAFETY: geteuid has no preconditions and cannot fail.
    if unsafe { libc::geteuid() } == 0 {
        return; // root removes it whatever the modes say
    }
    let base = tempfile::tempdir().unwrap();
    let dir = base.path().join("s7-3");
    std::fs::create_dir(&dir).unwrap();
    std::fs::write(dir.join("d.sock"), "").unwrap();
    std::fs::set_permissions(base.path(), Permissions::from_mode(0o500)).unwrap();
    let left = remove_dir(&dir);
    std::fs::set_permissions(base.path(), Permissions::from_mode(0o700)).unwrap();
    assert_eq!(
        left.as_deref(),
        Some("step 3: its directory is still in use; left for cleanup")
    );
    assert!(dir.exists());
    assert_eq!(remove_dir(&dir), None);
    assert!(!dir.exists());
}
