//! Milestone 9.3 task 10b: the sidebar's idle orchestrator row (decision 32, KG §3.2,
//! §6, §11): one muted row in its window's place, Enter focusing the window, and `.`'s
//! menu, `new goal here` (the goal dialog continuing the chain) and `close` (the kill
//! confirm page).

use crate::app::{App, Effect, Modal, PendingAction};
use crate::settings::UiSettings;
use crate::tree::run_fixtures::{PROJECT, pty, run, run_ref, snapshot};
use crate::tree::{self, NodeKey, RowKind};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use proto::{
    AgentRole, ClientMsg, DaemonMsg, IdleOrchestrator, RunReply, RunState, RunsSnapshot, Runtime,
    Status, WindowInfo,
};

pub(crate) const CHAIN: &str = "o-3f9a";
pub(crate) const AFTER: &str = "add-reset-3f9a";

/// Project `/r/demo`: the shell `1`, and `o-3f9a`'s idle orchestrator in window `7`
/// (`3f9a/orchestrator`, Claude) after the accepted run `add-reset-3f9a`, of `runs` runs.
pub(crate) fn idle_fixture(runs: u32) -> (RunsSnapshot, Vec<WindowInfo>) {
    let mut snap = snapshot(10_000, vec![run(AFTER, PROJECT, RunState::Accepted)]);
    snap.idle_orchestrators = vec![IdleOrchestrator {
        chain: CHAIN.into(),
        project: PROJECT.into(),
        after_run: AFTER.into(),
        outcome: RunState::Accepted,
        runtime: Runtime::Claude,
        model: "claude-opus-5-5".into(),
        window_id: Some(7),
        fresh: false,
        runs,
    }];
    let mut orchestrator = pty(7, "3f9a/orchestrator", PROJECT, Status::Idle);
    orchestrator.runtime = Runtime::Claude;
    orchestrator.run = Some(run_ref(AFTER, None, AgentRole::Orchestrator, 1));
    (
        snap,
        vec![pty(1, "shell", PROJECT, Status::Idle), orchestrator],
    )
}

pub(crate) fn app_of((snap, windows): (RunsSnapshot, Vec<WindowInfo>), ascii: bool) -> App {
    let mut settings = UiSettings::default();
    settings.badges.ascii = ascii;
    let mut app = App::new(windows, "/tmp".into(), settings);
    let _ = app.set_terminal_size(80, 24);
    let _ = app.run_subscription();
    app.on_daemon(DaemonMsg::Run(RunReply::Snapshot(snap)));
    app
}

pub(crate) fn tap(app: &mut App, code: KeyCode) -> Vec<Effect> {
    app.on_key(KeyEvent::new(code, KeyModifiers::NONE))
}

/// `C-b t`, then the idle row selected.
pub(crate) fn on_idle_row(mut app: App) -> App {
    app.on_key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL));
    tap(&mut app, KeyCode::Char('t'));
    let rows = tree::build_from(&app.windows, &app.runs, &app.tree);
    app.tree.select(&rows, NodeKey::Chain(CHAIN.into()));
    assert_eq!(app.tree.selected, Some(NodeKey::Chain(CHAIN.into())));
    app
}

fn keys(app: &App) -> Vec<NodeKey> {
    app.rows().into_iter().map(|row| row.key).collect()
}

#[test]
fn its_window_is_not_listed_twice() {
    let app = app_of(idle_fixture(1), false);
    assert_eq!(
        keys(&app),
        [
            NodeKey::Project(PROJECT.into()),
            NodeKey::Chain(CHAIN.into()),
            NodeKey::Window(1),
        ]
    );
    let rows = app.rows();
    let RowKind::IdleOrchestrator { idle, window } = &rows[1].kind else {
        panic!("the idle row");
    };
    assert_eq!((idle.chain.as_str(), window.id), (CHAIN, 7));
    // Not numbered: the shell keeps `1` (fix round 1: `C-b j`/`k` still reach the idle
    // window, after its project's windows; `C-b <n>` counts the numbered ones only).
    assert_eq!(tree::agent_order(&rows), [1, 7]);
    assert_eq!(tree::numbered_order(&rows), [1]);
    // The project's counts still name the window.
    let RowKind::Project { counts, .. } = &rows[0].kind else {
        panic!("the project row");
    };
    assert_eq!((counts.claude, counts.shell), (1, 1));
}

#[test]
fn an_ended_chain_has_no_row() {
    let ended = |change: fn(&mut IdleOrchestrator)| {
        let (mut snap, windows) = idle_fixture(1);
        change(&mut snap.idle_orchestrators[0]);
        keys(&app_of((snap, windows), false))
    };
    let plain = [
        NodeKey::Project(PROJECT.into()),
        NodeKey::Window(1),
        NodeKey::Window(7),
    ];
    assert_eq!(ended(|idle| idle.fresh = true), plain, "a fresh session");
    // Never window 0, nor a window the list does not hold.
    assert_eq!(ended(|idle| idle.window_id = None), plain, "no window");
    // Window 0 (a chain whose orchestrator never launched), even were one listed.
    let (mut snap, mut windows) = idle_fixture(1);
    windows.push(pty(0, "zero", PROJECT, Status::Idle));
    snap.idle_orchestrators[0].window_id = Some(0);
    let keys0 = keys(&app_of((snap, windows), false));
    assert!(
        !keys0.contains(&NodeKey::Chain(CHAIN.into())),
        "window 0: {keys0:?}"
    );
    assert!(keys0.contains(&NodeKey::Window(7)), "{keys0:?}");
    let (mut snap, mut windows) = idle_fixture(1);
    windows.pop();
    snap.idle_orchestrators[0].window_id = Some(7);
    assert_eq!(
        keys(&app_of((snap, windows), false)),
        [NodeKey::Project(PROJECT.into()), NodeKey::Window(1)],
        "a window not listed"
    );
}

#[test]
fn enter_focuses_its_window() {
    let mut app = on_idle_row(app_of(idle_fixture(1), false));
    let effects = tap(&mut app, KeyCode::Enter);
    assert_eq!(app.focused, Some(7));
    assert!(
        app.tree_input.is_none(),
        "Enter leaves tree mode, as on a window"
    );
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::Send(ClientMsg::Subscribe { window_id: 7, .. }))),
        "{effects:?}"
    );
}

#[test]
fn dot_opens_new_goal_here_and_close() {
    let mut app = on_idle_row(app_of(idle_fixture(1), false));
    assert!(tap(&mut app, KeyCode::Char('.')).is_empty());
    let Some(Modal::IdleMenu(menu)) = &app.modal else {
        panic!("`.` opens the idle menu: {:?}", app.modal);
    };
    assert_eq!(
        (menu.chain.as_str(), menu.window_id, menu.selected),
        (CHAIN, 7, 0)
    );
    assert_eq!(crate::app::idle_menu::ENTRIES, ["new goal here", "close"]);
    tap(&mut app, KeyCode::Char('j'));
    tap(&mut app, KeyCode::Char('j'));
    let Some(Modal::IdleMenu(menu)) = &app.modal else {
        panic!("still open");
    };
    assert_eq!(menu.selected, 1, "the selection stops at `close`");
    tap(&mut app, KeyCode::Esc);
    assert!(app.modal.is_none(), "Esc closes it");
}

#[test]
fn new_goal_here_opens_the_dialog_preset_to_continue() {
    let mut app = on_idle_row(app_of(idle_fixture(2), false));
    tap(&mut app, KeyCode::Char('.'));
    tap(&mut app, KeyCode::Enter);
    let Some(Modal::StartGoal(form)) = &app.modal else {
        panic!("`new goal here` opens the goal dialog: {:?}", app.modal);
    };
    assert_eq!(form.project, std::path::PathBuf::from(PROJECT));
    let continued = form.continues().expect("continue is the default");
    assert_eq!(continued.chain, CHAIN);
    // `C-b g` with the idle row selected opens the same dialog.
    let mut app = on_idle_row(app_of(idle_fixture(2), false));
    app.on_key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL));
    tap(&mut app, KeyCode::Char('g'));
    let Some(Modal::StartGoal(form)) = &app.modal else {
        panic!("`C-b g`");
    };
    assert_eq!(
        form.continues().map(|idle| idle.chain.as_str()),
        Some(CHAIN)
    );
}

#[test]
fn close_asks_the_kill_confirm_page() {
    let mut app = on_idle_row(app_of(idle_fixture(1), false));
    tap(&mut app, KeyCode::Char('.'));
    tap(&mut app, KeyCode::Char('j'));
    tap(&mut app, KeyCode::Enter);
    assert_eq!(
        app.modal,
        Some(Modal::Confirm {
            message: "Kill '3f9a/orchestrator'?".into(),
            action: PendingAction::Kill(7),
        })
    );
    // The existing destructive page: Enter only says so, `y` kills window 7.
    assert!(tap(&mut app, KeyCode::Enter).is_empty());
    assert!(app.modal.is_some(), "a destructive confirm takes only `y`");
    assert_eq!(
        tap(&mut app, KeyCode::Char('y')),
        [Effect::Send(ClientMsg::Kill { window_id: 7 })]
    );
}

/// A chain no longer idle on the same window when an entry is chosen (a next goal
/// adopted it meanwhile) is told, never acted on.
#[test]
fn a_chain_adopted_meanwhile_is_not_acted_on() {
    let mut app = on_idle_row(app_of(idle_fixture(1), false));
    tap(&mut app, KeyCode::Char('.'));
    tap(&mut app, KeyCode::Char('j'));
    let (mut snap, _) = idle_fixture(1);
    snap.revision = 2;
    snap.idle_orchestrators.clear();
    app.on_daemon(DaemonMsg::Run(RunReply::Snapshot(snap)));
    assert!(tap(&mut app, KeyCode::Enter).is_empty());
    assert!(app.modal.is_none(), "no kill confirm");
    let toast = app.toast.as_ref().map(|toast| toast.text.as_str());
    assert_eq!(toast, Some("o-3f9a is no longer idle"));
}

/// D17: a delivered `pr` run (complete, every PR landed) leaves its chain idle while the
/// run stays in the tree; its window is the idle row's, not the run's orchestrator.
pub(crate) fn delivered_fixture() -> (RunsSnapshot, Vec<WindowInfo>) {
    let (mut snap, windows) = idle_fixture(1);
    let run = &mut snap.runs[0];
    run.state = RunState::Complete;
    run.delivery = Some(crate::tree::pr_fixtures::delivery(false));
    snap.idle_orchestrators[0].outcome = RunState::Complete;
    (snap, windows)
}

#[test]
fn a_delivered_runs_window_is_its_idle_row_only() {
    let app = app_of(delivered_fixture(), false);
    let rows = app.rows();
    let kinds: Vec<_> = rows.iter().map(|row| row.key.clone()).collect();
    assert_eq!(
        kinds,
        [
            NodeKey::Project(PROJECT.into()),
            NodeKey::Run(AFTER.into()),
            NodeKey::Chain(CHAIN.into()),
            NodeKey::Window(1),
        ]
    );
    let RowKind::Run { orchestrator, .. } = &rows[1].kind else {
        panic!("the run row");
    };
    assert_eq!(*orchestrator, None, "window 7 is the idle row's");
    let inspection = crate::inspector::inspect(&rows[2], &app);
    let after = &inspection.fields[0];
    assert_eq!(
        (after.label, after.value.as_str()),
        ("after", "run add-reset-3f9a · delivered")
    );
}

fn chord(app: &mut App, c: char) -> Vec<Effect> {
    app.on_key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL));
    tap(app, KeyCode::Char(c))
}

/// Fix round 1 (ruling on concern 3): `C-b j`/`k` cycle through the idle row's window,
/// after its project's windows; `C-b <n>` still counts only the numbered windows.
#[test]
fn ctrl_b_j_and_k_cycle_through_the_idle_window() {
    let (snap, mut windows) = idle_fixture(1);
    windows.push(pty(2, "shell-2", PROJECT, Status::Idle));
    let mut app = app_of((snap, windows), false);
    assert_eq!(tree::agent_order(&app.rows()), [1, 2, 7]);
    let _ = app.focus(2);
    chord(&mut app, 'j');
    assert_eq!(app.focused, Some(7), "j from the last numbered window");
    chord(&mut app, 'k');
    assert_eq!(app.focused, Some(2), "k back from the idle window");
    chord(&mut app, 'j');
    chord(&mut app, 'j');
    assert_eq!(app.focused, Some(1), "j from the idle window wraps");
    chord(&mut app, 'k');
    assert_eq!(app.focused, Some(7), "k from the first wraps to it");
    chord(&mut app, '2');
    assert_eq!(
        app.focused,
        Some(2),
        "`C-b 2` is the second numbered window"
    );
    chord(&mut app, '3');
    assert_eq!(
        app.focused,
        Some(2),
        "`C-b 3` names no window: two are numbered"
    );
}

/// Fix round 1, m1: with the idle window focused, `C-b t` selects its row.
#[test]
fn ctrl_b_t_selects_the_focused_idle_row() {
    let mut app = on_idle_row(app_of(idle_fixture(1), false));
    tap(&mut app, KeyCode::Enter);
    assert_eq!(app.focused, Some(7));
    chord(&mut app, 't');
    assert_eq!(app.tree.selected, Some(NodeKey::Chain(CHAIN.into())));
}

/// Fix round 1, m2 (D17): a chain idle while `Complete` is a delivered `pr` run's, even
/// once the snapshot no longer lists the run.
#[test]
fn a_complete_outcome_reads_delivered_without_its_run() {
    let (mut snap, _) = delivered_fixture();
    snap.runs.clear();
    assert_eq!(
        tree::idle_outcome(&snap.idle_orchestrators[0], &snap.runs),
        "delivered"
    );
    let (snap, _) = idle_fixture(1);
    assert_eq!(
        tree::idle_outcome(&snap.idle_orchestrators[0], &snap.runs),
        "accepted"
    );
}

/// Fix round 1, m3: the `no longer idle` toast names a carried chain id cleaned.
#[test]
fn the_no_longer_idle_toast_is_sanitised() {
    let (mut snap, windows) = idle_fixture(1);
    snap.idle_orchestrators[0].chain = "o-3f\u{200D}9\u{202E}a".into();
    let mut app = app_of((snap, windows), false);
    app.on_key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL));
    tap(&mut app, KeyCode::Char('t'));
    let rows = tree::build_from(&app.windows, &app.runs, &app.tree);
    app.tree
        .select(&rows, NodeKey::Chain("o-3f\u{200D}9\u{202E}a".into()));
    tap(&mut app, KeyCode::Char('.'));
    let (mut snap, _) = idle_fixture(1);
    snap.revision = 2;
    snap.idle_orchestrators.clear();
    app.on_daemon(DaemonMsg::Run(RunReply::Snapshot(snap)));
    tap(&mut app, KeyCode::Enter);
    let toast = app.toast.as_ref().map(|toast| toast.text.as_str());
    assert_eq!(toast, Some("o-3f9a is no longer idle"));
}

/// Fix round 1: with a second project after the idle one, the idle window comes after
/// its own project's windows, and the numbers (`C-b <n>`, a sub-agent's `spawned by`)
/// count past it as the rows show them.
#[test]
fn an_idle_window_takes_no_number_from_a_later_project() {
    let (snap, mut windows) = idle_fixture(1);
    let mut other = pty(3, "zeta-shell", "/r/zeta", Status::Idle);
    other.subagents = vec![proto::SubagentInfo {
        id: "s1".into(),
        parent_id: None,
        kind: "Explore".into(),
        label: None,
        model: None,
        state: proto::SubagentState::Running,
        tool: None,
        started_secs: 0,
        ended_secs: None,
        needs_permission: false,
    }];
    windows.push(other);
    let mut app = app_of((snap, windows), false);
    let rows = app.rows();
    assert_eq!(tree::agent_order(&rows), [1, 7, 3]);
    assert_eq!(tree::numbered_order(&rows), [1, 3]);
    let sub = (rows.iter())
        .find(|row| matches!(row.kind, RowKind::Subagent { .. }))
        .expect("the sub-agent's row");
    let spawned = crate::inspector::inspect(sub, &app);
    let by = (spawned.fields.iter())
        .find(|field| field.label == "spawned by")
        .map(|field| field.value.clone());
    assert_eq!(by.as_deref(), Some("2 zeta-shell"), "{:?}", spawned.fields);
    drop(rows);
    chord(&mut app, '2');
    assert_eq!(
        app.focused,
        Some(3),
        "`C-b 2` is the second numbered window"
    );
}
