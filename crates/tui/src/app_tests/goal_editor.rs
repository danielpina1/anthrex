//! Milestone 9.3 task 9b: the goal dialog's keys (decision 7), its confirm page and
//! drafts (decision 8), and the orchestrator row (decision 25), reached by the keys a
//! user presses.

use super::goal_form::{app, focus, form, open_form, typed};
use super::orch::tagged;
use super::runs::{deliver, run_info, snapshot};
use super::*;
use crate::run_goal::GoalField;
use crate::ui::audit;
use proto::run_wire::request::START_GOAL;
use proto::{
    DeciderSource, IdleOrchestrator, OrchestratorChoice, RunPath, RunReply, RunRequest, RunState,
    Scale, TaskKind, TriageInfo,
};
use std::path::PathBuf;

pub(super) fn tap(app: &mut App, code: KeyCode) -> Vec<Effect> {
    press(app, code, KeyModifiers::NONE)
}

pub(super) fn ctrl(app: &mut App, c: char) -> Vec<Effect> {
    press(app, KeyCode::Char(c), KeyModifiers::CONTROL)
}

/// The last frame was 80x24: the large dialog's text is 72 columns by 10 rows.
pub(super) fn drawn(app: &mut App) {
    app.set_body_area(ratatui::layout::Rect::new(0, 0, 80, 23));
}

fn idle(project: &str, fresh: bool) -> IdleOrchestrator {
    IdleOrchestrator {
        project: project.into(),
        ..crate::ui::goal_editor::tests::idle(fresh)
    }
}

/// An app whose snapshot holds `idle` and `runs`.
fn app_with_chains(idle: Vec<IdleOrchestrator>, runs: Vec<proto::RunInfo>) -> App {
    let mut app = app();
    let mut snap = snapshot(2, 100, runs);
    snap.idle_orchestrators = idle;
    deliver(&mut app, snap);
    drawn(&mut app);
    app
}

/// Opens the dialog on `project` (its row selected, `C-b g`).
pub(super) fn open_on(app: &mut App, project: &str) {
    let rows = crate::tree::build_with_runs(&app.windows, &app.runs.runs, &app.tree);
    app.tree
        .select(&rows, crate::tree::NodeKey::Project(project.into()));
    prefix(app);
    assert!(press(app, KeyCode::Char('g'), KeyModifiers::NONE).is_empty());
    assert_eq!(form(app).project, PathBuf::from(project));
}

fn goal_of(request: &RunRequest) -> &str {
    match request {
        RunRequest::StartGoal { goal, .. } => goal,
        other => panic!("{other:?}"),
    }
}

pub(super) fn triaged(id: u64) -> DaemonMsg {
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
        run_id: Some("r-new".into()),
        message: "planning run r-new".into(),
        request_id: Some(id),
    })
}

pub(super) fn started(id: u64) -> DaemonMsg {
    DaemonMsg::Run(RunReply::Started {
        run_id: "r-next".into(),
        state: RunState::Planning,
        request_id: Some(id),
    })
}

pub(super) fn refused(id: u64) -> DaemonMsg {
    DaemonMsg::Run(RunReply::Refused {
        request: START_GOAL.into(),
        message: "not a git repository: /p/a".into(),
        request_id: Some(id),
    })
}

#[test]
fn ctrl_s_starts_from_the_text_and_from_every_row() {
    let fields = [
        GoalField::Goal,
        GoalField::Runtime,
        GoalField::Model,
        GoalField::Orchestrator,
        GoalField::Delivery,
        GoalField::Trust,
        GoalField::Yes,
        GoalField::UnconfinedChecks,
    ];
    for field in fields {
        let mut app = app();
        drawn(&mut app);
        open_form(&mut app);
        typed(&mut app, "add a");
        // Enter in the text is a newline, never a start.
        assert!(tap(&mut app, KeyCode::Enter).is_empty());
        typed(&mut app, "b");
        focus(&mut app, field);
        let (_, request) = tagged(&ctrl(&mut app, 's'));
        assert_eq!(goal_of(&request), "add a\nb", "{field:?}");
        assert!(form(&app).submitting, "{field:?}");
        assert!(ctrl(&mut app, 's').is_empty(), "submitting: nothing more");
    }
}

#[test]
fn enter_on_an_option_row_starts() {
    let mut app = app();
    drawn(&mut app);
    open_form(&mut app);
    typed(&mut app, "add a");
    focus(&mut app, GoalField::Trust);
    tap(&mut app, KeyCode::Char(' '));
    let (_, request) = tagged(&tap(&mut app, KeyCode::Enter));
    assert!(matches!(
        request,
        RunRequest::StartGoal {
            trust_project: true,
            ..
        }
    ));
    // A blank goal is refused inline and the focus goes back to the text.
    let mut app = self::app();
    open_form(&mut app);
    focus(&mut app, GoalField::Delivery);
    assert!(tap(&mut app, KeyCode::Enter).is_empty());
    assert_eq!(form(&app).error.as_deref(), Some("type a goal first"));
    assert_eq!(form(&app).focus, GoalField::Goal);
}

#[test]
fn tab_from_the_text_goes_to_the_first_option_and_shift_tab_back() {
    let mut app = app();
    drawn(&mut app);
    open_form(&mut app);
    typed(&mut app, "one");
    tap(&mut app, KeyCode::Enter);
    typed(&mut app, "two");
    // Up and Down move in the text, and stay put on its first and last rows.
    tap(&mut app, KeyCode::Up);
    tap(&mut app, KeyCode::Up);
    assert_eq!(
        (form(&app).focus, form(&app).goal.position()),
        (GoalField::Goal, (1, 4))
    );
    tap(&mut app, KeyCode::Down);
    tap(&mut app, KeyCode::Down);
    assert_eq!(
        (form(&app).focus, form(&app).goal.position()),
        (GoalField::Goal, (2, 4))
    );
    tap(&mut app, KeyCode::Tab);
    assert_eq!(form(&app).focus, GoalField::Runtime);
    tap(&mut app, KeyCode::BackTab);
    assert_eq!(form(&app).focus, GoalField::Goal);
    // Down and Up between the rows; Up from the first option row returns to the text.
    tap(&mut app, KeyCode::Tab);
    tap(&mut app, KeyCode::Down);
    assert_eq!(form(&app).focus, GoalField::Model);
    tap(&mut app, KeyCode::Down);
    assert_eq!(form(&app).focus, GoalField::Orchestrator);
    tap(&mut app, KeyCode::Up);
    tap(&mut app, KeyCode::Up);
    assert_eq!(form(&app).focus, GoalField::Runtime);
    tap(&mut app, KeyCode::Up);
    assert_eq!(form(&app).focus, GoalField::Goal);
    assert_eq!(
        form(&app).goal.text(),
        "one\ntwo",
        "the moves typed nothing"
    );
}

#[test]
fn esc_on_empty_closes() {
    let mut app = app();
    open_form(&mut app);
    assert!(tap(&mut app, KeyCode::Esc).is_empty());
    assert_eq!(app.modal, None);
    assert!(app.goal_drafts.is_empty());
    open_form(&mut app);
    assert!(ctrl(&mut app, 'c').is_empty());
    assert_eq!(app.modal, None);
}

/// The confirm page's rows at 80x24: the large dialog's frame, titled `discard`.
fn page_rows(app: &App) -> Vec<String> {
    let rows = audit::rows(&audit::draw(app, 80, 24));
    (rows[..4].iter())
        .map(|r| r.chars().skip(2).take(76).collect())
        .collect()
}

#[test]
fn esc_on_text_asks_and_y_discards() {
    let mut app = app();
    drawn(&mut app);
    open_form(&mut app);
    typed(&mut app, "add a");
    focus(&mut app, GoalField::Yes);
    assert!(tap(&mut app, KeyCode::Esc).is_empty());
    assert!(form(&app).discarding);
    let pad = |text: &str| format!("│ {text:<72} │");
    assert_eq!(
        page_rows(&app),
        [
            format!("┌ discard {}┐", "─".repeat(65)),
            pad("discard this goal text?"),
            pad(""),
            pad("y discard · any other key back"),
        ]
    );
    // `y` and `discard` in `Failed`, the frame still the one accented frame.
    let buffer = audit::draw(&app, 80, 24);
    let failed = crate::theme::role(crate::theme::Role::Failed, app.palette()).fg;
    let (x, y) = audit::find(&buffer, "y discard")[0];
    assert_eq!(buffer[(x, y)].fg, failed.unwrap());
    assert_eq!(audit::accented_frames(&buffer, app.palette()), 1);
    assert!(tap(&mut app, KeyCode::Char('y')).is_empty());
    assert_eq!(app.modal, None);
    assert!(app.goal_drafts.is_empty(), "discarded, not kept");
}

#[test]
fn any_other_key_goes_back() {
    for key in [
        KeyCode::Char('n'),
        KeyCode::Char('Y'),
        KeyCode::Esc,
        KeyCode::Enter,
    ] {
        let mut app = app();
        open_form(&mut app);
        typed(&mut app, "add a");
        focus(&mut app, GoalField::Trust);
        tap(&mut app, KeyCode::Esc);
        assert!(form(&app).discarding);
        assert!(tap(&mut app, key).is_empty(), "{key:?}");
        let f = form(&app);
        assert!(!f.discarding, "{key:?}");
        assert_eq!((f.focus, f.goal.text()), (GoalField::Goal, "add a"));
        assert!(!f.trust_project, "{key:?} reached no row");
    }
    // Ctrl-C asks as Esc does; Ctrl-Y is not `y`.
    let mut app = app();
    open_form(&mut app);
    typed(&mut app, "add a");
    ctrl(&mut app, 'c');
    assert!(form(&app).discarding);
    ctrl(&mut app, 'y');
    assert!(!form(&app).discarding);
    assert!(app.modal.is_some());
}

/// Ctrl-S, then Esc while it waits: the dialog closes at once, keeping its text.
pub(super) fn send_and_close(app: &mut App) -> u64 {
    let (id, _) = tagged(&ctrl(app, 's'));
    assert!(tap(app, KeyCode::Esc).is_empty());
    assert_eq!(app.modal, None);
    id
}

#[test]
fn a_draft_is_kept_per_project_and_restored_at_its_end() {
    let mut app = app();
    drawn(&mut app);
    open_on(&mut app, "/p/a");
    typed(&mut app, "add a");
    tap(&mut app, KeyCode::Enter);
    typed(&mut app, "then b");
    tap(&mut app, KeyCode::Home);
    send_and_close(&mut app);
    assert_eq!(app.goal_drafts.len(), 1);
    assert_eq!(app.goal_drafts[&PathBuf::from("/p/a")], "add a\nthen b");
    // Another project's dialog starts empty, and its empty close keeps nothing.
    open_on(&mut app, "/p/b");
    assert!(form(&app).goal.is_empty());
    tap(&mut app, KeyCode::Esc);
    assert_eq!(app.goal_drafts.len(), 1);
    // Back on `/p/a`: the draft, the cursor at its end, in the text.
    open_on(&mut app, "/p/a");
    let f = form(&app);
    assert_eq!(f.goal.text(), "add a\nthen b");
    assert_eq!(f.goal.position(), (2, 7));
    assert_eq!(f.focus, GoalField::Goal);
    typed(&mut app, "!");
    assert_eq!(form(&app).goal.text(), "add a\nthen b!");
}

#[test]
fn a_discarded_draft_is_gone() {
    let mut app = app();
    open_on(&mut app, "/p/a");
    typed(&mut app, "add a");
    send_and_close(&mut app);
    open_on(&mut app, "/p/a");
    tap(&mut app, KeyCode::Esc);
    tap(&mut app, KeyCode::Char('y'));
    assert!(app.goal_drafts.is_empty());
    open_on(&mut app, "/p/a");
    assert!(form(&app).goal.is_empty());
}

#[test]
fn a_successful_start_clears_the_draft() {
    // `Triaged` to the open dialog.
    let mut app = app();
    open_on(&mut app, "/p/a");
    typed(&mut app, "add a");
    send_and_close(&mut app);
    open_on(&mut app, "/p/a");
    let (id, _) = tagged(&ctrl(&mut app, 's'));
    app.on_daemon(triaged(id));
    assert_eq!(app.modal, None);
    assert!(app.goal_drafts.is_empty());
    assert_eq!(app.toast_text(), Some("planning run r-new"));

    // `Started` (a continued goal) to the open dialog.
    let mut app = app_with_chains(vec![idle("/p/a", false)], vec![]);
    open_on(&mut app, "/p/a");
    typed(&mut app, "add b");
    send_and_close(&mut app);
    open_on(&mut app, "/p/a");
    let (id, _) = tagged(&ctrl(&mut app, 's'));
    app.on_daemon(started(id));
    assert_eq!(app.modal, None);
    assert!(app.goal_drafts.is_empty());
    assert_eq!(app.toast_text(), Some("run r-next started"));
    let mut next = run_info("r-next");
    next.project = "/p/a".into();
    deliver(&mut app, snapshot(3, 100, vec![next]));
    assert_eq!(
        app.run_view.as_ref().map(|v| v.run_id.as_str()),
        Some("r-next"),
        "the run view opens on the new run"
    );

    // Either reply after the dialog closed while waiting.
    for reply in [triaged as fn(u64) -> DaemonMsg, started] {
        let mut app = self::app();
        open_on(&mut app, "/p/a");
        typed(&mut app, "add c");
        let id = send_and_close(&mut app);
        assert_eq!(app.goal_drafts.len(), 1);
        // Another request's reply is not this one's.
        app.on_daemon(reply(id + 1));
        assert_eq!(app.goal_drafts.len(), 1);
        app.on_daemon(reply(id));
        assert!(app.goal_drafts.is_empty());
        assert!(app.toast_text().is_some());
    }
}

#[test]
fn a_refused_start_keeps_the_draft() {
    // Refused while open: the error row, the text kept, not submitting.
    let mut app = app();
    open_on(&mut app, "/p/a");
    typed(&mut app, "add a");
    let (id, _) = tagged(&ctrl(&mut app, 's'));
    app.on_daemon(refused(id));
    let f = form(&app);
    assert_eq!(
        (f.error.as_deref(), f.goal.text(), f.submitting),
        (Some("not a git repository: /p/a"), "add a", false)
    );
    // Refused after the dialog closed while waiting: the draft stays.
    let id = send_and_close(&mut app);
    app.on_daemon(refused(id));
    assert_eq!(app.goal_drafts[&PathBuf::from("/p/a")], "add a");
    open_on(&mut app, "/p/a");
    assert_eq!(form(&app).goal.text(), "add a");
}

/// The large dialog's rows at 120x40, joined.
pub(super) fn screen(app: &App) -> String {
    audit::rows(&audit::draw(app, 120, 40)).join("\n")
}

#[test]
fn the_orchestrator_row_offers_continue_by_default() {
    let mut app = app_with_chains(vec![idle("/p/a", false)], vec![]);
    open_on(&mut app, "/p/a");
    assert!(form(&app).continuing);
    assert!(screen(&app).contains("orchestrator      ‹ continue o-3f9a (after 3f9a) ›"));
    focus(&mut app, GoalField::Orchestrator);
    tap(&mut app, KeyCode::Char(' '));
    assert!(screen(&app).contains("orchestrator      ‹ new ›"));
    tap(&mut app, KeyCode::Left);
    assert!(form(&app).continuing);
    // The same chain in the next snapshot keeps the choice; the session ending makes
    // it a fresh one.
    tap(&mut app, KeyCode::Right);
    let mut snap = snapshot(3, 100, vec![]);
    snap.idle_orchestrators = vec![idle("/p/a", true)];
    deliver(&mut app, snap);
    assert!(!form(&app).continuing, "the user's `new` holds");
    tap(&mut app, KeyCode::Right);
    assert!(screen(&app).contains("orchestrator      ‹ continue o-3f9a (fresh session) ›"));
    // Another project's dialog offers `new` only.
    let mut app = app_with_chains(vec![idle("/p/a", false)], vec![]);
    open_on(&mut app, "/p/b");
    assert!(!form(&app).continuing);
    focus(&mut app, GoalField::Orchestrator);
    tap(&mut app, KeyCode::Char(' '));
    assert!(!form(&app).continuing);
    assert!(screen(&app).contains("orchestrator      ‹ new ›"));
}

#[test]
fn continue_locks_runtime_and_model() {
    let mut app = app_with_chains(vec![idle("/p/a", false)], vec![]);
    open_on(&mut app, "/p/a");
    focus(&mut app, GoalField::Runtime);
    tap(&mut app, KeyCode::Right);
    tap(&mut app, KeyCode::Char(' '));
    assert_eq!(form(&app).runtime, None, "the chain's runtime holds");
    focus(&mut app, GoalField::Model);
    tap(&mut app, KeyCode::Right);
    assert_eq!(form(&app).model, crate::run_goal::GoalModel::Default);
    let buffer = audit::draw(&app, 120, 40);
    let muted = crate::theme::role(crate::theme::Role::Muted, app.palette()).fg;
    for value in ["‹ claude ›", "‹ claude-opus-5-5 ›"] {
        let (x, y) = audit::find(&buffer, value)[0];
        assert_eq!(buffer[(x + 2, y)].fg, muted.unwrap(), "{value} is muted");
    }
    // `new`: the rows are the user's again.
    focus(&mut app, GoalField::Orchestrator);
    tap(&mut app, KeyCode::Char(' '));
    focus(&mut app, GoalField::Runtime);
    tap(&mut app, KeyCode::Right);
    assert_eq!(form(&app).runtime, Some(Runtime::Claude));
    assert!(screen(&app).contains("runtime           ‹ claude ›"));
    assert!(screen(&app).contains("model             ‹ default ›"));
}

#[test]
fn continue_sends_continue_from_and_no_orchestrator() {
    let mut app = app_with_chains(vec![idle("/p/a", false)], vec![]);
    open_on(&mut app, "/p/a");
    typed(&mut app, "next goal");
    // A runtime chosen under `new`, then continue again: the chain's holds.
    focus(&mut app, GoalField::Orchestrator);
    tap(&mut app, KeyCode::Char(' '));
    focus(&mut app, GoalField::Runtime);
    tap(&mut app, KeyCode::Right);
    focus(&mut app, GoalField::Orchestrator);
    tap(&mut app, KeyCode::Char(' '));
    // `approve at once` is the user's, as for a new goal.
    focus(&mut app, GoalField::Yes);
    tap(&mut app, KeyCode::Char(' '));
    let (_, request) = tagged(&ctrl(&mut app, 's'));
    assert_eq!(
        request,
        RunRequest::StartGoal {
            goal: "next goal".into(),
            dir: "/p/a".into(),
            yes: true,
            trust_project: false,
            unconfined_checks: false,
            orchestrator: None,
            delivery: None,
            continue_from: Some("r-20261001-3f9a".into()),
            design: None,
        }
    );
    // `new` sends the runtime and no `continue_from`.
    let mut app = app_with_chains(vec![idle("/p/a", false)], vec![]);
    open_on(&mut app, "/p/a");
    typed(&mut app, "next goal");
    focus(&mut app, GoalField::Orchestrator);
    tap(&mut app, KeyCode::Right);
    focus(&mut app, GoalField::Runtime);
    tap(&mut app, KeyCode::Right);
    let (_, request) = tagged(&ctrl(&mut app, 's'));
    let RunRequest::StartGoal {
        orchestrator,
        continue_from,
        ..
    } = request
    else {
        panic!()
    };
    assert_eq!(
        (orchestrator, continue_from),
        (
            Some(OrchestratorChoice {
                runtime: Runtime::Claude,
                model: None
            }),
            None
        )
    );
}

#[test]
fn an_active_chain_says_this_goal_gets_a_new_orchestrator() {
    let mut run = run_info("r-20261003-77b2");
    run.project = "/p/a".into();
    run.chain = Some("o-3f9a".into());
    let mut ended = run_info("r-20261001-3f9a");
    ended.project = "/p/a".into();
    ended.chain = Some("o-1111".into());
    ended.state = RunState::Accepted;
    let mut app = app_with_chains(vec![], vec![ended, run]);
    open_on(&mut app, "/p/a");
    assert!(
        screen(&app).contains(
            "orchestrator      o-3f9a is working on run 77b2; this goal gets a new orchestrator"
        ),
        "{}",
        screen(&app)
    );
    // Muted, and not a choice: its keys do nothing, and the goal is a new chain's.
    let buffer = audit::draw(&app, 120, 40);
    let muted = crate::theme::role(crate::theme::Role::Muted, app.palette()).fg;
    let (x, y) = audit::find(&buffer, "o-3f9a is working")[0];
    assert_eq!(buffer[(x, y)].fg, muted.unwrap());
    typed(&mut app, "another goal");
    focus(&mut app, GoalField::Orchestrator);
    tap(&mut app, KeyCode::Char(' '));
    let (_, request) = tagged(&ctrl(&mut app, 's'));
    assert!(matches!(
        request,
        RunRequest::StartGoal {
            continue_from: None,
            orchestrator: None,
            ..
        }
    ));
}

/// Decision 5 through the dialog: PgDn and PgUp move by the large editor's visible
/// rows less one (10 at 80x24, so 9), from the view the app hands the keys.
#[test]
fn pgdn_and_pgup_move_by_the_visible_rows_less_one() {
    let mut app = app();
    drawn(&mut app);
    open_form(&mut app);
    let text: Vec<String> = (1..=30).map(|n| format!("line {n}")).collect();
    app.on_paste(text.join("\n"));
    press(&mut app, KeyCode::Home, KeyModifiers::CONTROL);
    assert_eq!(form(&app).goal.position(), (1, 1));
    tap(&mut app, KeyCode::PageDown);
    assert_eq!(form(&app).goal.position(), (10, 1));
    tap(&mut app, KeyCode::PageDown);
    assert_eq!(form(&app).goal.position(), (19, 1));
    tap(&mut app, KeyCode::PageUp);
    assert_eq!(form(&app).goal.position(), (10, 1));
}

/// Decision 5: a bracketed paste is the editor's: it stops at the cap and says so, and
/// it ends a run of Ctrl-Ks.
#[test]
fn a_paste_goes_to_the_editor() {
    let mut app = app();
    drawn(&mut app);
    open_form(&mut app);
    app.on_paste("y".repeat(proto::GOAL_MAX_CHARS + 5));
    assert_eq!(
        form(&app).goal.text().chars().count(),
        proto::GOAL_MAX_CHARS
    );
    assert!(form(&app).goal.at_cap());
    let rows = audit::rows(&audit::draw(&app, 80, 24)).join("\n");
    assert!(
        rows.contains("goal is at its 16,384-character limit"),
        "{rows}"
    );
}
