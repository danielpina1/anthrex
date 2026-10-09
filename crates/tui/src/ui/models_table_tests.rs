//! Milestone 9.8 task 11 (MR §5.1): the `models` section drawn as the spec's table, on
//! M9.8.10's fixture (the spec's roles, the fixture catalogs).

use crate::app::App;
use crate::app::model_picker::tests::fixture_catalogs;
use crate::app::models_table::RowKey;
use crate::app::models_table::tests::{
    opened_models, repo_requests, row, select, select_role, spec_roles, tap,
};
use crate::theme::{Role as Look, role};
use crate::ui::settings::tests::{draw, rows, screen_mut};
use crossterm::event::KeyCode;
use proto::models::{ModelTable, Role};
use proto::{DaemonMsg, RunReply, SettingsReply};

const SIZES: [(u16, u16); 2] = [(80, 24), (120, 40)];

/// The spec's table open on `everywhere`, `this repo` being `/src/repo`.
fn spec_app(roles: ModelTable) -> App {
    let mut app = opened_models(roles, fixture_catalogs());
    screen_mut(&mut app).models.project = Some("/src/repo".into());
    app
}

#[test]
fn the_table_draws_as_the_spec_at_80x24_and_120x40() {
    let mut app = spec_app(spec_roles());
    select_role(&mut app, Role::ImplementerMedium);
    for (w, h) in SIZES {
        let interior = rows(&app, w, h);
        assert!(
            interior[0].starts_with(" models · scope ‹ everywhere › / this repo (repo)"),
            "{w}x{h}: {interior:?}"
        );
        assert!(interior[0].ends_with("tab: limits"), "{w}x{h}");
        assert_eq!(interior[1], "", "{w}x{h}");
        // Real-CLI manual check fix: the real Claude catalog lists Opus and Haiku by
        // alias, so the built-in ids read as themselves; gate fix B3: a model wider
        // than its 24 columns is cut with `…` so the later columns stay aligned (the
        // fallback cut at 80); `claude-sonnet-5` is Sonnet's resolved model exactly.
        let opus_fallback = if w == 80 {
            "Claude · claude-opus-…"
        } else {
            "Claude · claude-opus-5-5"
        };
        assert_eq!(
            interior[2..14],
            [
                " ROLE                  MODEL                   EFFORT   IF IT STRUGGLES".into(),
                " orchestrator          Claude · claude-opus-5… high     —".into(),
                " planner               Claude · claude-opus-5… high     —".into(),
                " implementer · small   Codex  · gpt-6 luna     low      Codex · gpt-6 sol".into(),
                format!("▸implementer · medium  Codex  · gpt-6 sol      medium   {opus_fallback}"),
                " implementer · hub     Claude · claude-opus-5… high     Codex · gpt-6.1 sol"
                    .into(),
                " test writer           Codex  · gpt-6 sol      medium   —".into(),
                format!(" reviewer              Codex  · gpt-6.1 sol    high     {opus_fallback}"),
                " research              Claude · Sonnet         medium   —".into(),
                " brainstorm            Claude · claude-opus-5-5  +  Codex · gpt-6.1 sol".into(),
                " helpers ▸             Claude · claude-haiku-… —        —".into(),
                String::new(),
            ],
            "{w}x{h}"
        );
        assert_eq!(
            interior.last().unwrap(),
            " ⏎ choose model   e effort   f if-it-struggles   x reset   w save   esc back"
        );
    }
    // Gate fix B3: every role row's effort starts under the header's `EFFORT`.
    let interior = rows(&app, 120, 40);
    let effort_col = (interior[2].chars().collect::<String>().find("EFFORT")).unwrap();
    for r in &interior[3..14] {
        if r.is_empty() || r.contains("brainstorm") {
            continue;
        }
        let chars: Vec<char> = r.chars().collect();
        assert_eq!(chars[effort_col - 1], ' ', "{r:?}");
        assert_ne!(chars[effort_col], ' ', "{r:?}");
    }
    // The longest row fits 80 columns' interior (78) whole.
    let longest = rows(&app, 80, 24).iter().map(|r| r.chars().count()).max();
    assert!(longest <= Some(78), "{longest:?}");
}

#[test]
fn this_repo_scope_draws_overrides_and_inherited_rows() {
    let mut app = spec_app(spec_roles());
    let sent = repo_requests(&tap(&mut app, KeyCode::Right));
    let mut repo = ModelTable::default();
    repo.rows.insert(
        Role::Reviewer,
        row("claude:claude-sonnet-5", Some("high"), None),
    );
    app.on_daemon(DaemonMsg::Run(RunReply::Settings {
        reply: Box::new(SettingsReply::RepoModels {
            project: "/src/repo".into(),
            table: repo,
            path: "/data/repos/repo-1234/models.toml".into(),
            problems: vec![],
        }),
        request_id: Some(sent[0].0),
    }));
    let muted = role(Look::Muted, app.palette()).fg.unwrap();
    for (w, h) in SIZES {
        let interior = rows(&app, w, h);
        assert!(
            interior[0].starts_with(" models · scope everywhere / ‹ this repo (repo) ›"),
            "{w}x{h}: {}",
            interior[0]
        );
        // `▸` (the selection, on orchestrator) wins over `●`; reviewer is overridden.
        assert_eq!(
            interior[9],
            "●reviewer              Claude · Sonnet         high     —"
        );
        let buffer = draw(&app, w, h);
        for (i, line) in interior[3..13].iter().enumerate() {
            let y = 1 + 3 + i as u16;
            if i == 6 {
                assert_ne!(buffer[(2, y)].fg, muted, "{w}x{h} reviewer is not dimmed");
                continue;
            }
            // Inherited: dimmed, and `(everywhere)` closes the row, or the row is cut.
            assert!(
                line.ends_with(" (everywhere)") || (w == 80 && line.ends_with('…')),
                "{w}x{h}: {line}"
            );
            assert_eq!(buffer[(2, y)].fg, muted, "{w}x{h}: {line}");
        }
        if w == 120 {
            assert_eq!(
                interior[5],
                " implementer · small   Codex  · gpt-6 luna     low      Codex · gpt-6 sol (everywhere)"
            );
        }
    }
    assert_eq!(
        rows(&app, 80, 24)[5],
        " implementer · small   Codex  · gpt-6 luna     low      Codex · gpt-6 sol (ev…"
    );
}

#[test]
fn warnings_are_listed_under_the_table() {
    let mut roles = spec_roles();
    roles.rows.get_mut(&Role::ImplementerSmall).unwrap().effort = Some("max".into());
    roles
        .rows
        .insert(Role::Research, row("claude:claude-x", Some("medium"), None));
    let app = spec_app(roles);
    let attention = role(Look::Attention, app.palette()).fg.unwrap();
    for (w, h) in SIZES {
        let interior = rows(&app, w, h);
        assert_eq!(interior[13], "", "{w}x{h}");
        assert_eq!(
            interior[14..16],
            [
                "⚠ implementer · small: effort 'max' not offered",
                "⚠ research: not reported by claude 2.1.290",
            ],
            "{w}x{h}"
        );
        let buffer = draw(&app, w, h);
        assert_eq!(buffer[(1, 1 + 14)].fg, attention, "{w}x{h}");
        assert_eq!(buffer[(1, 1 + 15)].fg, attention, "{w}x{h}");
        assert_eq!(
            interior.last().unwrap(),
            " ⏎ choose model   e effort   f if-it-struggles   x reset   w save   esc back"
        );
    }
}

/// The footer drops its entries from the right until it fits; a narrow table cuts its
/// rows with `…`.
#[test]
fn a_narrow_screen_cuts_rows_and_drops_footer_entries() {
    let app = spec_app(spec_roles());
    let interior = rows(&app, 60, 24);
    assert_eq!(
        interior.last().unwrap(),
        " ⏎ choose model   e effort   f if-it-struggles   x reset"
    );
    assert_eq!(
        interior[5],
        " implementer · small   Codex  · gpt-6 luna     low      C…"
    );
}

/// The six helper kinds open inline and the table scrolls to keep the selection in
/// view on a short screen.
#[test]
fn helpers_open_inline_and_the_table_scrolls() {
    let mut app = spec_app(spec_roles());
    select_role(&mut app, Role::Helpers);
    tap(&mut app, KeyCode::Char(' '));
    tap(&mut app, KeyCode::Char('j'));
    let interior = rows(&app, 120, 40);
    assert_eq!(
        interior[12],
        " helpers ▾             Claude · claude-haiku-… —        —"
    );
    assert_eq!(interior[13], "▸  run name            same as helpers");
    for _ in 0..4 {
        tap(&mut app, KeyCode::Char('j'));
    }
    let short = rows(&app, 80, 16);
    assert!(
        short.iter().any(|r| r.starts_with("▸  ")) && short.iter().any(|r| r.contains("more")),
        "{short:?}"
    );
    assert_eq!(
        short.last().unwrap(),
        " ⏎ choose model   e effort   f if-it-struggles   x reset   w save   esc back"
    );
}

/// Preflight F17: the brainstorm row draws no effort; `e` there still cycles it, and
/// the status line says what it is now.
#[test]
fn e_on_brainstorm_says_its_effort_on_the_status_line() {
    let mut app = spec_app(spec_roles());
    select(&mut app, RowKey::Brainstorm);
    tap(&mut app, KeyCode::Char('e'));
    let text = crate::ui::settings::tests::screen_text(&app, 80, 24);
    let bar = text.lines().last().unwrap();
    assert!(bar.contains("brainstorm effort: max"), "{bar}");
    assert!(
        rows(&app, 80, 24)[11]
            .starts_with("▸brainstorm            Claude · claude-opus-5-5  +  Codex"),
        "no effort column"
    );
    tap(&mut app, KeyCode::Char('e'));
    assert_eq!(app.toast_text(), Some("brainstorm effort: default"));
}
