use super::*;
use proto::{GitState, Head};

fn state(dirty: u32) -> GitState {
    GitState {
        head: Head::Branch("main".into()),
        upstream: None,
        ahead: 0,
        behind: 0,
        dirty,
        untracked: 0,
        conflicts: 0,
        operation: None,
        stale: false,
    }
}

#[test]
fn git_state_is_stored_by_root() {
    let mut app = app_with(vec![win(1, "a", Status::Idle)]);
    let root = PathBuf::from("/repo");

    assert!(
        app.on_daemon(DaemonMsg::Git {
            root: root.clone(),
            state: Some(state(1)),
        })
        .is_empty()
    );
    assert_eq!(app.git.get(&root), Some(&state(1)));

    assert!(
        app.on_daemon(DaemonMsg::Git {
            root: root.clone(),
            state: None,
        })
        .is_empty()
    );
    assert!(!app.git.contains_key(&root));
}

#[test]
fn focused_git_follows_the_focused_window() {
    let mut a = win(1, "a", Status::Idle);
    a.worktree = Some("/repo/a".into());
    let mut b = win(2, "b", Status::Idle);
    b.worktree = Some("/repo/b".into());
    let c = win(3, "c", Status::Idle); // worktree: None

    let mut app = app_with(vec![a, b, c]);
    app.on_daemon(DaemonMsg::Git {
        root: "/repo/a".into(),
        state: Some(state(1)),
    });
    app.on_daemon(DaemonMsg::Git {
        root: "/repo/b".into(),
        state: Some(state(2)),
    });

    app.focus(1);
    assert_eq!(app.focused_git(), Some(&state(1)));

    app.focus(2);
    assert_eq!(app.focused_git(), Some(&state(2)));

    app.focus(3);
    assert_eq!(app.focused_git(), None);
}

#[test]
fn git_state_is_pruned_when_its_last_window_goes() {
    let mut a = win(1, "a", Status::Idle);
    a.worktree = Some("/repo/a".into());
    let mut app = app_with(vec![a]);
    app.on_daemon(DaemonMsg::Git {
        root: "/repo/a".into(),
        state: Some(state(1)),
    });
    assert!(app.git.contains_key(&PathBuf::from("/repo/a")));

    // The window list changes and no window references /repo/a any more. The daemon
    // publishes nothing for an unregistered root, so the client must drop it itself.
    let mut b = win(2, "b", Status::Idle);
    b.worktree = Some("/repo/b".into());
    app.on_daemon(DaemonMsg::WindowsChanged { windows: vec![b] });

    assert!(!app.git.contains_key(&PathBuf::from("/repo/a")));
}

#[test]
fn git_state_survives_when_its_window_is_merely_reordered() {
    let mut a = win(1, "a", Status::Idle);
    a.worktree = Some("/repo/a".into());
    let mut app = app_with(vec![a.clone()]);
    app.on_daemon(DaemonMsg::Git {
        root: "/repo/a".into(),
        state: Some(state(1)),
    });

    // Same window, same worktree, just resent as part of a larger list.
    let mut b = win(2, "b", Status::Idle);
    b.worktree = Some("/repo/b".into());
    app.on_daemon(DaemonMsg::WindowsChanged {
        windows: vec![b, a],
    });

    assert!(app.git.contains_key(&PathBuf::from("/repo/a")));
}
