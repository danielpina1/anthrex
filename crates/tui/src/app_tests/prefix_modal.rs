//! Milestone 9.11 task M9.11.23's small fixes: a modal clears a pending prefix
//! (decision 24, E2), a task never lists itself as a dependency (E25), the force
//! dialog lists its safe keys first (decision 25, B9), and the conversation view
//! drops hidden format characters (decision 28, D2).

use super::*;
use proto::{Size, TaskState};
use ratatui::Terminal;
use ratatui::backend::TestBackend;

/// A window this daemon made a worktree for, so the remove dialog offers the checkbox.
fn wt_win(id: u32, name: &str, branch: &str) -> WindowInfo {
    let mut window = win(id, name, Status::Idle);
    window.branch = Some(branch.to_string());
    window
}

#[test]
fn a_modal_opened_while_the_prefix_waits_clears_it() {
    let mut app = app_with(vec![wt_win(1, "api", "feat/api")]);
    // `C-b X`, tick the worktree box, confirm: the removal is in flight.
    prefix(&mut app);
    assert!(press(&mut app, KeyCode::Char('X'), KeyModifiers::SHIFT).is_empty());
    assert!(press(&mut app, KeyCode::Char('w'), KeyModifiers::NONE).is_empty());
    assert_eq!(
        press(&mut app, KeyCode::Char('y'), KeyModifiers::NONE).len(),
        1
    );
    // The user presses the prefix; before the next key, the daemon's refusal opens the
    // force-removal dialog.
    prefix(&mut app);
    assert!(app.keymap.pending());
    assert!(
        app.on_daemon(DaemonMsg::RemoveDirty {
            window_id: 1,
            message: "worktree /w has uncommitted changes".into(),
        })
        .is_empty()
    );
    assert!(matches!(app.modal, Some(Modal::ForceRemove { .. })));
    // `n` closes the dialog; the prefix it was opened over is gone with it.
    assert!(press(&mut app, KeyCode::Char('n'), KeyModifiers::NONE).is_empty());
    assert!(app.modal.is_none());
    assert!(
        !app.keymap.pending(),
        "the modal cleared the pending prefix"
    );
    let effects = press(&mut app, KeyCode::Char('d'), KeyModifiers::NONE);
    assert!(
        !effects.contains(&Effect::Quit),
        "`d` did not detach: {effects:?}"
    );
    assert_eq!(
        effects,
        vec![Effect::Send(ClientMsg::Input {
            window_id: 1,
            bytes: b"d".to_vec(),
        })],
        "`d` reached the PTY"
    );
}

#[test]
fn a_tick_under_an_open_modal_clears_a_pending_prefix() {
    let mut app = app_with(vec![win(1, "a", Status::Idle)]);
    prefix(&mut app);
    app.report_config_problems(vec!["a problem".into()]);
    assert!(app.modal.is_some());
    app.on_tick();
    assert!(!app.keymap.pending(), "the open modal cleared the prefix");
}

#[test]
fn all_deps_never_lists_the_task_itself() {
    let mut t = crate::tree::run_fixtures::task("t2", "the task", Size::M, TaskState::Pending);
    t.deps = vec!["t1".into(), "t2".into()];
    t.implicit_deps = vec!["t2".into(), "t3".into(), "t1".into()];
    assert_eq!(
        crate::app::plan_review::all_deps(&t),
        vec!["t1", "t3"],
        "explicit then implicit, without repeats and without the task's own id"
    );
}

/// The force dialog at a height whose interior holds the name line, a one-line message,
/// the blank line and exactly one key line (4 interior rows + 2 borders = 6): the key
/// shown is the safe `n`, never `f`.
#[test]
fn a_short_force_dialog_shows_cancel_before_force() {
    let mut terminal = Terminal::new(TestBackend::new(80, 6)).unwrap();
    terminal
        .draw(|frame| {
            crate::ui::dialog::render_force_remove(
                frame,
                "api",
                "worktree /w has uncommitted changes",
                frame.area(),
                crate::theme::Palette::PLAIN,
            )
        })
        .unwrap();
    let out = terminal.backend().to_string();
    assert!(out.contains("n  cancel"), "{out}");
    assert!(!out.contains("f  force"), "{out}");
    assert!(!out.contains("k  keep"), "{out}");
}

#[test]
fn clean_drops_hidden_format_characters() {
    assert_eq!(crate::conversation::clean("a\u{202E}b\u{200B}c"), "abc");
}
