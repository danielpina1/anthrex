//! Milestone 9.2 task M9.2.15: a `pr`-mode run in the inspector (decisions 36 and 42).
//! Each test compares the exact `(label, value)` list, the name and the right text.

use super::run_tests::{app_of, inspect_node, pairs, value};
use crate::app::App;
use crate::tree::NodeKey;
use crate::tree::pr_fixtures::{check, pr_fixture, single_pr_fixture};
use crate::tree::run_fixtures::RUN_ID;
use crate::tree::stage_fixtures::staged_fixture;
use proto::{CiState, RunState};
use ratatui::backend::TestBackend;
use ratatui::{Terminal, layout::Rect};

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

/// Review finding m1: the remote and the repo are the host's text too (the daemon's
/// `remote_problem` refuses ASCII controls only, so bidi and ZWJ pass), cleaned in the
/// run's `delivery` row.
#[test]
fn the_delivery_row_cleans_the_remote_and_the_repo() {
    let (mut snap, windows) = pr_fixture();
    let d = snap.runs[0].delivery.as_mut().expect("pr mode");
    d.remote = "or\u{200D}ig\u{202E}in".into();
    d.repo = "fa\u{2067}ke/a\u{FEFF}pp\u{2069}".into();
    let app = app_of((snap, windows));
    let run = inspect_node(&app, &run_key());
    assert_eq!(
        value(&run, "delivery"),
        Some("pr to origin (fake/app) · watching every 60 s")
    );
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

fn task_key(id: &str) -> NodeKey {
    NodeKey::Task {
        run: RUN_ID.into(),
        id: id.into(),
    }
}

/// OUTCOME's `(label, value)` rows of `id`.
fn outcome(app: &App, id: &str) -> Vec<(&'static str, String)> {
    let inspection = inspect_node(app, &task_key(id));
    let section = (inspection.sections.iter())
        .find(|s| s.title == "OUTCOME")
        .expect("OUTCOME");
    (section.fields.iter())
        .map(|f| (f.label, f.value.clone()))
        .collect()
}

fn outcome_value(app: &App, id: &str, label: &str) -> Option<String> {
    (outcome(app, id).into_iter())
        .find(|(l, _)| *l == label)
        .map(|(_, v)| v)
}

/// Ruling R-13: a `pr` run's task panel names its stage's PR with its state, after the
/// pipeline and the check, and what a fix task fixes; a local run's panel has neither.
#[test]
fn the_task_panel_names_its_pr_and_what_it_fixes() {
    let app = app_of(pr_fixture());
    let labels: Vec<&str> = outcome(&app, "fix3").iter().map(|(l, _)| *l).collect();
    assert_eq!(labels[..4], ["pipeline", "check", "pr", "fixes"]);
    for (id, pr, fixes) in [
        ("t1", "#141 merged", None),
        ("t2", "#142 open · ci red", None),
        ("fix3", "#142 open · ci red", Some("CI run 77")),
        ("fix4", "#142 open · ci red", Some("thread by @alice")),
        ("t3", "#143 open · ci pending", None),
    ] {
        assert_eq!(outcome_value(&app, id, "pr").as_deref(), Some(pr), "{id}");
        assert_eq!(outcome_value(&app, id, "fixes").as_deref(), fixes, "{id}");
    }
    // Before its PR opens, a skipped stage, a closed PR.
    let (mut snap, windows) = pr_fixture();
    snap.runs[0].stages[2].pr = None;
    snap.runs[0].stages[0].pr.as_mut().expect("#141").state = proto::PrState::Closed;
    let app = app_of((snap.clone(), windows.clone()));
    assert_eq!(
        outcome_value(&app, "t3", "pr").as_deref(),
        Some("no PR yet")
    );
    assert_eq!(
        outcome_value(&app, "t1", "pr").as_deref(),
        Some("#141 closed")
    );
    if let Some(d) = snap.runs[0].delivery.as_mut() {
        d.skipped_stages = vec![3];
    }
    let app = app_of((snap, windows));
    assert_eq!(outcome_value(&app, "t3", "pr").as_deref(), Some("skipped"));
    // A local run: neither row, whatever the task fixes.
    let (mut snap, windows) = pr_fixture();
    snap.runs[0].delivery = None;
    let app = app_of((snap, windows));
    assert_eq!(outcome_value(&app, "fix3", "pr"), None);
    assert_eq!(outcome_value(&app, "fix3", "fixes"), None);
}

/// The hostile-text rule: `fixes` is the engine's text built from the host's (a CI
/// run's or a thread author's name), cleaned where it is shown, and drawn without a
/// hidden character at 80×24 in ASCII.
#[test]
fn the_task_panels_fixes_is_cleaned() {
    let (mut snap, windows) = pr_fixture();
    let fix3 = (snap.runs[0].tasks.iter_mut())
        .find(|t| t.id == "fix3")
        .expect("fix3");
    fix3.fixes = Some("thread by @al\u{200D}ice\u{202E}\nnext\u{1b}[2J".into());
    let app = app_of((snap.clone(), windows.clone()));
    assert_eq!(
        outcome_value(&app, "fix3", "fixes").as_deref(),
        Some("thread by @alice next [2J")
    );
    let app = crate::ui::run_pr_tests::pr_view((snap, windows), true, task_key("fix3"));
    let rows = crate::ui::audit::rows(&crate::ui::audit::draw(&app, 80, 24));
    let text = rows.join("\n");
    assert!(text.contains("thread by @alice next [2J"), "{text}");
    assert!(text.contains("#142 open - ci red"), "{text}");
    for row in &rows {
        assert!(row.is_ascii(), "{row:?}");
        assert_eq!(crate::safe_text::tests::first_hostile(row), None, "{row:?}");
    }
}

/// Deferred from task 15: the `ci` row's marks are coloured from each check's state,
/// by position, never by reading the text, so a check named `✓` stays default.
#[test]
fn the_ci_row_marks_each_check_by_its_state_never_its_text() {
    use crate::theme::Role;
    let (mut snap, windows) = pr_fixture();
    let two = snap.runs[0].stages[1].pr.as_mut().expect("#142");
    two.checks = vec![
        check("✓", CiState::Red, None),
        check("build", CiState::Green, None),
    ];
    let ci = |app: &App| {
        let inspection = inspect_node(app, &stage_key(2));
        let field = inspection.fields.iter().find(|f| f.label == "ci").cloned();
        field.expect("a ci row")
    };
    let row = ci(&app_of((snap.clone(), windows.clone())));
    assert_eq!(row.value, "✗ ✓ · ✓ build");
    assert_eq!(row.marks, [(0, Role::Failed), (3, Role::Done)]);
    // ASCII: a name whose fold is more words (`☐` is `[ ]`) does not move a mark.
    let two = snap.runs[0].stages[1].pr.as_mut().expect("#142");
    two.checks[0].name = "☐ ☐ +".into();
    let row = ci(&ascii(app_of((snap.clone(), windows.clone()))));
    assert_eq!(row.value, "x [ ] [ ] + - + build");
    assert_eq!(row.marks, [(0, Role::Failed), (7, Role::Done)]);

    // Drawn: each mark in its role's colour, the names (`✓` included) default.
    let two = snap.runs[0].stages[1].pr.as_mut().expect("#142");
    two.checks[0].name = "✓".into();
    let app = app_of((snap, windows));
    let inspection = inspect_node(&app, &stage_key(2));
    let p = crate::theme::Palette::PLAIN;
    let (width, height) = (60, 12);
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| {
            crate::inspector::panel::render_in(
                frame,
                &inspection,
                Rect::new(0, 0, width, height),
                p,
            )
        })
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    let line = (0..height)
        .map(|y| {
            (0..width)
                .map(|x| buffer[(x, y)].symbol().to_owned())
                .collect::<String>()
        })
        .position(|row| row.contains("✗ ✓ · ✓ build"))
        .expect("the ci row is drawn");
    let row: String = (0..width)
        .map(|x| buffer[(x, line as u16)].symbol().to_owned())
        .collect();
    let at = |nth: usize, symbol: &str| {
        let x = row
            .match_indices(symbol)
            .nth(nth)
            .map(|(i, _)| row[..i].chars().count());
        buffer[(x.expect(symbol) as u16, line as u16)].fg
    };
    let fg = |role| crate::theme::role(role, p).fg.unwrap();
    assert_eq!(at(0, "✗"), fg(Role::Failed));
    assert_eq!(
        at(0, "✓"),
        ratatui::style::Color::Reset,
        "the check named ✓"
    );
    assert_eq!(at(1, "✓"), fg(Role::Done));
    assert_eq!(at(0, "b"), ratatui::style::Color::Reset);
}
