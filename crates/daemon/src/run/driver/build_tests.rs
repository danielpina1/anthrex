//! M9.17 fix round 2, item 3: decision 17's `installed` reads `PATH` as a shell does, a
//! leading `~` expanded to `HOME`.

use std::ffi::OsStr;
use std::os::unix::fs::PermissionsExt;

use super::executable_in;

#[test]
fn a_tilde_entry_on_path_is_expanded_to_home() {
    let home = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(home.path().join("bin")).unwrap();
    for file in ["bin/agent", "top"] {
        let path = home.path().join(file);
        std::fs::write(&path, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let h = Some(home.path().as_os_str());
    let path = |p: &'static str| Some(OsStr::new(p));
    assert!(executable_in("agent", path("/nonexistent:~/bin"), h));
    assert!(executable_in("top", path("~"), h));
    assert!(!executable_in("agent", path("~"), h));
    // No HOME: the entry names nothing.
    assert!(!executable_in("agent", path("~/bin"), None));
    // `~user` is not the user's home; an absolute entry is read as it is.
    assert!(!executable_in("agent", path("~nobody/bin"), h));
    let absolute = home.path().join("bin");
    assert!(executable_in("agent", Some(absolute.as_os_str()), None));
    // A binary with a `/` never reads PATH.
    assert!(!executable_in("bin/agent", path("~"), h));
    assert!(!executable_in("agent", None, h));
}
