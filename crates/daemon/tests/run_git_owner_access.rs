//! Milestone 9.11 decision 17 (D9): `restore_owner_access` gives the owner access back
//! on a checkout before it is removed. It walked by path: a directory checked, then
//! listed and its children changed through a path re-resolved at each step, so a
//! worker that swapped a checked directory for a link between the steps had the
//! daemon change modes outside the checkout. It now walks by descriptor.

use daemon::run::git::restore_owner_access_with;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

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
fn restore_owner_access_never_leaves_its_root() {
    let tmp = tempfile::tempdir().unwrap();
    let top = tmp.path().canonicalize().unwrap();
    let root = top.join("checkout");
    let a = root.join("a");
    std::fs::create_dir_all(a.join("locked")).unwrap();
    // Outside the checkout: a directory with a subdirectory, both closed to writes.
    let outside = top.join("outside");
    std::fs::create_dir_all(outside.join("inner")).unwrap();
    set_mode(&outside.join("inner"), 0o500);
    set_mode(&outside, 0o500);
    set_mode(&a.join("locked"), 0o500);

    let mut swapped = false;
    restore_owner_access_with(&root, |opened: &Path| {
        // Once `a` has been opened, it is swapped for a link to `outside`.
        if opened == a && !swapped {
            swapped = true;
            std::fs::rename(&a, root.join("a-moved")).unwrap();
            std::os::unix::fs::symlink(&outside, &a).unwrap();
        }
    })
    .unwrap();

    assert!(swapped, "the seam never saw `a` opened");
    assert_eq!(
        mode(&outside),
        0o500,
        "the walk changed a directory outside"
    );
    assert_eq!(
        mode(&outside.join("inner")),
        0o500,
        "the walk changed a directory outside, through the link"
    );
    // The real `a`'s child, reached through its descriptor, got its access back.
    assert_eq!(mode(&root.join("a-moved/locked")) & 0o700, 0o700);

    set_mode(&outside, 0o700);
    set_mode(&outside.join("inner"), 0o700);
}

#[test]
fn a_locked_tree_is_opened_up_and_links_are_left_alone() {
    let tmp = tempfile::tempdir().unwrap();
    let top = tmp.path().canonicalize().unwrap();
    let root = top.join("checkout");
    std::fs::create_dir_all(root.join("x/y")).unwrap();
    std::fs::write(root.join("x/y/f"), "f").unwrap();
    let outside = top.join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    set_mode(&outside, 0o500);
    std::os::unix::fs::symlink(&outside, root.join("x/link")).unwrap();
    // Locked bottom-up: `y` searchable by no one, `x` read-only, the root execute-only.
    set_mode(&root.join("x/y"), 0o000);
    set_mode(&root.join("x"), 0o500);
    set_mode(&root, 0o100);

    restore_owner_access_with(&root, |_: &Path| {}).unwrap();

    for dir in [root.clone(), root.join("x"), root.join("x/y")] {
        assert_eq!(mode(&dir) & 0o700, 0o700, "{}", dir.display());
    }
    assert_eq!(mode(&outside), 0o500);
    std::fs::remove_dir_all(&root).unwrap();
    set_mode(&outside, 0o700);
}
