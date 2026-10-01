//! Milestone 9.0.6 task 10: the accept page and the moved-base page (decisions 14, 18;
//! review focus 4).

use super::actions::{action, flow, sent_tagged_id};
use super::runs::app_with_runs;
use super::*;
use crate::actions_request::ActionTarget;
use crate::app::actions::ActionStep;
use crate::tree::run_fixtures::{run, snapshot, task};
use crate::tree::stage_fixtures::stage;
use proto::{
    ActionKind, BaseMovedInfo, FinishAction, FullState, RunReply, RunRequest, RunState, Size,
    TaskState,
};

/// A complete two-stage run `id` (one merged task per stage, both tier 3s green) on
/// `main@b0b0b0b…`, head `d1d1d1d…`, both tasks started (each had a worktree), with
/// `Accept` and `Discard` listed.
pub(super) fn app_with_complete_run(id: &str) -> App {
    let mut info = run(id, "/tmp/r", RunState::Complete);
    info.goal = "Add mul()".into();
    info.base_sha = "b0".repeat(20);
    info.run_branch = format!("anthrex/{id}/integration");
    info.run_head = "d1".repeat(20);
    let mut t2 = task("t2", "mul tests", Size::S, TaskState::Merged);
    t2.stage = 2;
    info.tasks = vec![task("t1", "mul", Size::S, TaskState::Merged), t2];
    for t in &mut info.tasks {
        t.start_commit = Some("b0".repeat(20));
    }
    let mut one = stage(1, Some(&"1".repeat(40)), 1, 1);
    one.full.state = FullState::Green;
    let mut two = stage(2, Some(&"2".repeat(40)), 1, 1);
    two.full.state = FullState::Green;
    info.stages = vec![one, two];
    info.actions = vec![
        action(ActionKind::Accept, "accept", None),
        action(ActionKind::Discard, "discard", None),
    ];
    app_with_runs(vec![], snapshot(10_000, vec![info]))
}

/// Review focus 4: the resend after a moved base carries `<id>@<full to>`.
#[test]
fn moved_base_accept_resends_the_full_head() {
    let mut app = app_with_complete_run("add-mul-0723"); // a fixture helper in this file
    let mut effects = app.open_actions(
        ("add-mul-0723".into(), ActionTarget::Run),
        Some(ActionKind::Accept),
    );
    effects.extend(press(&mut app, KeyCode::Enter, KeyModifiers::NONE)); // the Accept entry
    effects.extend(press(&mut app, KeyCode::Char('y'), KeyModifiers::NONE)); // its page
    let id = sent_tagged_id(&effects, |r| {
        matches!(r,
        RunRequest::Finish { action: FinishAction::Accept, confirm: Some(c), .. } if c == "add-mul-0723")
    });
    let to = "a".repeat(40);
    app.on_run_reply(RunReply::ConfirmNeeded {
        run_id: "add-mul-0723".into(),
        prompt: "merge anthrex/add-mul-0723/integration into main in /tmp/r?".into(),
        base_moved: Some(BaseMovedInfo {
            from: "b".repeat(40),
            to: to.clone(),
            commits: vec!["aaaaaaa fix the readme".into()],
            total: 1,
        }),
        request_id: Some(id),
    });
    assert!(
        matches!(&app.modal, Some(Modal::Action(flow)) if matches!(flow.step, ActionStep::MovedBase(_)))
    );
    for c in "072".chars() {
        press(&mut app, KeyCode::Char(c), KeyModifiers::NONE);
    }
    assert!(
        press(&mut app, KeyCode::Enter, KeyModifiers::NONE).is_empty(),
        "a wrong id sends nothing"
    );
    let ActionStep::MovedBase(page) = &flow(&app).step else {
        panic!("the page stays");
    };
    assert!(page.wrong, "`type 0723 exactly` shows");
    press(&mut app, KeyCode::Char('3'), KeyModifiers::NONE);
    let resent = press(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    let again = sent_tagged_id(&resent, |r| {
        matches!(r,
        RunRequest::Finish { action: FinishAction::Accept, confirm: Some(c), .. }
            if *c == format!("add-mul-0723@{to}"))
    });
    assert!(app.modal.is_none());
    assert!(
        app.replies.contains(again),
        "the resend waits for its reply"
    );
}

#[test]
fn accept_page_lists_stages_base_and_head() {
    let mut app = app_with_complete_run("add-mul-0723");
    app.open_actions(
        ("add-mul-0723".into(), ActionTarget::Run),
        Some(ActionKind::Accept),
    );
    press(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    let ActionStep::Confirm(page) = &flow(&app).step else {
        panic!("not a page");
    };
    let rows: Vec<(&str, &str)> = page
        .details
        .iter()
        .map(|(l, v)| (l.as_str(), v.as_str()))
        .collect();
    assert_eq!(
        rows,
        vec![
            ("stage 1", "1/1 merged · tier 3 ✓ green"),
            ("stage 2", "1/1 merged · tier 3 ✓ green"),
            ("base", "main@b0b0b0b"),
            ("merges", "anthrex/add-mul-0723/integration@d1d1d1d"),
        ]
    );
}

#[test]
fn accept_page_of_a_run_without_stages_counts_tasks() {
    let mut info = run("r-0001", "/tmp/r", RunState::Complete);
    info.base_sha = "c".repeat(40);
    info.run_branch = "anthrex/r-0001/integration".into();
    info.run_head = "e".repeat(40);
    info.tasks = vec![
        task("t1", "a", Size::S, TaskState::Merged),
        task("t2", "b", Size::S, TaskState::Cancelled),
    ];
    info.actions = vec![action(ActionKind::Accept, "accept", None)];
    let mut app = app_with_runs(vec![], snapshot(10_000, vec![info]));
    app.settings.badges.ascii = true;
    app.open_actions(("r-0001".into(), ActionTarget::Run), None);
    press(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    let ActionStep::Confirm(page) = &flow(&app).step else {
        panic!("not a page");
    };
    assert_eq!(page.details[0], ("tasks".into(), "1/2 merged".into()));
    assert_eq!(page.details[1], ("base".into(), "main@ccccccc".into()));
}

#[test]
fn discard_and_cancel_pages_say_what_goes() {
    let mut app = app_with_complete_run("add-mul-0723");
    app.open_actions(
        ("add-mul-0723".into(), ActionTarget::Run),
        Some(ActionKind::Discard),
    );
    press(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    let ActionStep::Confirm(page) = &flow(&app).step else {
        panic!("not a page");
    };
    assert_eq!(
        page.details,
        vec![
            (
                "removes".to_string(),
                "the run's remaining worktrees and its anthrex/add-mul-0723/* branches".to_string()
            ),
            (
                "keeps".to_string(),
                "main unchanged · uncommitted work under refs/anthrex/salvage/add-mul-0723/"
                    .to_string()
            ),
        ]
    );

    let mut app = super::actions::running_app();
    app.open_actions(
        (crate::tree::run_fixtures::RUN_ID.into(), ActionTarget::Run),
        Some(ActionKind::Cancel),
    );
    press(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    let ActionStep::Confirm(page) = &flow(&app).step else {
        panic!("not a page");
    };
    assert_eq!(
        page.details,
        vec![
            ("stops".to_string(), "1 worker: t1".to_string()),
            ("cancels".to_string(), "2 unmerged tasks".to_string()),
        ]
    );
}

/// The base moved again under the resend: the page reopens on the newer `to`, and the
/// next resend carries it in full.
#[test]
fn a_second_moved_base_reopens_the_page_on_the_new_head() {
    let mut app = app_with_complete_run("add-mul-0723");
    app.open_actions(
        ("add-mul-0723".into(), ActionTarget::Run),
        Some(ActionKind::Accept),
    );
    press(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    let mut id = sent_tagged_id(
        &press(&mut app, KeyCode::Char('y'), KeyModifiers::NONE),
        |_| true,
    );
    for to in ["a".repeat(40), "c".repeat(40)] {
        app.on_run_reply(RunReply::ConfirmNeeded {
            run_id: "add-mul-0723".into(),
            prompt: "merge?".into(),
            base_moved: Some(BaseMovedInfo {
                from: "b".repeat(40),
                to: to.clone(),
                commits: vec![],
                total: 1,
            }),
            request_id: Some(id),
        });
        let ActionStep::MovedBase(page) = &flow(&app).step else {
            panic!("no moved-base page");
        };
        assert_eq!(page.moved.to, to);
        assert_eq!(page.typed, "", "a fresh box");
        for c in "0723".chars() {
            press(&mut app, KeyCode::Char(c), KeyModifiers::NONE);
        }
        let resent = press(&mut app, KeyCode::Enter, KeyModifiers::NONE);
        id = sent_tagged_id(&resent, |r| {
            matches!(r, RunRequest::Finish { confirm: Some(c), .. }
                if *c == format!("add-mul-0723@{to}"))
        });
    }
}
