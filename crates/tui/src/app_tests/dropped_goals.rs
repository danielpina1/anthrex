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

/// Fix round 1 (review I1): the daemon's drain hides a goal before its start returns,
/// so the first status, asked while the drain runs, holds no drop yet. The wait
/// survives it; the next snapshot change asks again (once the gap has passed) and the
/// drop on that reply raises the alert.
#[test]
fn a_drop_recorded_after_the_first_status_still_raises_the_alert() {
    let (mut app, effects) = left_the_queue();
    let [(first, _)] = status_requests(&effects)[..] else {
        panic!("one status request: {effects:?}");
    };
    // Asked during the drain: no drop yet.
    reply(&mut app, first, status(R, vec![]));
    assert!(alerts(&app).is_empty());
    // The drain ends (the daemon publishes); the gap since the first ask has not
    // passed on the real clock, so the ask waits for a tick past it.
    let effects = deliver(&mut app, snapshot(QUEUED_AT + 20, vec![]));
    let mut asked = status_requests(&effects);
    if asked.is_empty() {
        asked = status_requests(&app.on_tick_at(Instant::now() + Duration::from_secs(2), 0));
    }
    let [(second, ref dir)] = asked[..] else {
        panic!("one second status request: {asked:?}");
    };
    assert_eq!(dir, &PathBuf::from(R));
    reply(
        &mut app,
        second,
        status(R, vec![drop_of("add a", REASON, QUEUED_AT + 15)]),
    );
    assert_eq!(
        listed(&app),
        vec![(
            3,
            "r".to_owned(),
            format!("could not start \"add a\": {REASON}")
        )]
    );
}

/// Fix round 1: a goal whose run appears in a later snapshot is no longer waited for:
/// nothing more is asked, and no alert is raised.
#[test]
fn a_goal_whose_run_appears_later_ends_the_wait() {
    let (mut app, effects) = left_the_queue();
    let [(first, _)] = status_requests(&effects)[..] else {
        panic!("one status request: {effects:?}");
    };
    reply(&mut app, first, status(R, vec![]));
    let effects = deliver(
        &mut app,
        snapshot(QUEUED_AT + 20, vec![started("run-1", R, "add a")]),
    );
    assert!(status_requests(&effects).is_empty(), "{effects:?}");
    let later = Instant::now() + Duration::from_secs(5);
    assert!(status_requests(&app.on_tick_at(later, 0)).is_empty());
    assert!(alerts(&app).is_empty());
}

/// The give-up bound is the CLI's `GOAL_REQUEST_TIMEOUT` (1220 s,
/// `docs/timing-budgets.md`), rebuilt from the daemon's constants.
#[test]
fn the_start_budget_is_a_goal_starts_worst_case() {
    assert_eq!(
        crate::app::alerts::GOAL_START_BUDGET,
        Duration::from_secs(1220)
    );
}

/// Fix round 1: past one start budget per goal up to this one in its queue, the wait
/// ends silently: nothing more is asked, and a drop that arrives later raises nothing.
#[test]
fn the_wait_ends_silently_at_its_bound() {
    let (mut app, effects) = left_the_queue();
    let [(first, _)] = status_requests(&effects)[..] else {
        panic!("one status request: {effects:?}");
    };
    // The goal left at `QUEUED_AT + 10`, first in its queue: one budget.
    let bound = QUEUED_AT + 10 + crate::app::alerts::GOAL_START_BUDGET.as_secs();
    let effects = deliver(&mut app, snapshot(bound - 1, vec![]));
    let later = Instant::now() + Duration::from_secs(5);
    assert!(
        status_requests(&effects).is_empty(),
        "the first ask is still in flight"
    );
    let effects = deliver(&mut app, snapshot(bound, vec![]));
    assert!(status_requests(&effects).is_empty(), "{effects:?}");
    assert!(status_requests(&app.on_tick_at(later, 0)).is_empty());
    // The first ask's reply, late, names the drop: the wait is over, so no alert.
    reply(
        &mut app,
        first,
        status(R, vec![drop_of("add a", REASON, QUEUED_AT + 5)]),
    );
    assert!(alerts(&app).is_empty(), "{:?}", alerts(&app));
}

/// The second goal of a queue waits behind the first one's start: two budgets.
#[test]
fn a_goal_behind_another_waits_two_budgets() {
    let mut app = app_with_runs(
        vec![],
        with_queue(
            QUEUED_AT,
            vec![queued("q-1", R, "add a"), queued("q-2", R, "fix b")],
        ),
    );
    let effects = deliver(&mut app, snapshot(QUEUED_AT + 10, vec![]));
    let [(first, _)] = status_requests(&effects)[..] else {
        panic!("one status request for the directory: {effects:?}");
    };
    reply(&mut app, first, status(R, vec![]));
    let budget = crate::app::alerts::GOAL_START_BUDGET.as_secs();
    let past_one = QUEUED_AT + 10 + budget;
    let effects = deliver(&mut app, snapshot(past_one, vec![]));
    let mut asked = status_requests(&effects);
    if asked.is_empty() {
        asked = status_requests(&app.on_tick_at(Instant::now() + Duration::from_secs(2), 0));
    }
    let [(second, _)] = asked[..] else {
        panic!("the second goal still waits: {asked:?}");
    };
    reply(
        &mut app,
        second,
        status(
            R,
            vec![
                drop_of("add a", REASON, past_one),
                drop_of("fix b", "the brief is empty", past_one),
            ],
        ),
    );
    assert_eq!(
        listed(&app),
        vec![(
            3,
            "r".to_owned(),
            "could not start \"fix b\": the brief is empty".to_owned()
        )],
        "only the goal still waited for is raised"
    );
}

/// The re-ask is rate-limited: none while one is in flight, none within the gap of
/// the last, and only once a snapshot made it due.
#[test]
fn a_reask_waits_for_the_reply_and_the_gap() {
    use crate::app::alerts::ASK_GAP;
    let (mut app, effects) = left_the_queue();
    let base = Instant::now();
    let [(first, _)] = status_requests(&effects)[..] else {
        panic!("one status request: {effects:?}");
    };
    let ask = |app: &mut App, at: Instant| status_requests(&app.on_tick_at(at, 0));
    // In flight: a later snapshot asks nothing.
    assert!(status_requests(&deliver(&mut app, snapshot(QUEUED_AT + 20, vec![]))).is_empty());
    assert!(ask(&mut app, base).is_empty());
    // Answered with no drop: the change is asked once the gap has passed.
    reply(&mut app, first, status(R, vec![]));
    assert!(ask(&mut app, base).is_empty(), "within the gap");
    let second = ask(&mut app, base + ASK_GAP);
    assert_eq!(second.len(), 1, "{second:?}");
    reply(&mut app, second[0].0, status(R, vec![]));
    // Nothing is due until another snapshot.
    assert!(ask(&mut app, base + ASK_GAP * 3).is_empty(), "nothing due");
    assert!(status_requests(&deliver(&mut app, snapshot(QUEUED_AT + 30, vec![]))).is_empty());
    let third = ask(&mut app, base + ASK_GAP * 4);
    assert_eq!(third.len(), 1, "{third:?}");
    // Its reply not back: another snapshot and a tick far past the gap ask nothing.
    assert!(status_requests(&deliver(&mut app, snapshot(QUEUED_AT + 40, vec![]))).is_empty());
    assert!(
        ask(&mut app, base + ASK_GAP * 10).is_empty(),
        "one in flight"
    );
}
