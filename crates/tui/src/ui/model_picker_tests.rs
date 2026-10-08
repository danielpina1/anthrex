//! Milestone 9.8 task 11 (MR §5.2): the model picker drawn as a kit dialog over the
//! Settings screen, with the brief's widths (label 17, description 38).

use super::age_text;
use crate::app::App;
use crate::app::model_picker::tests::fixture_catalogs;
use crate::app::models_table::tests::{opened_models, row, select_role, spec_roles, tap};
use crate::theme::{Role as Look, role};
use crate::ui::settings::tests::{draw, screen_text};
use crossterm::event::KeyCode;
use proto::Runtime;
use proto::models::{CatalogSource, Role};
use std::time::Duration;

/// The picker's frame: its top border, its interior rows (inside the border and its
/// one column of padding, trailing spaces trimmed), and its bottom border's row.
pub(crate) fn dialog(app: &App, w: u16, h: u16) -> (String, Vec<String>, u16) {
    let buffer = draw(app, w, h);
    let row = |y: u16, from: u16, to: u16| -> String {
        (from..=to).map(|x| buffer[(x, y)].symbol()).collect()
    };
    let y0 = (0..h)
        .find(|&y| row(y, 0, w - 1).contains("┌ choose model"))
        .expect("the picker is drawn");
    let top = row(y0, 0, w - 1);
    let x0 = (0..w).find(|&x| buffer[(x, y0)].symbol() == "┌").unwrap();
    let x1 = (x0..w).find(|&x| buffer[(x, y0)].symbol() == "┐").unwrap();
    let y1 = (y0 + 1..h)
        .find(|&y| buffer[(x0, y)].symbol() == "└")
        .unwrap();
    let inner = (y0 + 1..y1)
        .map(|y| row(y, x0 + 2, x1 - 2).trim_end().to_string())
        .collect();
    let top: String = (top.chars().skip(usize::from(x0)))
        .take(usize::from(x1 - x0) + 1)
        .collect();
    (top.trim_end().to_string(), inner, y0)
}

/// The spec's table, the picker open on `implementer · medium`, the catalogs received
/// two minutes before the last tick.
fn picker_app() -> App {
    let mut app = opened_models(spec_roles(), fixture_catalogs());
    let at = app.ticked_at;
    app.catalogs.received_at = vec![(Runtime::Claude, at), (Runtime::Codex, at)];
    app.ticked_at = at + Duration::from_secs(125);
    select_role(&mut app, Role::ImplementerMedium);
    tap(&mut app, KeyCode::Enter);
    app
}

const FOOTER: &str = " j/k move  ⏎ select  r refresh  esc cancel";

#[test]
fn the_picker_draws_as_the_spec() {
    let app = picker_app();
    let (top, inner, _) = dialog(&app, 120, 40);
    assert!(
        top.starts_with("┌ choose model · implementer · medium ─"),
        "{top}"
    );
    assert!(top.ends_with("─ r refresh · updated 2m ago ┐"), "{top}");
    assert_eq!(top.chars().count(), 74, "the picker's widest");
    assert_eq!(
        inner,
        [
            "  CLAUDE  (claude 2.1.290)",
            "    Haiku 4.5        Fastest for quick tasks               effort —",
            "    Sonnet 5         Best for everyday tasks               low … max",
            "    Opus 5.5         Most capable for complex work         low … max",
            "  CODEX  (codex 0.160.1)",
            "    gpt-6 luna       Fast, low cost                        low … xhigh",
            "▸ ● gpt-6 sol        Balanced (Codex default)              low … max",
            "    gpt-6.1 sol      Frontier                              low … max",
            "  ─────",
            "    custom…   type any model name",
            "",
            FOOTER,
        ]
    );
    // At 80 columns the picker is the kit's 64 wide: the description column shrinks
    // and a description too long for it is cut with `…`.
    let (top, inner, _) = dialog(&app, 80, 24);
    assert_eq!(top.chars().count(), 64, "{top}");
    // Both do not fit beside the title: the key goes (the footer names it).
    assert!(
        top.starts_with("┌ choose model · implementer · medium ─"),
        "{top}"
    );
    assert!(top.ends_with("─ updated 2m ago ┐"), "{top}");
    assert_eq!(
        inner,
        [
            "  CLAUDE  (claude 2.1.290)",
            "    Haiku 4.5        Fastest for quick tasks     effort —",
            "    Sonnet 5         Best for everyday tasks     low … max",
            "    Opus 5.5         Most capable for complex w… low … max",
            "  CODEX  (codex 0.160.1)",
            "    gpt-6 luna       Fast, low cost              low … xhigh",
            "▸ ● gpt-6 sol        Balanced (Codex default)    low … max",
            "    gpt-6.1 sol      Frontier                    low … max",
            "  ─────",
            "    custom…   type any model name",
            "",
            FOOTER,
        ]
    );
    // The selection is in the accent; the screen under the dialog is still drawn.
    let text = screen_text(&app, 80, 24);
    assert!(
        text.starts_with("┌ settings · /cfg/config.toml ─"),
        "{text}"
    );
}

#[test]
fn the_age_and_the_catalogs_source() {
    assert_eq!(age_text(Duration::from_secs(5)), "5s");
    assert_eq!(age_text(Duration::from_secs(125)), "2m");
    assert_eq!(age_text(Duration::from_secs(7300)), "2h");
    let mut app = picker_app();
    app.catalogs.list[0].source = CatalogSource::Cached;
    app.catalogs.list[1].source = CatalogSource::Builtin;
    app.catalogs.received_at.clear();
    let catalogs = app.catalogs.clone();
    let s = crate::ui::settings::tests::screen_mut(&mut app);
    s.models.picker.as_mut().unwrap().refresh(&catalogs);
    let (top, inner, _) = dialog(&app, 120, 40);
    // No catalog received: only the key.
    assert!(top.ends_with("─ r refresh ┐"), "{top}");
    assert_eq!(inner[0], "  CLAUDE  (cached)");
    assert_eq!(inner[4], "  CODEX  (built-in list)");
}

#[test]
fn an_unlisted_current_model_is_drawn_dimmed() {
    let mut roles = spec_roles();
    roles
        .rows
        .insert(Role::Research, row("claude:claude-x", Some("medium"), None));
    let mut app = opened_models(roles, fixture_catalogs());
    select_role(&mut app, Role::Research);
    tap(&mut app, KeyCode::Enter);
    let (top, inner, y0) = dialog(&app, 120, 40);
    assert_eq!(
        inner[4],
        "▸ ● claude-x         not reported                          effort —"
    );
    let buffer = draw(&app, 120, 40);
    // The border, the padding, the marks (4) and the label (17).
    let x = (120 - top.chars().count() as u16) / 2 + 2 + 4 + 17;
    assert_eq!(buffer[(x, y0 + 5)].symbol(), "n");
    assert_eq!(
        buffer[(x, y0 + 5)].fg,
        role(Look::Muted, app.palette()).fg.unwrap(),
        "dimmed"
    );
}

#[test]
fn custom_draws_the_runtime_then_the_name() {
    let mut app = picker_app();
    for _ in 0..3 {
        tap(&mut app, KeyCode::Char('j'));
    }
    tap(&mut app, KeyCode::Enter);
    let (_, inner, _) = dialog(&app, 120, 40);
    assert_eq!(inner[9], "    custom…   runtime claude / ‹ codex ›");
    assert_eq!(inner.last().unwrap(), " ←/→ runtime  ⏎ next  esc back");
    tap(&mut app, KeyCode::Enter);
    for c in "a b".chars() {
        tap(&mut app, KeyCode::Char(c));
    }
    tap(&mut app, KeyCode::Enter);
    let (_, inner, _) = dialog(&app, 120, 40);
    assert_eq!(inner[9], "    custom…   codex: a b█");
    assert!(inner[10].starts_with("    ✗ "), "{inner:?}");
    assert_eq!(inner.last().unwrap(), " ⏎ select  esc back");
}

/// Review focus 3: a catalog's label and description are drawn clean, and a long
/// description is cut inside the dialog.
#[test]
fn a_hostile_label_draws_clean() {
    let mut app = picker_app();
    let long = format!("\u{202e}{}", "d".repeat(499));
    for m in &mut app.catalogs.list[0].models {
        m.label = "\u{1b}[31mOK".into();
        m.description = long.clone();
    }
    let catalogs = app.catalogs.clone();
    let s = crate::ui::settings::tests::screen_mut(&mut app);
    s.models.picker.as_mut().unwrap().refresh(&catalogs);
    for (w, h) in [(80, 24), (120, 40)] {
        let buffer = draw(&app, w, h);
        for y in 0..h {
            for x in 0..w {
                let cell = buffer[(x, y)].symbol();
                assert!(
                    !cell.contains('\u{1b}') && !cell.contains('\u{202e}'),
                    "{w}x{h} ({x},{y})"
                );
            }
        }
        let (top, inner, _) = dialog(&app, w, h);
        let width = top.chars().count() - 4;
        assert!(
            inner[1].starts_with("    [31mOK           ddd"),
            "{inner:?}"
        );
        assert!(inner[1].contains("d… "), "{}", inner[1]);
        assert!(
            inner.iter().all(|l| l.chars().count() <= width),
            "{inner:?}"
        );
    }
}
