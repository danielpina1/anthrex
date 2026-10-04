//! Milestone 9.0.7 decision 36's `fixtures`, moved out of `ui/audit.rs` (milestone 9.3
//! task 9b, move only): one `App` per key region, built with the audit's helpers.

use super::*;
use crate::tree::run_fixtures::{pair_fixture, race_fixture};
use crate::ui::run_pr_tests;

/// One `App` per key region of decision 1 that exists, each reached by the keys a
/// user presses where it can be. Later tasks add theirs (the plan review's frame, the
/// old dialogs).
pub(crate) fn fixtures() -> Vec<(&'static str, App)> {
    use crate::app::profile_screen::ProfilePage;
    use crate::app::screens::Screen;
    use crate::app::stats::{StatsScreen, StatsState};
    let profile = || crate::ui::profile::tests::app_with_profile(|_, _| {});
    vec![
        ("pane", gate()),
        ("sidebar tree", with(gate(), |a| chord(a, 't'))),
        // Decision 27: rows cut above and below, marked in the accented border.
        (
            "sidebar tree overflowing",
            with(crowded(), |a| {
                chord(a, 't');
                a.tree.sidebar.top = 6;
            }),
        ),
        (
            "sidebar tree, then hidden",
            with(gate(), |a| {
                chord(a, 't');
                chord(a, 's');
            }),
        ),
        ("project overview", with(gate(), |a| chord(a, 'T'))),
        // Decisions 9 and 10: three alerts grow the box under the agents block.
        ("alerts box", crate::ui::alerts::fixture::three_runs()),
        (
            "sidebar tree over the alerts box",
            with(crate::ui::alerts::fixture::three_runs(), |a| chord(a, 't')),
        ),
        ("run view at the gate", run_view(gate())),
        ("run view running", run_view(running())),
        ("run view on a task", on_task(run_view(running()), "t1")),
        (
            "run view racing",
            on_task(run_view(loaded(race_fixture())), "t2"),
        ),
        (
            "run view pairing",
            on_task(run_view(loaded(pair_fixture())), "t3"),
        ),
        (
            "run view on a task in review",
            on_task(run_view(with(running(), in_review_r2)), "t1"),
        ),
        (
            "run view as a list",
            crate::ui::run_list::tests::two_stage_run_selected("t2"),
        ),
        (
            "run view delivering",
            run_pr_tests::pr_view(pr_fixture(), false, run_pr_tests::stage_key(2)),
        ),
        (
            "run view delivering one stage",
            run_pr_tests::pr_view(
                single_pr_fixture(),
                false,
                tree::NodeKey::Run(RUN_ID.into()),
            ),
        ),
        ("conversation", with(gate(), |a| chord(a, 'm'))),
        // Decisions 23–26: `p` in the run view at the gate.
        ("plan review", with(run_view(gate()), |a| tap(a, 'p'))),
        // Decision 11: `C-b a`, then `j`: the Alerts view on the blocked alert.
        (
            "alerts view",
            with(crate::ui::alerts::fixture::three_runs(), |a| {
                chord(a, 'a');
                tap(a, 'j');
            }),
        ),
        (
            "help over the sidebar tree",
            with(gate(), |a| {
                chord(a, 't');
                chord(a, '?');
            }),
        ),
        (
            "confirm over the overview",
            with(gate(), |a| {
                chord(a, 'T');
                chord(a, 'x');
                assert!(
                    matches!(a.modal, Some(Modal::Confirm { .. })),
                    "`C-b x` asks"
                );
            }),
        ),
        (
            "action menu over the run view",
            with(run_view(gate()), |a| tap(a, '.')),
        ),
        (
            "profile with a page",
            with(profile(), |a| {
                crate::ui::profile::tests::screen_mut(a).page = Some(ProfilePage::Reject);
            }),
        ),
        ("profile under the help", with(profile(), |a| chord(a, '?'))),
        (
            "help over the run view",
            with(run_view(gate()), |a| chord(a, '?')),
        ),
        (
            "help over the alerts view",
            with(crate::ui::alerts::fixture::three_runs(), |a| {
                chord(a, 'a');
                chord(a, '?');
            }),
        ),
        (
            "settings",
            crate::ui::settings::tests::opened(false, crate::ui::settings::tests::sample()),
        ),
        // Milestone 9.5 decision 48: the limits with S's refit under its budget.
        (
            "settings limits with a refit",
            with(
                crate::ui::settings::tests::opened(false, crate::ui::settings::tests::sample()),
                |a| {
                    let Some(Screen::Settings(s)) = &mut a.screen else {
                        panic!("the settings screen");
                    };
                    let mut report = crate::ui::stats::tests::tuning_report();
                    report.classes[0].configured = false;
                    report.orchestrator_list = Some("claude/claude-opus-5-5 high".into());
                    s.tuning = Some(Box::new(report));
                    s.section = crate::app::settings_screen::SettingsSection::Limits;
                },
            ),
        ),
        // Decision 35: the old dialogs, each over the screen it opens from.
        ("new agent over the pane", with(gate(), |a| chord(a, 'c'))),
        (
            "remove over the pane",
            with(gate(), |a| {
                chord(a, 'X');
                let Some(Modal::Remove(confirm)) = &mut a.modal else {
                    panic!("`C-b X` asks");
                };
                confirm.branch = Some("anthrex/wt".into());
            }),
        ),
        (
            "force remove over the pane",
            with(gate(), |a| {
                a.modal = Some(Modal::ForceRemove {
                    window_id: a.windows[0].id,
                    name: a.windows[0].name.clone(),
                    message: "worktree /tmp/wt has uncommitted or untracked changes".into(),
                });
            }),
        ),
        ("rename over the pane", with(gate(), |a| chord(a, ','))),
        (
            "config notice over the pane",
            with(gate(), |a| {
                a.report_config_problems(vec!["config.toml: unknown key `x`".into()]);
            }),
        ),
        ("edit form over the run view", edit_form()),
        // Final fix wave M6: the states decision 36's list left out.
        (
            "goal form over the pane",
            with(gate(), |a| {
                chord(a, 'g');
                assert!(matches!(a.modal, Some(Modal::StartGoal(_))), "`C-b g`");
            }),
        ),
        (
            "goal form continuing",
            with(gate(), |a| {
                a.runs.idle_orchestrators = vec![proto::IdleOrchestrator {
                    project: PROJECT.into(),
                    ..crate::ui::goal_editor::tests::idle(false)
                }];
                chord(a, 'g');
                let Some(Modal::StartGoal(form)) = &a.modal else {
                    panic!("`C-b g`");
                };
                assert!(form.continuing, "continue is the default");
            }),
        ),
        (
            "goal form discard page",
            with(gate(), |a| {
                chord(a, 'g');
                tap(a, 'x');
                a.on_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
                let Some(Modal::StartGoal(form)) = &a.modal else {
                    panic!("`C-b g`");
                };
                assert!(form.discarding, "Esc on a text asks");
            }),
        ),
        // Milestone 9.3 decision 32: the menu's `iterate` on a complete run, and a later
        // round's plan review.
        (
            "iterate dialog over the run view",
            with(run_view(iterable()), |a| {
                tap(a, '.');
                a.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
                assert!(matches!(a.modal, Some(Modal::Iterate(_))), "`iterate`");
            }),
        ),
        (
            "plan review of round 2",
            with(run_view(round_two_gate()), |a| tap(a, 'p')),
        ),
        // Milestone 9.3 task 10b: a run of two rounds, and the sidebar's idle
        // orchestrator with its menu.
        (
            "run view with rounds",
            crate::ui::run_list::rounds_tests::rounds_view(false, 80, 24),
        ),
        ("sidebar idle orchestrator", idle_row()),
        (
            "idle menu over the sidebar",
            with(idle_row(), |a| {
                tap(a, '.');
                assert!(matches!(a.modal, Some(Modal::IdleMenu(_))), "`.`");
            }),
        ),
        (
            "action menu on its message form",
            with(crate::ui::alerts::fixture::three_runs(), |a| {
                chord(a, 'a');
                tap(a, 'j');
                tap(a, 'm');
                a.on_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
                let Some(Modal::Action(flow)) = &a.modal else {
                    panic!("the menu is open");
                };
                assert!(
                    matches!(flow.step, crate::app::actions::ActionStep::Form(_)),
                    "Enter on `message` opens its form"
                );
            }),
        ),
        (
            "settings discard page",
            with(
                crate::ui::settings::tests::opened(false, crate::ui::settings::tests::sample()),
                |a| {
                    let Some(Screen::Settings(s)) = &mut a.screen else {
                        panic!("the settings screen");
                    };
                    s.page = Some(crate::app::settings_screen::SettingsPage::Discard);
                },
            ),
        ),
        (
            "profile confirm page",
            with(profile(), |a| {
                crate::ui::profile::tests::screen_mut(a).page = Some(ProfilePage::Confirm {
                    toml: "check = \"cargo test\"".into(),
                    scroll: 0,
                });
            }),
        ),
        (
            "alerts view, empty",
            with(
                app_of(
                    vec![pty(1, "shell", PROJECT, proto::Status::Idle)],
                    crate::tree::run_fixtures::snapshot(1, vec![]),
                ),
                |a| {
                    chord(a, 'a');
                    assert!(a.alerts_focus.is_some(), "`C-b a` opens the view");
                },
            ),
        ),
        (
            "stats",
            with(gate(), |a| {
                a.screen = Some(Screen::Stats(Box::new(StatsScreen {
                    project: "/r/demo".into(),
                    state: StatsState::Ready(Box::new(
                        crate::ui::stats::tests::history_with_tuning(),
                    )),
                    scroll: 0,
                    request: 4,
                })));
            }),
        ),
    ]
}

/// The sidebar tree on `o-3f9a`'s idle row (four runs), its window `7`.
fn idle_row() -> App {
    use crate::app::idle_menu::tests::{app_of, idle_fixture, on_idle_row};
    on_idle_row(app_of(idle_fixture(4), false))
}

/// The gate fixture's run as a `pr` run, complete (every PR landed, so no `ready to
/// accept` alert), every task merged, listing `iterate` first.
fn iterable() -> App {
    let (mut snap, windows) = gate_fixture();
    let run = &mut snap.runs[0];
    run.state = proto::RunState::Complete;
    run.delivery = Some(crate::tree::pr_fixtures::delivery(false));
    for task in &mut run.tasks {
        task.state = proto::TaskState::Merged;
    }
    run.actions = vec![proto::ActionInfo {
        kind: proto::ActionKind::Iterate,
        label: "iterate".into(),
        effect: "plan round 2 of this run with its orchestrator".into(),
        needs: proto::ActionKind::Iterate.needs(),
        destructive: false,
        refused_why: None,
    }];
    app_of(windows, snap)
}

/// The gate fixture as round 2's gate: `t1` merged in round 1, `t2` the round's.
fn round_two_gate() -> App {
    let (mut snap, windows) = gate_fixture();
    let run = &mut snap.runs[0];
    let round = |n, goal_head: &str, outcome| proto::RoundInfo {
        n,
        goal_head: goal_head.into(),
        origin: proto::RoundOrigin::User,
        outcome,
        summary_head: None,
        ended: outcome.is_some(),
    };
    run.round = 2;
    run.rounds = vec![
        round(
            1,
            "Add password reset",
            Some(proto::RoundOutcome::Completed),
        ),
        round(2, "add a reset email", None),
    ];
    run.tasks[0].state = proto::TaskState::Merged;
    run.tasks[1].round = 2;
    app_of(windows, snap)
}
