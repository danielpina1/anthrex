//! Milestone 9.2 task M9.2.15: a `pr`-mode run in the inspector (decisions 36 and 42).
//! Each test compares the exact `(label, value)` list, the name and the right text.

use super::run_tests::{app_of, inspect_node, pairs, value};
use crate::app::App;
use crate::tree::NodeKey;
use crate::tree::pr_fixtures::{check, pr_fixture, single_pr_fixture};
use crate::tree::run_fixtures::RUN_ID;
use crate::tree::stage_fixtures::staged_fixture;
use proto::{CiState, RunState};

fn stage_key(n: u16) -> NodeKey {
    NodeKey::Stage {
        run: RUN_ID.into(),
        n,
    }
}

fn run_key() -> NodeKey {
    NodeKey::Run(RUN_ID.into())
}

fn ascii(mut app: App) -> App {
    app.settings.badges.ascii = true;
    app
}

#[test]
fn stage_inspector_lines_are_exact() {
    let app = app_of(pr_fixture());
    let two = inspect_node(&app, &stage_key(2));
    assert_eq!(two.name, "stage 2/3");
    assert_eq!(
        pairs(&two),
        [
            ("branch", "anthrex/add-reset-3f9a/stage-2 at 2222222"),
            ("tier 3", "green · 38s"),
            (
                "pr",
                "#142 open, based on anthrex/add-reset-3f9a/stage-1, opened 02:30"
            ),
            ("url", "https://github.com/fake/app/pull/142"),
            ("ci", "✓ build · ✗ test (fix3)"),
            ("threads", "2 open, 1 addressed, 1 replied, 4 ignored"),
            ("fix tasks", "ci: fix3 working; review: fix4 merged"),
        ]
    );
    // A merged PR says when; its checks are gone, so CI is the summary's word.
    let one = inspect_node(&app, &stage_key(1));
    assert_eq!(
        value(&one, "pr"),
        Some("#141 merged, based on main, opened 02:30, merged 03:00")
    );
    assert_eq!(value(&one, "ci"), Some("green"));
    assert_eq!(value(&one, "threads"), Some("none"));
    assert_eq!(value(&one, "fix tasks"), None);
    let three = inspect_node(&app, &stage_key(3));
    assert_eq!(value(&three, "ci"), Some("pending"));
    // The checks' marks have their ASCII twins.
    let two = inspect_node(&ascii(app_of(pr_fixture())), &stage_key(2));
    assert_eq!(value(&two, "ci"), Some("+ build - x test (fix3)"));
}

/// The CI line's every state, a paused PR and a closed one, and a fix task whose
/// state the snapshot spells `merge_queue`.
#[test]
fn stage_inspector_shows_each_ci_state_and_a_paused_or_closed_pr() {
    let (mut snap, windows) = pr_fixture();
    let two = snap.runs[0].stages[1].pr.as_mut().expect("#142");
    two.checks = vec![
        check("lint", CiState::Pending, None),
        check("docs", CiState::None, None),
    ];
    two.paused = true;
    two.fix_tasks = vec!["fix5 review merge_queue".into(), "odd".into()];
    let three = snap.runs[0].stages[2].pr.as_mut().expect("#143");
    three.state = proto::PrState::Closed;
    three.ci = CiState::None;
    let app = app_of((snap, windows));
    let two = inspect_node(&app, &stage_key(2));
    assert_eq!(value(&two, "ci"), Some("… lint · – docs"));
    assert_eq!(
        value(&two, "pr"),
        Some("#142 open, based on anthrex/add-reset-3f9a/stage-1, opened 02:30 · paused")
    );
    assert_eq!(
        value(&two, "fix tasks"),
        Some("review: fix5 merge queue; odd")
    );
    let three = inspect_node(&app, &stage_key(3));
    assert_eq!(
        value(&three, "pr"),
        Some("#143 closed, based on anthrex/add-reset-3f9a/stage-2, opened 02:30")
    );
    assert_eq!(value(&three, "ci"), Some("no checks"));
}

/// The hostile-text rule: a check's name, the base, the URL and a fix task's text come
/// from the host or an agent, so every one is cleaned. Exact text, with zero-width,
/// bidi and control characters planted where they would be drawn.
#[test]
fn stage_inspector_cleans_host_text() {
    let (mut snap, windows) = pr_fixture();
    let two = snap.runs[0].stages[1].pr.as_mut().expect("#142");
    two.checks = vec![check(
        "te\u{200D}st\u{202E}x",
        CiState::Red,
        Some("fi\u{2066}x3"),
    )];
    two.base = "ma\u{200B}in\u{1b}[2J".into();
    two.url = "https://e\u{FEFF}x.com/\u{202D}1".into();
    two.fix_tasks = vec!["fix3 c\u{200D}i work\u{2069}ing".into()];
    let app = app_of((snap, windows));
    let two = inspect_node(&app, &stage_key(2));
    assert_eq!(value(&two, "ci"), Some("✗ testx (fix3)"));
    assert_eq!(
        value(&two, "pr"),
        Some("#142 open, based on main [2J, opened 02:30")
    );
    assert_eq!(value(&two, "url"), Some("https://ex.com/1"));
    assert_eq!(value(&two, "fix tasks"), Some("ci: fix3 working"));
}

#[test]
fn single_stage_run_shows_its_pr_in_the_run_inspector() {
    let app = app_of(single_pr_fixture());
    let run = inspect_node(&app, &run_key());
    let shown: Vec<(&str, &str)> = pairs(&run)
        .into_iter()
        .skip_while(|(label, _)| *label != "delivery")
        .collect();
    assert_eq!(
        shown,
        [
            ("delivery", "pr to origin (fake/app) · watching every 60 s"),
            ("pr", "#7 open, based on main, opened 02:30"),
            ("url", "https://github.com/fake/app/pull/7"),
            ("ci", "✓ build"),
            ("threads", "none"),
        ]
    );
    // A multi-stage run's root shows the delivery only: its stages show their PRs.
    let (mut snap, windows) = pr_fixture();
    if let Some(d) = snap.runs[0].delivery.as_mut() {
        d.watching = false;
    }
    let app = app_of((snap, windows));
    let run = inspect_node(&app, &run_key());
    assert_eq!(
        value(&run, "delivery"),
        Some("pr to origin (fake/app) · not watching")
    );
    assert_eq!(value(&run, "pr"), None);
}

#[test]
fn run_header_says_delivering() {
    let right = |app: &App| inspect_node(app, &run_key()).right.unwrap_or_default();
    let app = app_of(pr_fixture());
    assert!(
        right(&app).starts_with("running (delivering) · "),
        "{}",
        right(&app)
    );
    // Not every stage has a PR yet: running, not delivering.
    let (mut snap, windows) = pr_fixture();
    if let Some(d) = snap.runs[0].delivery.as_mut() {
        d.delivering = false;
    }
    let app = app_of((snap, windows));
    assert!(right(&app).starts_with("running · "), "{}", right(&app));
    // A paused run says where from, as before.
    let (mut snap, windows) = pr_fixture();
    snap.runs[0].state = RunState::Paused;
    snap.runs[0].paused_from = Some(RunState::Running);
    let app = app_of((snap, windows));
    assert!(
        right(&app).starts_with("paused (from running) · "),
        "{}",
        right(&app)
    );
}

/// Pinning (decision 42): a local run has no delivery row, no PR lines and 9.1's fix
/// tasks line; 9.1's and 9.0.7's own render tests run unchanged beside this.
#[test]
fn local_run_renders_as_before() {
    let app = app_of(staged_fixture());
    let run = inspect_node(&app, &run_key());
    assert_eq!(value(&run, "delivery"), None);
    assert!(run.right.unwrap_or_default().starts_with("running · "));
    let one = inspect_node(&app, &stage_key(1));
    assert_eq!(
        pairs(&one),
        [
            ("branch", "anthrex/add-reset-3f9a/stage-1 at 1111111"),
            (
                "tier 3",
                "bisecting at 2222222 · 41m12s · 2 shards · flaky a::flaky"
            ),
            ("failing", "a::works"),
            ("bisect", "running · 1 fix task"),
            ("fix tasks", "fix1 (bisect of t2)"),
        ]
    );
}
