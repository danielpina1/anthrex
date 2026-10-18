//! Milestone 9.6 task 18 (addendum "Non-design runs"): a run without the design flow
//! is drawn exactly as before, its run view at 80×24 (the compact list) and 120×40 (the
//! graph), running and at its plan gate. The rows were captured on `264f3539`, before
//! the task touched the run view; milestone 9.19 decision 5 then gave the run box its
//! narrow `1/3 ✓`, two columns wider, and moved the boxes after it; its 9.19
//! follow-up 1 wrapped the 80×24 spend row at a separator, a row taller.

use super::tests::{app_of, run_view};
use crate::app::App;
use crate::tree::run_fixtures::{RUN_ID, gate_fixture, three_task_fixture};
use crate::ui::audit;

fn rows_at(fixture: (proto::RunsSnapshot, Vec<proto::WindowInfo>), w: u16, h: u16) -> Vec<String> {
    let mut app: App = run_view(app_of(fixture, false), RUN_ID);
    let _ = app.set_terminal_size(w, h);
    audit::rows(&audit::draw(&app, w, h))
}

/// 9.19 follow-up 1: at 80×24 the spend row wraps at its ` · ` separators, a whole part
/// to a row, so the cache share and the worker tool calls stay readable; at 120×40 a
/// row that fits is decision 2's one row.
#[test]
fn the_spend_row_keeps_its_parts_readable() {
    let (mut snapshot, windows) = crate::tree::run_fixtures::gemini_fixture();
    let run = &mut snapshot.runs[0];
    // 75 billed, 71 of 100 input tokens read from cache, 12 tool calls.
    run.usage.as_mut().expect("usage").total = proto::TokenUsage {
        input: 29,
        output: 46,
        cache_read: 71,
        cache_write: 0,
    };
    for (i, task) in run.tasks.iter_mut().enumerate() {
        task.spent_total.tool_calls = if i == 0 { 12 } else { 0 };
    }
    let rows_of = |w: u16, h: u16| {
        let mut app: App = run_view(app_of((snapshot.clone(), windows.clone()), false), "r1");
        let _ = app.set_terminal_size(w, h);
        audit::rows(&audit::draw(&app, w, h))
    };
    let small = rows_of(80, 24);
    let at = small
        .iter()
        .position(|r| r.contains("│ spend     "))
        .unwrap_or_else(|| panic!("{small:#?}"));
    let parts = [
        "│ spend     tokens 75 (all roles) ·",
        "│           cache hits 71% of input ·",
        "│           worker tool calls 12",
    ];
    for (i, part) in parts.iter().enumerate() {
        let row = &small[at + i];
        let end = row.find(part).map(|x| &row[x + part.len()..]);
        assert!(
            end.is_some_and(|e| e.starts_with(' ') && !e.contains('…')),
            "{part:?} in {row:?}\n{small:#?}"
        );
    }
    let large = rows_of(120, 40);
    let full =
        "│ spend     tokens 75 (all roles) · cache hits 71% of input · worker tool calls 12 │";
    assert!(large.iter().any(|r| r.contains(full)), "{large:#?}");
}

#[test]
fn a_run_without_the_flow_is_drawn_as_before() {
    assert_eq!(rows_at(three_task_fixture(), 80, 24), RUNNING_80X24);
    assert_eq!(rows_at(gate_fixture(), 80, 24), GATE_80X24);
    assert_eq!(rows_at(three_task_fixture(), 120, 40), RUNNING_120X40);
    assert_eq!(rows_at(gate_fixture(), 120, 40), GATE_120X40);
}

/// Captured before task 18 on `264f3539`.
#[rustfmt::skip]
const RUNNING_80X24: [&str; 24] = [
    "╭ agents · tree ─────────────────╮╭ run · Add passw… · 3f9a ─ 1/3 merged · 10m ╮",
    "│▾ demo                   ⠋  sh 1││▌◉ orchestrator  1/3 ✓                      │",
    "▌├─  ◉ 1 Add passwo… · 3f9a 1/3 ✓││  ✓ t0 proto   M tdd  merged                │",
    "│└─▎ ○ 2 shell             sh  0s││    ✓ worker #1 claude                      │",
    "│                                ││    ✓ review #1 codex                       │",
    "│                                ││  ● t1 spawn   M tdd  working  after t0     │",
    "│                                ││    ● worker #1 claude                      │",
    "│                                ││  ▫ t2 status  S tdd  queued   after t0     │",
    "│                                ││                                            │",
    "│                                ││                                            │",
    "│                                ││                                            │",
    "│                                ││                                            │",
    "│                                ││                                            │",
    "│                                ││╭──────────────────────────────────────────╮│",
    "│                                │││ ◉ add-reset-3f9a  Add password reset     ││",
    "│                                │││ progress  ██████░░░░░░░░░░░░  1/3 merge… ││",
    "│                                │││ path      critical path t0 → t1 · 1 tas… ││",
    "│                                │││ agents    writers 0/3 · readers 0/3 · c… ││",
    "│3 agents                        │││ spend     tokens unknown (tasks only) ·  ││",
    "╰────────────────────────────────╯││           worker tool calls 0            ││",
    "╭ Alerts ────────────────────────╮││ gate      plan not approved              ││",
    "│no alerts                       ││╰──────────────────────────────────────────╯│",
    "╰────────────────────────────────╯╰────────────────────────────────────────────╯",
    " RUN  j/k move  h/l tier  ⏎ open  . actions  space fold  f filter: all  esc back",
];

/// Captured before task 18 on `264f3539`.
#[rustfmt::skip]
const GATE_80X24: [&str; 24] = [
    "╭ agents · tree ─────────────────╮╭ run · Add passwo… · 3f9a ─ 0/2 merged · 2h ╮",
    "│▾ demo                   ⚑  sh 1││▌⚑ run 3f9a  0/2 ✓                          │",
    "▌├─  ⚑   Add passwo… · 3f9a 0/2 ✓││  ○ t1 reset token model  M tdd  planned    │",
    "│└─▎ ○ 1 shell             sh  0s││  ○ t2 reset endpoint     S tdd  planned    │",
    "│                                ││                                            │",
    "│                                ││                                            │",
    "│                                ││                                            │",
    "│                                ││                                            │",
    "│                                ││                                            │",
    "│                                ││                                            │",
    "│                                ││                                            │",
    "│                                ││                                            │",
    "│                                ││                                            │",
    "│                                ││                                            │",
    "│                                ││╭──────────────────────────────────────────╮│",
    "│                                │││ ⚑ add-reset-3f9a  Add password reset     ││",
    "│1 agent                         │││ progress  ░░░░░░░░░░░░░░░░░░  0/2 merge… ││",
    "╰────────────────────────────────╯││ agents    writers 0/3 · readers 0/3 · c… ││",
    "╭ ⚑ Alerts 1 ────────────────────╮││ spend     tokens unknown (tasks only) ·  ││",
    "│⚑ Add password reset · 3f9a     │││           worker tool calls 0            ││",
    "│  plan awaits approval · 2 tasks│││ gate      awaiting approval · a approve… ││",
    "│                                ││╰──────────────────────────────────────────╯│",
    "╰─────────────────── C-b a open ─╯╰────────────────────────────────────────────╯",
    " RUN  a approve  x reject  e edit  d remove  p review  . actions  esc back",
];

/// Captured before task 18 on `264f3539`.
#[rustfmt::skip]
const RUNNING_120X40: [&str; 40] = [
    "╭ agents · tree ─────────────────╮╭ run · Add password reset · 3f9a ───────────────────────────────── 1/3 merged · 10m ╮",
    "│▾ demo                   ⠋  sh 1││                                                          ╭────────────────────╮    │",
    "▌├─  ◉ 1 Add passwo… · 3f9a 1/3 ✓││                                                        ┌─┤ ✓ worker #1 claude │    │",
    "│└─▎ ○ 2 shell             sh  0s││                            ╭─────────────────────────╮ │ ╰────────────────────╯    │",
    "│                                ││                          ┌─┤ ✓ t0 proto M ◆          ├─┤                           │",
    "│                                ││                          │ ╰─────────────────────────╯ │ ╭────────────────────╮    │",
    "│                                ││                          │                             └─┤ ✓ review #1 codex  │    │",
    "│                                ││                          │                               ╰────────────────────╯    │",
    "│                                ││╭───────────────────────╮ │                                                         │",
    "│                                ││▌ ◉ orchestrator  1/3 ✓ ├─┤ ╭─────────────────────────╮   ╭────────────────────╮    │",
    "│                                ││╰───────────────────────╯ ├─┤ ● t1 spawn M  after t0  ├───┤ ● worker #1 claude │    │",
    "│                                ││                          │ ╰─────────────────────────╯   ╰────────────────────╯    │",
    "│                                ││                          │                                                         │",
    "│                                ││                          │ ╭─────────────────────────╮                             │",
    "│                                ││                          └─┤ ▫ t2 status S  after t0 │                             │",
    "│                                ││                            ╰─────────────────────────╯                             │",
    "│                                ││                                                                                    │",
    "│                                ││                                                                                    │",
    "│                                ││                                                                                    │",
    "│                                ││                                                                                    │",
    "│                                ││                                                                                    │",
    "│                                ││                                                                                    │",
    "│                                ││                                                                                    │",
    "│                                ││                                                                                    │",
    "│                                ││                                                                                    │",
    "│                                ││                                                                                    │",
    "│                                ││                                                                                    │",
    "│                                ││                                                                                    │",
    "│                                ││                                                                                    │",
    "│                                ││                                                                                    │",
    "│                                ││╭──────────────────────────────────────────────────────────────────────────────────╮│",
    "│                                │││ ◉ add-reset-3f9a  Add password reset                           running · 10m 00s ││",
    "│                                │││ progress  ██████░░░░░░░░░░░░  1/3 merged · 1 working · 1 waiting                 ││",
    "│                                │││ path      critical path t0 → t1 · 1 task left                                    ││",
    "│3 agents                        │││ agents    writers 0/3 · readers 0/3 · claude ok                                  ││",
    "╰────────────────────────────────╯││ spend     tokens unknown (tasks only) · worker tool calls 0                      ││",
    "╭ Alerts ────────────────────────╮││ gate      plan not approved                                                      ││",
    "│no alerts                       ││╰──────────────────────────────────────────────────────────────────────────────────╯│",
    "╰────────────────────────────────╯╰────────────────────────────────────────────────────────────────────────────────────╯",
    " RUN  j/k move  h/l tier  ⏎ open  . actions  space fold  f filter: all  / find  esc back",
];

/// Captured before task 18 on `264f3539`.
#[rustfmt::skip]
const GATE_120X40: [&str; 40] = [
    "╭ agents · tree ─────────────────╮╭ run · Add password reset · 3f9a ────────────────────────────────── 0/2 merged · 2h ╮",
    "│▾ demo                   ⚑  sh 1││                        ╭──────────────────────────╮                                │",
    "▌├─  ⚑   Add passwo… · 3f9a 0/2 ✓││                      ┌─┤ ○ t1 reset token model M │                                │",
    "│└─▎ ○ 1 shell             sh  0s││╭───────────────────╮ │ ╰──────────────────────────╯                                │",
    "│                                ││▌ ⚑ run 3f9a  0/2 ✓ ├─┤                                                             │",
    "│                                ││╰───────────────────╯ │ ╭──────────────────────────╮                                │",
    "│                                ││                      └─┤ ○ t2 reset endpoint S    │                                │",
    "│                                ││                        ╰──────────────────────────╯                                │",
    "│                                ││                                                                                    │",
    "│                                ││                                                                                    │",
    "│                                ││                                                                                    │",
    "│                                ││                                                                                    │",
    "│                                ││                                                                                    │",
    "│                                ││                                                                                    │",
    "│                                ││                                                                                    │",
    "│                                ││                                                                                    │",
    "│                                ││                                                                                    │",
    "│                                ││                                                                                    │",
    "│                                ││                                                                                    │",
    "│                                ││                                                                                    │",
    "│                                ││                                                                                    │",
    "│                                ││                                                                                    │",
    "│                                ││                                                                                    │",
    "│                                ││                                                                                    │",
    "│                                ││                                                                                    │",
    "│                                ││                                                                                    │",
    "│                                ││                                                                                    │",
    "│                                ││                                                                                    │",
    "│                                ││                                                                                    │",
    "│                                ││                                                                                    │",
    "│                                ││╭──────────────────────────────────────────────────────────────────────────────────╮│",
    "│                                │││ ⚑ add-reset-3f9a  Add password reset                  awaiting approval · 2h 46m ││",
    "│1 agent                         │││ progress  ░░░░░░░░░░░░░░░░░░  0/2 merged · 2 waiting                             ││",
    "╰────────────────────────────────╯││ agents    writers 0/3 · readers 0/3 · claude ok                                  ││",
    "╭ ⚑ Alerts 1 ────────────────────╮││ spend     tokens unknown (tasks only) · worker tool calls 0                      ││",
    "│⚑ Add password reset · 3f9a     │││ gate      awaiting approval · a approve · x reject · e edit · d remove           ││",
    "│  plan awaits approval · 2 tasks│││                                                                                  ││",
    "│                                ││╰──────────────────────────────────────────────────────────────────────────────────╯│",
    "╰─────────────────── C-b a open ─╯╰────────────────────────────────────────────────────────────────────────────────────╯",
    " RUN  a approve  x reject  e edit  d remove  p review  ⏎ open  . actions  f filter: all  esc back",
];
