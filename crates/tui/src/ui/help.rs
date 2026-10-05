//! Milestone 9.0.7 decision 34: the help, grouped by context and scrolled by its top
//! line (`app::help::HelpView`). Every row names a key its context's handler takes:
//! the prefix commands (`keymap.rs`), the tree (`tree_input.rs`), the run view
//! (`app/runs.rs::on_run_view_key`), the plan review, the Alerts view, the conversation
//! view, the action menu and its forms, and the Profile and Settings screens.

use crate::app::App;
use crate::app::help::HelpView;
use crate::safe_text::one_line;
use crate::theme::{Palette, Role, fold, role};
use crate::ui::kit::{self, Hint};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use unicode_width::UnicodeWidthStr;

/// One group: its name and its `(key, words)` rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HelpGroup {
    pub name: &'static str,
    pub rows: Vec<(String, String)>,
}

/// The help's title (decision 34).
pub const TITLE: &str = "keys";
/// The rows sit this far in from their group's name.
const INDENT: usize = 2;
/// Between the key column and the words.
const GAP: usize = 2;

fn rows(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|(k, w)| (k.to_string(), w.to_string()))
        .collect()
}

/// The nine groups in the spec's order, each with its context's keys; every key that
/// names the prefix takes it from `prefix_label` (milestone 6.9 decision 38).
pub fn help_groups(prefix_label: &str) -> Vec<HelpGroup> {
    let p = prefix_label;
    let global = [
        ("j / k", "next / previous agent"),
        ("1-9", "focus agent by number"),
        ("c", "new agent"),
        (",", "rename agent"),
        ("R", "restart agent"),
        ("x", "kill agent"),
        ("X", "remove agent (and worktree)"),
        ("m", "conversation"),
        ("t", "tree"),
        ("T", "overview"),
        ("g", "start a goal"),
        ("a", "alerts"),
        ("P", "profile"),
        ("S", "settings"),
        ("s", "toggle sidebar"),
        ("< / >", "sidebar width"),
        ("r", "reconnect"),
        ("d", "detach (agents keep running)"),
        ("Q", "stop daemon and all agents"),
    ];
    let mut global: Vec<(String, String)> = global
        .iter()
        .map(|(k, w)| (format!("{p} {k}"), w.to_string()))
        .collect();
    global.push((format!("{p} {p}"), format!("send a literal {p}")));
    global.push((format!("{p} ?"), "this help".to_string()));
    vec![
        HelpGroup {
            name: "global",
            rows: global,
        },
        HelpGroup {
            name: "sidebar",
            rows: rows(&[
                ("j / k", "move"),
                ("h / l", "parent / child"),
                ("⏎", "focus, or open a run"),
                ("space", "fold"),
                ("/", "filter"),
                (".", "actions on the selected run, stage or task"),
                ("esc", "back"),
            ]),
        },
        HelpGroup {
            name: "run view",
            rows: rows(&[
                ("j / k", "move"),
                ("h / l", "tier"),
                ("⏎", "open"),
                (".", "actions on the selected run, stage or task"),
                ("space", "fold"),
                ("f", "filter"),
                ("/", "find"),
                ("i", "show / hide the inspector"),
                ("PgUp / PgDn", "scroll the panel"),
                ("b", "the whole brief"),
                ("p", "review the plan"),
                ("a / x", "approve / reject (gate, hold)"),
                ("e / d", "edit / remove a task (gate)"),
                ("s", "submit (planning)"),
                ("esc", "back"),
            ]),
        },
        HelpGroup {
            name: "plan review",
            rows: rows(&[
                ("j / k", "task"),
                ("PgUp / PgDn", "scroll"),
                ("a / x", "approve / reject"),
                ("e / d", "edit / drop a task (gate)"),
                ("tab", "the plan / the tasks (design run)"),
                ("c", "ask for changes (design run)"),
                ("b", "back to the spec (design run)"),
                ("esc", "back"),
            ]),
        },
        HelpGroup {
            name: "alerts",
            rows: rows(&[
                ("j / k", "move"),
                ("⏎", "the alert's action"),
                (".", "all actions"),
                ("m", "message the task"),
                ("o", "open the task or run"),
                ("PgUp / PgDn", "scroll the detail"),
                ("esc", "back"),
            ]),
        },
        HelpGroup {
            name: "conversation",
            rows: rows(&[
                ("j / k", "move"),
                ("g / G", "first / last"),
                ("⏎ / o", "fold a tool, open a sub-agent"),
                ("/", "search"),
                ("n / N", "next / previous hit"),
                ("esc", "back"),
                ("q", "close"),
            ]),
        },
        HelpGroup {
            name: "action menu",
            rows: rows(&[
                ("j / k", "move"),
                ("⏎", "choose"),
                ("tab", "a message's kind, or the next choice"),
                ("^J", "new line"),
                ("y", "confirm"),
                ("esc", "back"),
            ]),
        },
        HelpGroup {
            name: "profile",
            rows: rows(&[
                ("tab", "status / profile tab"),
                ("j / k", "move"),
                ("d", "detect"),
                ("x", "reject the proposal"),
                ("s", "store once verification passes"),
                ("e", "edit"),
                ("u", "unset"),
                ("p", "stored / proposal"),
                ("c", "confirm the proposal"),
                ("⏎", "a check's output"),
                ("esc", "back"),
            ]),
        },
        HelpGroup {
            name: "settings",
            rows: rows(&[
                ("tab", "section"),
                ("j / k", "move"),
                ("space", "toggle a model"),
                ("← / →", "strength, or the orchestrator's choice"),
                ("⏎", "add a custom model"),
                ("0-9", "edit a limit"),
                ("w", "save"),
                ("esc", "back (asks before discarding changes)"),
            ]),
        },
    ]
}

/// The line each group's name is drawn on: the groups' names and rows, in order.
pub(crate) fn group_tops(groups: &[HelpGroup]) -> Vec<usize> {
    let mut at = 0;
    groups
        .iter()
        .map(|g| {
            let top = at;
            at += 1 + g.rows.len();
            top
        })
        .collect()
}

/// Every line of the help, the groups' names bold and an aligned key column in the
/// accent; words cut to `width`. Folded and sanitised (decisions 5, Global
/// Constraint 4).
fn lines(groups: &[HelpGroup], width: usize, p: Palette) -> Vec<Line<'static>> {
    let clean = |s: &str| fold(&one_line(s), p.ascii);
    let key_w = groups
        .iter()
        .flat_map(|g| &g.rows)
        .map(|(k, _)| clean(k).width())
        .max()
        .unwrap_or(0);
    let room = width.saturating_sub(INDENT + key_w + GAP);
    let ellipsis = crate::theme::ellipsis(p);
    let bold = Style::default().add_modifier(Modifier::BOLD);
    let mut out = Vec::new();
    for g in groups {
        out.push(Line::styled(clean(g.name), bold));
        for (key, words) in &g.rows {
            let key = clean(key);
            let pad = " ".repeat(key_w - key.width() + GAP);
            out.push(Line::from(vec![
                Span::raw(" ".repeat(INDENT)),
                Span::styled(key, role(Role::Accent, p)),
                Span::raw(pad),
                Span::raw(kit::cut(&clean(words), room, ellipsis)),
            ]));
        }
    }
    out
}

/// How many lines the groups take: each group's name and its rows.
pub(crate) fn line_count(groups: &[HelpGroup]) -> usize {
    groups.iter().map(|g| 1 + g.rows.len()).sum()
}

/// The box over `area` (the whole terminal) and its interior.
fn boxed(groups: &[HelpGroup], area: Rect, p: Palette) -> (Rect, Rect) {
    // The lines and the hint row.
    let content = u16::try_from(line_count(groups) + 1).unwrap_or(u16::MAX);
    let rect = kit::dialog_area(area, content);
    let inner = kit::dialog_frame(TITLE, false, p).inner(rect);
    (rect, inner)
}

/// How many rows the help's lines get over `area`: the interior less the hint row.
pub(crate) fn view_rows(groups: &[HelpGroup], area: Rect, p: Palette) -> usize {
    usize::from(boxed(groups, area, p).1.height.saturating_sub(1))
}

/// The largest scroll over `area`: the stats screen's end (`kit::last_top`), or the
/// line the help opened on when that lies past it, so the context's group is the first
/// drawn at every size (fix round 1 ruling on decision 34; blank rows may follow).
pub(crate) fn max_scroll(groups: &[HelpGroup], view: &HelpView, area: Rect, p: Palette) -> usize {
    let end = kit::last_top(line_count(groups), view_rows(groups, area, p));
    end.max(usize::from(view.opened))
}

/// The hint row: `j/k scroll · tab group · esc close`, dropped by priority, `esc` kept.
fn hint_row(width: u16, p: Palette) -> Line<'static> {
    let hint = |key: &str, word: &str, priority| Hint {
        key: key.to_string(),
        word: word.to_string(),
        priority,
    };
    let hints = [
        hint("j/k", "scroll", 6),
        hint("tab", "group", 5),
        hint("esc", "close", 1),
    ];
    kit::hints_joined(width, &hints, " · ", p)
}

pub fn render(frame: &mut Frame, app: &App, view: &HelpView, area: Rect) {
    let p = app.palette();
    let groups = help_groups(&app.settings.prefix_label);
    let (rect, inner) = boxed(&groups, area, p);
    frame.render_widget(Clear, rect);
    frame.render_widget(kit::dialog_frame(TITLE, false, p), rect);
    if inner.height == 0 || inner.width == 0 {
        return;
    }
    let rows = inner.height - 1;
    let limit = max_scroll(&groups, view, area, p);
    let all = lines(&groups, usize::from(inner.width), p);
    let top = usize::from(view.scroll);
    let shown = kit::from_top_until(all, top, limit, usize::from(rows), p);
    frame.render_widget(
        Paragraph::new(shown),
        Rect {
            height: rows,
            ..inner
        },
    );
    let hint = Rect {
        y: inner.y + rows,
        height: 1,
        ..inner
    };
    frame.render_widget(Paragraph::new(hint_row(inner.width, p)), hint);
}

#[cfg(test)]
#[path = "help_tests.rs"]
mod tests;
