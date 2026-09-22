use super::*;
use crate::app::{App, Link};
use crate::settings::UiSettings;
use proto::{GitOperation, Runtime, Status, WindowInfo};
use ratatui::{Terminal, backend::TestBackend, buffer::Buffer};
use std::path::PathBuf;
use unicode_width::UnicodeWidthStr;

fn clean_state() -> GitState {
    GitState {
        head: Head::Branch("main".into()),
        upstream: None,
        ahead: 0,
        behind: 0,
        dirty: 0,
        untracked: 0,
        conflicts: 0,
        operation: None,
        stale: false,
    }
}

fn text(spans: &[Span<'_>]) -> String {
    spans.iter().map(|s| s.content.as_ref()).collect()
}

#[test]
fn renders_a_clean_repository_as_a_tick() {
    assert_eq!(text(&git_spans(&clean_state(), 80)), "main ✓");
}

#[test]
fn renders_dirty_untracked_and_divergence() {
    let mut state = clean_state();
    state.dirty = 3;
    state.untracked = 1;
    state.ahead = 2;
    state.behind = 1;
    assert_eq!(text(&git_spans(&state, 80)), "main ●3 ?1 ⇡2⇣1");

    // Zero untracked and zero divergence omit those parts entirely.
    let mut only_dirty = clean_state();
    only_dirty.dirty = 3;
    assert_eq!(text(&git_spans(&only_dirty, 80)), "main ●3");
}

#[test]
fn renders_conflicts_and_the_operation() {
    let mut state = clean_state();
    state.conflicts = 2;
    state.operation = Some(GitOperation::Rebase);
    assert_eq!(text(&git_spans(&state, 80)), "main ⚠2 rebase");
}

#[test]
fn renders_a_detached_head() {
    let mut state = clean_state();
    state.head = Head::Detached("a1b2c3d".into());
    state.dirty = 1;
    assert_eq!(text(&git_spans(&state, 80)), "@a1b2c3d ●1");
}

#[test]
fn renders_an_unborn_branch() {
    let mut state = clean_state();
    state.head = Head::Unborn("main".into());
    assert_eq!(text(&git_spans(&state, 80)), "main (unborn)");
}

#[test]
fn an_unborn_branch_still_counts_and_still_goes_stale() {
    // The unborn head used to return early, so the bar could only ever say
    // `main (unborn)`: a freshly scaffolded project showed no untracked count, and a
    // failing probe against one showed a confident head with nothing marking it stale.
    let mut fresh = clean_state();
    fresh.head = Head::Unborn("main".into());
    fresh.untracked = 4;
    assert_eq!(text(&git_spans(&fresh, 80)), "main (unborn) ?4");

    let mut stale = fresh.clone();
    stale.stale = true;
    assert_eq!(text(&git_spans(&stale, 80)), "main (unborn) ?4 (stale)");

    // `(unborn)` qualifies the head itself, so it is the last part to go: at a budget
    // that fits only one of them, it is `(stale)` (priority 5) that is dropped.
    let budget = UnicodeWidthStr::width("main (unborn)");
    assert_eq!(text(&git_spans(&stale, budget)), "main (unborn)");
}

#[test]
fn a_clean_tree_mid_rebase_is_not_a_tick() {
    // `GitState::is_clean` counts the operation, and the bar asks it rather than
    // deciding cleanliness a second way from the parts it built.
    let mut state = clean_state();
    state.operation = Some(GitOperation::Rebase);
    assert!(!state.is_clean());
    assert_eq!(text(&git_spans(&state, 80)), "main rebase");
}

#[test]
fn renders_stale() {
    let mut state = clean_state();
    state.dirty = 3;
    state.stale = true;
    assert_eq!(text(&git_spans(&state, 80)), "main ●3 (stale)");
}

#[test]
fn drops_parts_right_to_left_when_the_budget_shrinks() {
    let mut state = clean_state();
    state.dirty = 3;
    state.untracked = 1;
    state.ahead = 2;
    state.behind = 1;
    state.operation = Some(GitOperation::Rebase);

    let full = "main ●3 ?1 ⇡2⇣1 rebase";
    assert_eq!(text(&git_spans(&state, 80)), full);

    let without_operation = "main ●3 ?1 ⇡2⇣1";
    let budget = UnicodeWidthStr::width(without_operation);
    assert_eq!(text(&git_spans(&state, budget)), without_operation);

    let without_untracked = "main ●3 ⇡2⇣1";
    let budget = UnicodeWidthStr::width(without_untracked);
    assert_eq!(text(&git_spans(&state, budget)), without_untracked);

    let without_divergence = "main ●3";
    let budget = UnicodeWidthStr::width(without_divergence);
    assert_eq!(text(&git_spans(&state, budget)), without_divergence);

    let head_only = "main";
    let budget = UnicodeWidthStr::width(head_only);
    assert_eq!(text(&git_spans(&state, budget)), head_only);
}

#[test]
fn stale_outlives_the_named_parts_when_the_budget_shrinks() {
    // Controller ruling: `(stale)` is a trust flag on the numbers beside it, not another
    // datum competing with them, so it shares conflicts' priority rather than being the
    // first thing dropped. It must survive operation/untracked/divergence/dirty being
    // dropped in turn, and only disappear once nothing is left to qualify.
    let mut state = clean_state();
    state.dirty = 3;
    state.untracked = 1;
    state.ahead = 2;
    state.behind = 1;
    state.operation = Some(GitOperation::Rebase);
    state.stale = true;

    let full = "main ●3 ?1 ⇡2⇣1 rebase (stale)";
    assert_eq!(text(&git_spans(&state, 80)), full);

    let without_operation = "main ●3 ?1 ⇡2⇣1 (stale)";
    let budget = UnicodeWidthStr::width(without_operation);
    assert_eq!(text(&git_spans(&state, budget)), without_operation);

    let without_untracked = "main ●3 ⇡2⇣1 (stale)";
    let budget = UnicodeWidthStr::width(without_untracked);
    assert_eq!(text(&git_spans(&state, budget)), without_untracked);

    // A count and `(stale)` still surviving together at a tight budget.
    let without_divergence = "main ●3 (stale)";
    let budget = UnicodeWidthStr::width(without_divergence);
    assert_eq!(text(&git_spans(&state, budget)), without_divergence);

    // Dirty (priority 4) is still lower than `(stale)` (priority 5, matching conflicts),
    // so it is the next to go, leaving `(stale)` alone beside the head.
    let stale_alone = "main (stale)";
    let budget = UnicodeWidthStr::width(stale_alone);
    assert_eq!(text(&git_spans(&state, budget)), stale_alone);

    let head_only = "main";
    let budget = UnicodeWidthStr::width(head_only);
    assert_eq!(text(&git_spans(&state, budget)), head_only);
}

#[test]
fn is_hidden_when_it_cannot_fit_the_head() {
    let state = clean_state(); // head "main" has width 4
    assert!(git_spans(&state, 3).is_empty());
}

fn window(id: u32, worktree: Option<PathBuf>) -> WindowInfo {
    WindowInfo {
        id,
        name: format!("w{id}"),
        runtime: Runtime::Shell,
        cwd: "/tmp".into(),
        project: "/tmp".into(),
        worktree,
        branch: None,
        status: Status::Idle,
        tool: None,
        since_secs: 0,
        last_output_secs: 0,
        session_id: None,
        model: None,
        subagents: vec![],
        exit: None,
    }
}

fn render_row(app: &App, width: u16) -> Buffer {
    let area = Rect::new(0, 0, width, 1);
    let mut terminal = Terminal::new(TestBackend::new(width, 1)).unwrap();
    terminal.draw(|f| render(f, app, area)).unwrap();
    terminal.backend().buffer().clone()
}

fn row_text(buffer: &Buffer) -> String {
    let area = buffer.area;
    (area.x..area.right())
        .map(|x| buffer[(x, area.y)].symbol())
        .collect()
}

#[test]
fn no_segment_without_a_focused_worktree() {
    let mut app = App::new(vec![window(1, None)], "/tmp".into(), UiSettings::default());
    app.set_terminal_size(80, 24);
    let buffer = render_row(&app, 80);
    let text = row_text(&buffer);
    // '?' is deliberately excluded: it's also part of the literal "C-b ?" help hint, so
    // checking for it here would flag the hints themselves rather than a git segment.
    for glyph in ['✓', '●', '⇡', '⇣', '⚠'] {
        assert!(
            !text.contains(glyph),
            "expected no git glyphs without a focused worktree, got {text:?}"
        );
    }
    assert!(!text.contains("(stale)"));
    assert!(!text.contains("(unborn)"));
}

#[test]
fn hints_drop_from_the_right_one_at_a_time() {
    // Decision 20 drops key hints from the right *one at a time*. The other render
    // tests only ever hit the ends of that range — no git segment at all, or a budget
    // so tight that all five hints are gone at once — so this pins an intermediate
    // width, where the first three hints survive and the last two do not.
    let mut app = App::new(
        vec![window(1, Some("/repo".into()))],
        "/tmp".into(),
        UiSettings::default(),
    );
    app.set_terminal_size(50, 24);
    let mut state = clean_state();
    state.dirty = 3;
    app.on_daemon(proto::DaemonMsg::Git {
        root: "/repo".into(),
        state: Some(state),
    });

    let text = row_text(&render_row(&app, 50));

    assert!(
        text.contains("main ●3"),
        "the git segment must survive whole while hints are still being dropped: {text:?}"
    );
    for kept in ["C-b ?", "C-b c", "C-b t"] {
        assert!(
            text.contains(kept),
            "hint {kept:?} still fits and must be rendered: {text:?}"
        );
    }
    for dropped in ["C-b j/k", "C-b d"] {
        assert!(
            !text.contains(dropped),
            "hint {dropped:?} does not fit and must be dropped: {text:?}"
        );
    }
}

#[test]
fn the_toast_is_never_overwritten_by_git() {
    let mut app = App::new(
        vec![window(1, Some("/repo".into()))],
        "/tmp".into(),
        UiSettings::default(),
    );
    app.set_terminal_size(80, 24);
    let mut state = clean_state();
    state.dirty = 3;
    app.on_daemon(proto::DaemonMsg::Git {
        root: "/repo".into(),
        state: Some(state),
    });
    app.toast("a rather long toast message taking up real room");

    let width = 60;
    let buffer = render_row(&app, width);
    let toast_text = app.toast_text().unwrap();
    let toast_width = (UnicodeWidthStr::width(toast_text) as u16 + 1).min(width);
    let start = width - toast_width;

    // The git segment must actually be present (and not itself truncated away) in the room
    // left of the toast — otherwise this test would pass with no git segment at all.
    let before_toast: String = (0..start).map(|x| buffer[(x, 0)].symbol()).collect();
    assert!(
        before_toast.contains("main ●3"),
        "expected the dirty git segment before the toast, got {before_toast:?}"
    );

    let rendered: String = (start..width).map(|x| buffer[(x, 0)].symbol()).collect();
    assert!(
        rendered.trim_end().ends_with(toast_text),
        "toast cells were not intact: {rendered:?}"
    );
}

/// Task M6.11, decision 34: the persistent `DISCONNECTED` badge and its status text,
/// for both `Link::Reconnecting` and `Link::Lost`.
#[test]
fn statusbar_shows_reconnect_state() {
    let mut app = App::new(vec![window(1, None)], "/tmp".into(), UiSettings::default());
    app.set_terminal_size(80, 24);

    app.link = Link::Reconnecting {
        attempts: 3,
        reason: "x".into(),
    };
    let text = row_text(&render_row(&app, 80));
    assert!(text.contains("DISCONNECTED"), "{text:?}");
    assert!(text.contains("reconnecting (attempt 3)"), "{text:?}");

    app.link = Link::Lost { reason: "x".into() };
    let text = row_text(&render_row(&app, 80));
    assert!(text.contains("DISCONNECTED"), "{text:?}");
    assert!(text.contains("C-b r to reconnect"), "{text:?}");
}
