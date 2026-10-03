//! Milestone 9.3 task 9b fix round 1: the goal dialog's replies that race the dialog
//! (decision 8's drafts): another project's success first, a triage that started no
//! run, a success that finds the same project's dialog open again, and the confirm
//! page's Ctrl-S.

use super::goal_editor::{
    ctrl, drawn, open_on, refused, screen, send_and_close, started, tap, triaged,
};
use super::goal_form::{app, form, typed};
use super::orch::tagged;
use super::*;
use proto::{DeciderSource, RunPath, RunReply, Scale, TaskKind, TriageInfo};
use std::path::PathBuf;

/// `Triaged` with no run: triage refused the goal (a refused start).
fn triaged_no_run(id: u64) -> DaemonMsg {
    DaemonMsg::Run(RunReply::Triaged {
        triage: TriageInfo {
            kinds: vec![TaskKind::Code],
            scale: Scale::Plan,
            path: RunPath::Plan,
            reason: "r".into(),
            source: DeciderSource::Decider,
            fallback_reason: None,
            at: 0,
        },
        run_id: None,
        message: "not a goal for a run".into(),
        request_id: Some(id),
    })
}

/// Review I1: `/p/a` closed while it waits, `/p/b` started and answered first, then
/// `/p/a`'s success: `/p/a`'s draft goes, as any successful start's does.
#[test]
fn a_success_after_another_projects_clears_its_draft() {
    let mut app = app();
    drawn(&mut app);
    open_on(&mut app, "/p/a");
    typed(&mut app, "goal a");
    let a = send_and_close(&mut app);
    open_on(&mut app, "/p/b");
    typed(&mut app, "goal b");
    let (b, _) = tagged(&ctrl(&mut app, 's'));
    app.on_daemon(triaged(b));
    assert_eq!(app.modal, None);
    assert_eq!(app.goal_drafts[&PathBuf::from("/p/a")], "goal a");
    app.on_daemon(triaged(a));
    assert!(app.goal_drafts.is_empty(), "{:?}", app.goal_drafts);
    // A refusal for a closed dialog's request ends its wait: the draft stays, and a
    // reply for that id later clears nothing.
    open_on(&mut app, "/p/a");
    typed(&mut app, "goal c");
    let c = send_and_close(&mut app);
    app.on_daemon(refused(c));
    app.on_daemon(started(c));
    assert_eq!(app.goal_drafts[&PathBuf::from("/p/a")], "goal c");
}

/// Review m3: a `Triaged` that names no run is a refused start: the draft is kept,
/// whether the dialog was open or closed while it waited.
#[test]
fn a_triage_with_no_run_keeps_the_draft() {
    let mut app = app();
    open_on(&mut app, "/p/a");
    typed(&mut app, "add a");
    let (id, _) = tagged(&ctrl(&mut app, 's'));
    app.on_daemon(triaged_no_run(id));
    assert_eq!(app.modal, None);
    assert_eq!(app.toast_text(), Some("not a goal for a run"));
    assert_eq!(app.goal_drafts[&PathBuf::from("/p/a")], "add a");
    open_on(&mut app, "/p/a");
    typed(&mut app, " now");
    let id = send_and_close(&mut app);
    app.on_daemon(triaged_no_run(id));
    assert_eq!(app.goal_drafts[&PathBuf::from("/p/a")], "add a now");
}

/// Review m4: a success that finds the same project's dialog open again clears its
/// editor only while it still holds exactly the goal sent, and is toasted either way.
#[test]
fn a_late_success_clears_the_reopened_editor_only_if_unchanged() {
    for (edit, left) in [("", ""), ("!", "add a!")] {
        let mut app = app();
        drawn(&mut app);
        open_on(&mut app, "/p/a");
        typed(&mut app, "add a");
        let id = send_and_close(&mut app);
        open_on(&mut app, "/p/a");
        typed(&mut app, edit);
        app.on_daemon(started(id));
        assert_eq!(form(&app).goal.text(), left, "{edit:?}");
        assert!(!form(&app).submitting);
        assert_eq!(app.toast_text(), Some("run r-next started"));
        assert!(app.goal_drafts.is_empty());
    }
    // `Triaged` too.
    let mut app = app();
    open_on(&mut app, "/p/a");
    typed(&mut app, "add a");
    let id = send_and_close(&mut app);
    open_on(&mut app, "/p/a");
    app.on_daemon(triaged(id));
    assert_eq!(form(&app).goal.text(), "");
    assert_eq!(app.toast_text(), Some("run r-new started"));
}

/// Review m6: Ctrl-S on the confirm page is "any other key": back to the text, nothing
/// sent.
#[test]
fn ctrl_s_on_the_confirm_page_goes_back() {
    let mut app = app();
    drawn(&mut app);
    open_on(&mut app, "/p/a");
    typed(&mut app, "add a");
    tap(&mut app, KeyCode::Esc);
    assert!(form(&app).discarding);
    assert!(ctrl(&mut app, 's').is_empty());
    assert!(!form(&app).discarding && !form(&app).submitting);
    assert_eq!(form(&app).goal.text(), "add a");
    assert!(screen(&app).contains("^S start"));
}
