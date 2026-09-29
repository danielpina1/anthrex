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

/// M9.17 fix round 3, item 3: an empty `HOME` is no home; `~/bin` is not read as the
/// relative `bin`.
#[test]
fn an_empty_home_expands_nothing() {
    use std::path::{Path, PathBuf};
    let entry =
        |dir: &str, home: Option<&str>| super::path_entry(PathBuf::from(dir), home.map(OsStr::new));
    assert_eq!(entry("~/bin", Some("")), None);
    assert_eq!(entry("~", Some("")), None);
    assert_eq!(entry("~/bin", None), None);
    assert_eq!(
        entry("~/bin", Some("/h")),
        Some(Path::new("/h/bin").to_path_buf())
    );
    assert_eq!(entry("/usr/bin", Some("")), Some(PathBuf::from("/usr/bin")));
    assert_eq!(entry("~x/bin", Some("/h")), Some(PathBuf::from("~x/bin")));
}

/// M9.17 fix round 3, item 3: the check matches what launches the binary. Only the
/// orchestrator's window goes through a shell (`/bin/sh -c 'exec "$0" "$@"'`), and only
/// macOS's `/bin/sh` (bash) expands `~` in `PATH`; a headless session (a sub-planner, a
/// scout) is spawned directly, whose `PATH` search never expands it.
#[test]
fn only_a_window_on_macos_finds_a_binary_through_a_tilde_entry() {
    let home = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(home.path().join("bin")).unwrap();
    let agent = home.path().join("bin/claude");
    std::fs::write(&agent, "#!/bin/sh\n").unwrap();
    std::fs::set_permissions(&agent, std::fs::Permissions::from_mode(0o755)).unwrap();
    let path = Some(OsStr::new("~/bin"));
    let h = Some(home.path().as_os_str());
    let yes = |map: &std::collections::BTreeMap<String, bool>| map.get("claude") == Some(&true);

    let mac = super::found_in(("claude", "codex"), path, h, true);
    assert!(yes(&mac.window), "a macOS window's shell expands it");
    assert!(!yes(&mac.headless), "a headless spawn does not");
    let linux = super::found_in(("claude", "codex"), path, h, false);
    assert!(!yes(&linux.window) && !yes(&linux.headless));
    let empty = super::found_in(("claude", "codex"), path, Some(OsStr::new("")), true);
    assert!(!yes(&empty.window));
    // An absolute entry is found by both.
    let absolute = home.path().join("bin");
    let both = super::found_in(("claude", "codex"), Some(absolute.as_os_str()), None, false);
    assert!(yes(&both.window) && yes(&both.headless));
    assert_eq!(both.headless.get("codex"), Some(&false));
}
