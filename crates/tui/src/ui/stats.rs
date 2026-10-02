//! Milestone 9.0.6 decision 38: the run-history stats screen, drawn over the body. A
//! frame titled `stats · <project>`, then the records line, the table (one row per size
//! class, a `None` median read `–`), the deciders line, the flaky proposals and the
//! problems, one row each, and last the history file (`history: <path>`, as the CLI
//! ends its first line), scrolled from `scroll` with the kit's marks. Loading shows
//! `loading…`; a refusal or a lost reply shows in `Failed`. A dialog over it mutes its
//! border (decision 5). Every class, test name, problem, path and refusal passes
//! `safe_text`. Pure: `&App` in.

use crate::app::stats::{StatsScreen, StatsState};
use crate::app::{App, region::KeyRegion};
use crate::safe_text::{multi_line, one_line};
use crate::theme::{Palette, Role, dot, ellipsis, role};
use crate::ui::kit::{self, Hint, cut, wrap_words};
use proto::HistoryStats;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Clear, Paragraph};
use unicode_width::UnicodeWidthStr;

/// Interfaces "Stats sections": the table's header, one column each.
const HEADER: [&str; 9] = [
    "class", "tasks", "merged", "lines", "calls", "tokens", "work", "bounces", "reverted",
];

/// A `None` median.
fn none(p: Palette) -> &'static str {
    if p.ascii { "-" } else { "–" }
}

fn hint(key: &str, word: &str, priority: u8) -> Hint {
    Hint {
        key: key.to_string(),
        word: word.to_string(),
        priority,
    }
}

/// `<n> <word>`, with an `s` unless `n` is 1.
fn count(n: u32, word: &str) -> String {
    if n == 1 {
        format!("1 {word}")
    } else {
        format!("{n} {word}s")
    }
}

/// The status bar's hints while the screen has the keys (decision 6).
pub(crate) fn hints() -> Vec<Hint> {
    vec![
        hint("j/k", "scroll", 7),
        hint("PgUp/PgDn", "page", 5),
        hint("esc", "back", 9),
    ]
}

/// `stats · <project dir name>`.
pub(crate) fn title(s: &StatsScreen, p: Palette) -> String {
    let name = s
        .project
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| s.project.display().to_string());
    format!("stats {} {}", dot(p), one_line(&name))
}

/// Tokens as `anthrex run stats` writes them (`daemon/src/run/stats.rs::tokens`, the
/// rule copied, not linked): `<n>`, `<n>.<d>k` under 10k, `<n>k`, or `<n>.<d>M`,
/// rounded down.
pub(crate) fn tokens(n: u64) -> String {
    match n {
        0..1_000 => n.to_string(),
        1_000..10_000 => format!("{}.{}k", n / 1_000, n % 1_000 / 100),
        10_000..1_000_000 => format!("{}k", n / 1_000),
        _ => format!("{}.{}M", n / 1_000_000, n % 1_000_000 / 100_000),
    }
}

/// Work time in minutes, rounded to the nearest minute as `anthrex run stats` does.
pub(crate) fn work(secs: u64) -> String {
    format!("{}m", secs.saturating_add(30) / 60)
}

/// The table's cells, header first, every class sanitised.
fn table(stats: &HistoryStats, p: Palette) -> Vec<Vec<String>> {
    let or = |v: Option<String>| v.unwrap_or_else(|| none(p).to_string());
    let mut out = vec![HEADER.map(str::to_string).to_vec()];
    for r in &stats.rows {
        out.push(vec![
            one_line(&r.class),
            r.tasks.to_string(),
            r.merged.to_string(),
            or(r.median_lines.map(|n| n.to_string())),
            or(r.median_tool_calls.map(|n| n.to_string())),
            or(r.median_tokens.map(tokens)),
            or(r.median_work_secs.map(work)),
            r.bounces.to_string(),
            r.reverted.to_string(),
        ]);
    }
    out
}

/// Columns as wide as their header or widest cell, two spaces apart, left-aligned
/// (as `anthrex run stats` lays them out).
fn table_lines(stats: &HistoryStats, p: Palette) -> Vec<String> {
    let cells = table(stats, p);
    let widths: Vec<usize> = (0..HEADER.len())
        .map(|i| cells.iter().map(|row| row[i].width()).max().unwrap_or(0))
        .collect();
    cells
        .iter()
        .map(|row| {
            let padded: Vec<String> = row
                .iter()
                .zip(&widths)
                .map(|(cell, &w)| format!("{cell}{}", " ".repeat(w - cell.width())))
                .collect();
            padded.join("  ").trim_end().to_string()
        })
        .collect()
}

/// Decision 38's lines while ready, one row each, cut to `width`, then the history
/// file. (`FlakyProposal.last_at` is not drawn: Task 15 notes.)
fn ready_lines(stats: &HistoryStats, width: usize, p: Palette) -> Vec<Line<'static>> {
    let e = ellipsis(p);
    let line = |text: String, style: Style| Line::styled(cut(&text, width, e), style);
    let plain = Style::default();
    let bold = Style::default().add_modifier(Modifier::BOLD);
    let muted = role(Role::Muted, p);
    let d = dot(p);
    let records = format!(
        "{} {d} {}",
        count(stats.task_records, "task record"),
        count(stats.run_records, "run")
    );
    let mut out = vec![line(records, muted), Line::default()];
    for (i, text) in table_lines(stats, p).into_iter().enumerate() {
        out.push(line(text, if i == 0 { bold } else { plain }));
    }
    out.push(Line::default());
    out.push(line(
        format!(
            "deciders {} {d} {} {d} size raised {}/{}",
            count(stats.decider_calls, "call"),
            count(stats.decider_fallbacks, "fallback"),
            stats.size_raised,
            stats.size_checked
        ),
        plain,
    ));
    out.push(Line::default());
    out.push(line(
        format!(
            "flaky proposals ({} days, after {})",
            stats.window_days, stats.quarantine_after
        ),
        bold,
    ));
    let names: Vec<String> = stats
        .flaky_proposals
        .iter()
        .map(|f| one_line(&f.test))
        .collect();
    // The runs column stays in view: a long name is cut first.
    let name_w = names
        .iter()
        .map(|n| n.width())
        .max()
        .unwrap_or(0)
        .min(width.saturating_sub(12).max(1));
    for (name, f) in names.iter().zip(&stats.flaky_proposals) {
        let name = cut(name, name_w, e);
        let pad = " ".repeat(name_w.saturating_sub(name.width()));
        out.push(line(
            format!("  {name}{pad}  {}", count(f.runs, "run")),
            plain,
        ));
    }
    if names.is_empty() {
        out.push(line("  none".into(), muted));
    }
    if !stats.problems.is_empty() {
        out.push(Line::default());
        out.push(line("problems".into(), role(Role::Attention, p)));
        for problem in &stats.problems {
            out.push(line(format!("  {}", one_line(problem)), plain));
        }
    }
    out.push(Line::default());
    let path = format!("history: {}", one_line(&stats.path.display().to_string()));
    out.push(line(path, muted));
    out
}

/// Everything the screen shows under its title, before scrolling.
pub(crate) fn body_lines(app: &App, s: &StatsScreen, width: u16) -> Vec<Line<'static>> {
    let p = app.palette();
    let width = usize::from(width).max(1);
    match &s.state {
        StatsState::Loading(_) => {
            let text = if p.ascii { "loading..." } else { "loading…" };
            vec![Line::styled(text, role(Role::Muted, p))]
        }
        StatsState::Ready(stats) => ready_lines(stats, width, p),
        StatsState::Failed(text) => multi_line(text)
            .lines()
            .flat_map(|l| wrap_words(&one_line(l), width))
            .map(|l| Line::styled(l, role(Role::Failed, p)))
            .collect(),
    }
}

/// The interior of the screen's frame over `body` (the renderer's own block).
fn interior(body: Rect, p: Palette) -> Rect {
    kit::screen_frame("", true, p).inner(body)
}

/// The largest useful `scroll` over `body`, by the renderer's own lines and rows, so
/// the screen's keys stop where the view stops (the plan review's rule).
pub(crate) fn max_scroll(app: &App, s: &StatsScreen, body: Rect) -> usize {
    let inner = interior(body, app.palette());
    let len = body_lines(app, s, inner.width).len();
    kit::last_top(len, usize::from(inner.height))
}

pub fn render(frame: &mut Frame, app: &App, s: &StatsScreen, area: Rect) {
    let p = app.palette();
    // Decision 5: the one accented border is the dialog's while one is open.
    let keys_here = app.key_region() == KeyRegion::Screen;
    let block = kit::screen_frame(&title(s, p), keys_here, p);
    let inner = interior(area, p);
    frame.render_widget(Clear, area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let lines = body_lines(app, s, inner.width);
    let shown = kit::from_top(lines, s.scroll, usize::from(inner.height), p);
    frame.render_widget(Paragraph::new(shown), inner);
}

#[cfg(test)]
#[path = "stats_tests.rs"]
pub(crate) mod tests;
