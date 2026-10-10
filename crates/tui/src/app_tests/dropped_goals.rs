//! Milestone 9.11 decision 22 (A7): a queued goal that leaves the queue with no run for
//! it asks for its project's status; a drop recorded there for the same goal text, at
//! or after the goal was queued, raises one alert for the project, the user's, for this
//! client session only. Enter opens the Profile screen and clears it.

use super::alerts::listed;
use super::runs::{app_with_runs, deliver};
use super::*;
use crate::app::alerts_route::{RouteKind, route_kind};
use crate::app::{AlertKey, alerts};
use crate::tree::run_fixtures::{run, snapshot};
use proto::{
    DroppedGoal, ProfileReply, ProfileRequest, ProfileSource, ProfileStatus, QueuedGoalInfo,
    RunReply, RunRequest, RunState, RunsSnapshot, SetupState,
};
use std::path::PathBuf;

const R: &str = "/r";
const S: &str = "/s";
const QUEUED_AT: u64 = 1_000;
const REASON: &str = "base branch main is gone";

fn queued(id: &str, project: &str, goal: &str) -> QueuedGoalInfo {
    QueuedGoalInfo {
        id: id.into(),
        project: project.into(),
        goal: goal.into(),
        queued_at: QUEUED_AT,
        yes: false,
        trust_project: false,
        unconfined_checks: false,
        setup: SetupState::Reading,
    }
}

fn with_queue(now: u64, goals: Vec<QueuedGoalInfo>) -> RunsSnapshot {
    let mut snap = snapshot(now, vec![]);
    snap.queued_goals = goals;
    snap
}

/// A run started for `goal` in `project`.
fn started(id: &str, project: &str, goal: &str) -> proto::RunInfo {
    let mut info = run(id, project, RunState::Planning);
    info.goal = goal.into();
    info
}

/// The `Status` requests among `effects`, by request id and directory.
fn status_requests(effects: &[Effect]) -> Vec<(u64, PathBuf)> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Send(ClientMsg::RunTagged {
                id,
                request: RunRequest::Profile(ProfileRequest::Status { dir }),
            }) => Some((*id, dir.clone())),
            _ => None,
        })
        .collect()
}

fn status(project: &str, dropped: Vec<DroppedGoal>) -> ProfileReply {
    ProfileReply::Status(ProfileStatus {
        project: project.into(),
        repo_dir: "/data/repos/r".into(),
        source: ProfileSource::Stored,
        confirmed_at: Some(0),
        stale: vec![],
        unparseable: None,
        proposal: None,
        scout: None,
        verify_confined: true,
        queued: Vec::new(),
        checking: None,
        verified_at: None,
        unreadable_text: None,
        dropped_goals: dropped,
    })
}

fn drop_of(goal: &str, reason: &str, at: u64) -> DroppedGoal {
    DroppedGoal {
        goal: goal.into(),
        reason: reason.into(),
        at,
    }
}

fn reply(app: &mut App, id: u64, reply: ProfileReply) -> Vec<Effect> {
    app.on_daemon(DaemonMsg::Run(RunReply::Profile {
        reply: Box::new(reply),
        request_id: Some(id),
    }))
}

/// Queues `"add a"` in `/r`, then a snapshot without it and without a run for it.
fn left_the_queue() -> (App, Vec<Effect>) {
    let mut app = app_with_runs(
        vec![],
        with_queue(QUEUED_AT, vec![queued("q-1", R, "add a")]),
    );
    let effects = deliver(&mut app, snapshot(QUEUED_AT + 10, vec![]));
    (app, effects)
}

/// The alert raised for `"add a"` in `/r`, dropped 5 s after it was queued.
fn raised() -> App {
    let (mut app, effects) = left_the_queue();
    let [(id, _)] = status_requests(&effects)[..] else {
        panic!("one status request: {effects:?}");
    };
    reply(
        &mut app,
        id,
        status(R, vec![drop_of("add a", REASON, QUEUED_AT + 5)]),
    );
    app
}

#[test]
fn a_goal_that_leaves_the_queue_without_a_run_asks_for_the_status() {
    let (_, effects) = left_the_queue();
    let requests = status_requests(&effects);
    assert_eq!(requests.len(), 1, "{effects:?}");
    assert_eq!(requests[0].1, PathBuf::from(R));
}

#[test]
fn a_drop_in_the_status_raises_the_alert() {
    let app = raised();
    assert_eq!(
        listed(&app),
        vec![(
            3,
            "r".to_owned(),
            format!("could not start \"add a\": {REASON}")
        )]
    );
    let alert = &alerts(&app)[0];
    assert_eq!(alert.key, AlertKey::Dropped(R.into()));
    assert!(!alert.you);
}

#[test]
fn a_goal_that_started_raises_nothing() {
    let mut app = app_with_runs(
        vec![],
        with_queue(QUEUED_AT, vec![queued("q-1", R, "add a")]),
    );
    let effects = deliver(
        &mut app,
        snapshot(QUEUED_AT + 10, vec![started("run-1", R, "add a")]),
    );
    assert!(status_requests(&effects).is_empty(), "{effects:?}");
}

/// Review focus 5: the same goal text queued in two repositories; it started in `/s`
/// only, so only `/r` is asked about. And a run in `/r` under a different text does
/// not count as the goal's.
#[test]
fn the_match_is_by_project_and_by_text() {
    let mut app = app_with_runs(
        vec![],
        with_queue(
            QUEUED_AT,
            vec![queued("q-1", R, "add a"), queued("q-2", S, "add a")],
        ),
    );
    let effects = deliver(
        &mut app,
        snapshot(
            QUEUED_AT + 10,
            vec![started("run-1", S, "add a"), started("run-2", R, "add b")],
        ),
    );
    let requests = status_requests(&effects);
    assert_eq!(
        requests
            .iter()
            .map(|(_, dir)| dir.clone())
            .collect::<Vec<_>>(),
        vec![PathBuf::from(R)]
    );
    // `/s`'s status, should one arrive, names a drop of the same text: it raises nothing,
    // since `/s`'s goal started.
    reply(
        &mut app,
        requests[0].0,
        status(R, vec![drop_of("add a", REASON, QUEUED_AT + 5)]),
    );
    let keys: Vec<AlertKey> = alerts(&app).into_iter().map(|a| a.key).collect();
    assert_eq!(keys, vec![AlertKey::Dropped(R.into())]);
}

#[test]
fn a_goal_still_queued_asks_nothing() {
    let mut app = app_with_runs(
        vec![],
        with_queue(QUEUED_AT, vec![queued("q-1", R, "add a")]),
    );
    let effects = deliver(
        &mut app,
        with_queue(QUEUED_AT + 10, vec![queued("q-1", R, "add a")]),
    );
    assert!(status_requests(&effects).is_empty(), "{effects:?}");
}

#[test]
fn an_older_drop_of_the_same_text_is_ignored() {
    let (mut app, effects) = left_the_queue();
    let [(id, _)] = status_requests(&effects)[..] else {
        panic!("one status request: {effects:?}");
    };
    reply(
        &mut app,
        id,
        status(R, vec![drop_of("add a", REASON, QUEUED_AT - 1)]),
    );
    assert!(alerts(&app).is_empty(), "{:?}", alerts(&app));
}

#[test]
fn a_long_goal_is_cut_to_sixty_characters_and_made_one_line() {
    let goal = format!("{}\u{202E}\n{}", "g".repeat(30), "h".repeat(40));
    let mut app = app_with_runs(vec![], with_queue(QUEUED_AT, vec![queued("q-1", R, &goal)]));
    let effects = deliver(&mut app, snapshot(QUEUED_AT + 10, vec![]));
    let [(id, _)] = status_requests(&effects)[..] else {
        panic!("one status request: {effects:?}");
    };
    reply(
        &mut app,
        id,
        status(R, vec![drop_of(&goal, "gone\x1b[31m", QUEUED_AT)]),
    );
    let text = alerts(&app)[0].text.clone();
    assert!(!text.contains('\u{202E}') && !text.contains('\n') && !text.contains('\x1b'));
    let quoted = text
        .strip_prefix("could not start \"")
        .and_then(|rest| rest.split_once('"'))
        .map(|(goal, _)| goal.to_owned())
        .expect("the quoted goal");
    assert_eq!(
        quoted.chars().count(),
        61,
        "60 characters and the ellipsis: {quoted}"
    );
    assert!(quoted.ends_with('…'));
}

#[test]
fn enter_opens_the_profile_and_clears_it() {
    let mut app = raised();
    prefix(&mut app);
    press(&mut app, KeyCode::Char('a'), KeyModifiers::NONE);
    assert_eq!(
        app.alerts_focus.as_ref().and_then(|f| f.selected.clone()),
        Some(AlertKey::Dropped(R.into()))
    );
    let effects = press(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    assert!(
        matches!(&app.screen, Some(crate::app::screens::Screen::Profile(s)) if s.dir == std::path::Path::new(R)),
        "the Profile screen is open on the project"
    );
    assert!(!effects.is_empty(), "the screen fetches its views");
    assert!(alerts(&app).is_empty(), "Enter cleared the alert");
}

#[test]
fn only_enter_clears_it() {
    let mut app = raised();
    // Later snapshots, and a later status with nothing new, leave it.
    deliver(&mut app, snapshot(QUEUED_AT + 20, vec![]));
    deliver(&mut app, snapshot(QUEUED_AT + 30, vec![]));
    assert_eq!(alerts(&app).len(), 1);
}

#[test]
fn the_alert_is_the_users() {
    assert_eq!(
        route_kind(&AlertKey::Dropped(R.into()), None),
        RouteKind::Proposal
    );
    assert_eq!(
        route_kind(&AlertKey::Setup(R.into()), None),
        RouteKind::Proposal,
        "as the set-up alert"
    );
}
