//! `checkout_dir`: the directory a test binary was compiled in must be the one it runs in.

use anthrex_testexec::checkout_dir;

#[test]
fn the_running_checkout_is_returned() {
    assert_eq!(
        checkout_dir(env!("CARGO_MANIFEST_DIR")),
        std::env::current_dir().unwrap().canonicalize().unwrap()
    );
}

#[test]
#[should_panic(expected = "was compiled in")]
fn another_checkout_is_refused() {
    let other = tempfile::tempdir().unwrap();
    checkout_dir(other.path().to_str().unwrap());
}

#[test]
fn a_symlinked_spelling_is_the_same_checkout() {
    let dir = tempfile::tempdir().unwrap();
    let link = dir.path().join("checkout");
    std::os::unix::fs::symlink(checkout_dir(env!("CARGO_MANIFEST_DIR")), &link).unwrap();
    assert_eq!(
        checkout_dir(link.to_str().unwrap()),
        std::env::current_dir().unwrap().canonicalize().unwrap()
    );
}
