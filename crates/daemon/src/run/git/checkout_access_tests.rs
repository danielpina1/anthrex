//! Fix round 1 (I5): when `fchmodat` cannot honour `AT_SYMLINK_NOFOLLOW` (an older
//! Linux), a locked directory swapped for a link at that moment must never have its
//! target changed: nothing is changed by a name re-resolved after the check.

use std::cell::{Cell, RefCell};
use std::ffi::CStr;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

type Hook = Box<dyn Fn(&CStr)>;

thread_local! {
    pub(super) static FORCE_UNSUPPORTED: Cell<bool> = const { Cell::new(false) };
    pub(super) static BEFORE_FALLBACK: RefCell<Option<Hook>> = const { RefCell::new(None) };
}

fn mode(path: &Path) -> u32 {
    std::fs::symlink_metadata(path)
        .unwrap()
        .permissions()
        .mode()
        & 0o7777
}

fn set_mode(path: &Path, mode: u32) {
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
}

#[test]
fn an_unsupported_nofollow_never_changes_a_swapped_in_links_target() {
    let tmp = tempfile::tempdir().unwrap();
    let top = tmp.path().canonicalize().unwrap();
    let root = top.join("checkout");
    let locked = root.join("locked");
    std::fs::create_dir_all(&locked).unwrap();
    let outside = top.join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("f"), "f").unwrap();
    set_mode(&outside, 0o500);
    set_mode(&locked, 0o000);

    FORCE_UNSUPPORTED.with(|force| force.set(true));
    let (moved, link, target) = (root.join("locked-away"), locked.clone(), outside.clone());
    BEFORE_FALLBACK.with(|slot| {
        *slot.borrow_mut() = Some(Box::new(move |_: &CStr| {
            // (A directory with no access cannot be moved on macOS; reopening it
            // first changes nothing the test asserts.)
            set_mode(&link, 0o700);
            std::fs::rename(&link, &moved).unwrap();
            std::os::unix::fs::symlink(&target, &link).unwrap();
        }))
    });
    let result = super::restore_owner_access(&root);
    FORCE_UNSUPPORTED.with(|force| force.set(false));
    BEFORE_FALLBACK.with(|slot| *slot.borrow_mut() = None);

    let after = mode(&outside);
    set_mode(&outside, 0o700);
    if root.join("locked-away").exists() {
        set_mode(&root.join("locked-away"), 0o700);
    }
    assert_eq!(after, 0o500, "the link's target was changed ({result:?})");
}

#[test]
fn an_unsupported_nofollow_refuses_and_leaves_the_directory() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap().join("checkout");
    let locked = root.join("locked");
    std::fs::create_dir_all(&locked).unwrap();
    set_mode(&locked, 0o000);
    FORCE_UNSUPPORTED.with(|force| force.set(true));
    let result = super::restore_owner_access(&root);
    FORCE_UNSUPPORTED.with(|force| force.set(false));
    let after = mode(&locked);
    set_mode(&locked, 0o700);
    let refused = result.expect_err("a locked directory was changed by name");
    assert!(refused.contains("cannot restore access"), "{refused}");
    assert_eq!(after, 0o000);
}
