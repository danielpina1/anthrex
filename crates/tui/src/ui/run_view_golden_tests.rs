//! Milestone 9.6 task 18 (addendum "Non-design runs"): a run without the design flow
//! is drawn exactly as before, its run view at 80×24 (the compact list) and 120×40 (the
//! graph), running and at its plan gate. The rows were captured on `264f3539`, before
//! the task touched the run view.

use super::tests::{app_of, run_view};
use crate::app::App;
use crate::tree::run_fixtures::{RUN_ID, gate_fixture, three_task_fixture};
use crate::ui::audit;

fn rows_at(fixture: (proto::RunsSnapshot, Vec<proto::WindowInfo>), w: u16, h: u16) -> Vec<String> {
    let mut app: App = run_view(app_of(fixture, false), RUN_ID);
    let _ = app.set_terminal_size(w, h);
    audit::rows(&audit::draw(&app, w, h))
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
    "│▾ demo                   ⠋  sh 1││▌◉ orchestrator  1/3                        │",
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
    "│                                ││                                            │",
    "│                                ││╭──────────────────────────────────────────╮│",
    "│                                │││ ◉ add-reset-3f9a  Add password reset     ││",
    "│                                │││ progress  ██████░░░░░░░░░░░░  1/3 merge… ││",
    "│                                │││ path      critical path t0 → t1 · 1 tas… ││",
    "│3 agents                        │││ agents    workers 0/3 · readers 0/3 · c… ││",
    "╰────────────────────────────────╯││ spend     tokens 0 · tool calls 0        ││",
    "╭ Alerts ────────────────────────╮││ gate      plan not approved              ││",
    "│no alerts                       ││╰──────────────────────────────────────────╯│",
    "╰────────────────────────────────╯╰────────────────────────────────────────────╯",
    " RUN  j/k move  h/l tier  ⏎ open  . actions  space fold  f filter: all  esc back",
];

/// Captured before task 18 on `264f3539`.
#[rustfmt::skip]
const GATE_80X24: [&str; 24] = [
    "╭ agents · tree ─────────────────╮╭ run · Add passwo… · 3f9a ─ 0/2 merged · 2h ╮",
    "│▾ demo                   ⚑  sh 1││▌⚑ run 3f9a  0/2                            │",
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
    "╰────────────────────────────────╯││ agents    workers 0/3 · readers 0/3 · c… ││",
    "╭ ⚑ Alerts 1 ────────────────────╮││ spend     tokens 0 · tool calls 0        ││",
    "│⚑ Add password reset · 3f9a     │││ gate      awaiting approval · a approve… ││",
    "│  plan awaits approval · 2 tasks│││                                          ││",
    "│                                ││╰──────────────────────────────────────────╯│",
    "╰─────────────────── C-b a open ─╯╰────────────────────────────────────────────╯",
    " RUN  a approve  x reject  e edit  d remove  p review  . actions  esc back",
];

/// Captured before task 18 on `264f3539`.
#[rustfmt::skip]
const RUNNING_120X40: [&str; 40] = [
    "╭ agents · tree ─────────────────╮╭ run · Add password reset · 3f9a ───────────────────────────────── 1/3 merged · 10m ╮",
    "│▾ demo                   ⠋  sh 1││                                                        ╭────────────────────╮      │",
    "▌├─  ◉ 1 Add passwo… · 3f9a 1/3 ✓││                                                      ┌─┤ ✓ worker #1 claude │      │",
    "│└─▎ ○ 2 shell             sh  0s││                          ╭─────────────────────────╮ │ ╰────────────────────╯      │",
    "│                                ││                        ┌─┤ ✓ t0 proto M ◆          ├─┤                             │",
    "│                                ││                        │ ╰─────────────────────────╯ │ ╭────────────────────╮      │",
    "│                                ││                        │                             └─┤ ✓ review #1 codex  │      │",
    "│                                ││                        │                               ╰────────────────────╯      │",
    "│                                ││╭─────────────────────╮ │                                                           │",
    "│                                ││▌ ◉ orchestrator  1/3 ├─┤ ╭─────────────────────────╮   ╭────────────────────╮      │",
    "│                                ││╰─────────────────────╯ ├─┤ ● t1 spawn M  after t0  ├───┤ ● worker #1 claude │      │",
    "│                                ││                        │ ╰─────────────────────────╯   ╰────────────────────╯      │",
    "│                                ││                        │                                                           │",
    "│                                ││                        │ ╭─────────────────────────╮                               │",
    "│                                ││                        └─┤ ▫ t2 status S  after t0 │                               │",
    "│                                ││                          ╰─────────────────────────╯                               │",
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
    "│                                │││ ◉ add-reset-3f9a  Add password reset                               running · 10m ││",
    "│                                │││ progress  ██████░░░░░░░░░░░░  1/3 merged · 1 working · 1 waiting                 ││",
    "│                                │││ path      critical path t0 → t1 · 1 task left                                    ││",
    "│3 agents                        │││ agents    workers 0/3 · readers 0/3 · claude ok                                  ││",
    "╰────────────────────────────────╯││ spend     tokens 0 · tool calls 0                                                ││",
    "╭ Alerts ────────────────────────╮││ gate      plan not approved                                                      ││",
    "│no alerts                       ││╰──────────────────────────────────────────────────────────────────────────────────╯│",
    "╰────────────────────────────────╯╰────────────────────────────────────────────────────────────────────────────────────╯",
    " RUN  j/k move  h/l tier  ⏎ open  . actions  space fold  f filter: all  / find  esc back",
];

/// Captured before task 18 on `264f3539`.
#[rustfmt::skip]
const GATE_120X40: [&str; 40] = [
    "╭ agents · tree ─────────────────╮╭ run · Add password reset · 3f9a ────────────────────────────────── 0/2 merged · 2h ╮",
    "│▾ demo                   ⚑  sh 1││                      ╭──────────────────────────╮                                  │",
    "▌├─  ⚑   Add passwo… · 3f9a 0/2 ✓││                    ┌─┤ ○ t1 reset token model M │                                  │",
    "│└─▎ ○ 1 shell             sh  0s││╭─────────────────╮ │ ╰──────────────────────────╯                                  │",
    "│                                ││▌ ⚑ run 3f9a  0/2 ├─┤                                                               │",
    "│                                ││╰─────────────────╯ │ ╭──────────────────────────╮                                  │",
    "│                                ││                    └─┤ ○ t2 reset endpoint S    │                                  │",
    "│                                ││                      ╰──────────────────────────╯                                  │",
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
    "│                                │││ ⚑ add-reset-3f9a  Add password reset                   awaiting approval · 2h46m ││",
    "│1 agent                         │││ progress  ░░░░░░░░░░░░░░░░░░  0/2 merged · 2 waiting                             ││",
    "╰────────────────────────────────╯││ agents    workers 0/3 · readers 0/3 · claude ok                                  ││",
    "╭ ⚑ Alerts 1 ────────────────────╮││ spend     tokens 0 · tool calls 0                                                ││",
    "│⚑ Add password reset · 3f9a     │││ gate      awaiting approval · a approve · x reject · e edit · d remove           ││",
    "│  plan awaits approval · 2 tasks│││                                                                                  ││",
    "│                                ││╰──────────────────────────────────────────────────────────────────────────────────╯│",
    "╰─────────────────── C-b a open ─╯╰────────────────────────────────────────────────────────────────────────────────────╯",
    " RUN  a approve  x reject  e edit  d remove  p review  ⏎ open  . actions  f filter: all  esc back",
];
