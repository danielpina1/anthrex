//! Which badge the status bar wears and which hints it lists (milestone 9.0.6
//! decision 6). Pure: both are read off `&App`; `statusbar.rs` only draws them.
//!
//! Milestone 9.0.7 decision 33: every hint carries its priority from the brief's
//! Interfaces "Hint priorities" table (higher survives longer), and `esc` is never
//! dropped (`kit::hints`).

use crate::app::screens::Screen;
use crate::app::{App, ReviewTarget, TreeInput};
use crate::ui::kit::Hint;

/// How the hints are joined, and what leads them.
pub(super) struct Body {
    /// Text that always shows before the hints (the prefix list's `C-b ›`).
    pub lead: Option<(String, String)>,
    pub hints: Vec<Hint>,
    pub separator: &'static str,
    /// The focused worktree's git segment follows the hints (the default bar and the
    /// pending prefix; the modes that own the keys have no room for it).
    pub git: bool,
}

fn hint(key: &str, word: &str, priority: u8) -> Hint {
    Hint {
        key: key.to_string(),
        word: word.to_string(),
        priority,
    }
}

/// `esc back`: `kit::hints` never drops it, so its priority is never compared.
fn esc_back() -> Hint {
    hint("esc", "back", u8::MAX)
}

/// `true` while the action menu is open: it draws its own key line.
fn menu_open(app: &App) -> bool {
    matches!(app.modal, Some(crate::app::Modal::Action(_)))
}

/// The mode badge, in decision 6's precedence: ` PREFIX ` first, then ` MENU `, then
/// the screens' (` PROFILE `, ` SETTINGS `, ` STATS `), above ` PLAN `. Milestone 9.0.7
/// decision 31: the run view wears ` RUN `, the project overview ` OVERVIEW ` and the
/// sidebar tree ` TREE `.
pub(super) fn badge(app: &App) -> Option<&'static str> {
    if app.keymap.pending() {
        Some(" PREFIX ")
    } else if menu_open(app) {
        Some(" MENU ")
    } else if matches!(app.screen, Some(Screen::Profile(_))) {
        Some(" PROFILE ")
    } else if matches!(app.screen, Some(Screen::Settings(_))) {
        Some(" SETTINGS ")
    } else if matches!(app.screen, Some(Screen::Stats(_))) {
        Some(" STATS ")
    } else if app.plan_review.is_some() {
        Some(" PLAN ")
    } else if app.alerts_focus.is_some() {
        Some(" ALERTS ")
    } else if app.conversation.is_open() {
        Some(" CHAT ")
    } else {
        match app.tree_input {
            Some(TreeInput::Navigate) if app.run_view.is_some() => Some(" RUN "),
            Some(TreeInput::Navigate) if app.overview => Some(" OVERVIEW "),
            Some(TreeInput::Navigate) => Some(" TREE "),
            Some(TreeInput::Filter) => Some(" FILTER "),
            None => None,
        }
    }
}

/// `true` while a filter is being typed: the bar shows `/text`, not hints.
pub(super) fn filtering(app: &App) -> bool {
    !app.keymap.pending()
        && !menu_open(app)
        && app.screen.is_none()
        && app.plan_review.is_none()
        && app.alerts_focus.is_none()
        && app.tree_input == Some(TreeInput::Filter)
}

/// The hints to list. The pending prefix wins, then the action menu's `esc back` alone
/// (decision 31), the screens, the review, the Alerts view, tree navigation, then the
/// default bar.
pub(super) fn body(app: &App) -> Body {
    let (hints, git) = if app.keymap.pending() {
        let list = [
            ("a", "alerts", 8),
            ("g", "goal", 7),
            ("T", "run", 6),
            ("P", "profile", 5),
            ("S", "settings", 4),
            ("?", "help", 9),
        ];
        let lead = (
            app.settings.prefix_label.clone(),
            crate::theme::glyph(crate::theme::Glyph::Separator, app.settings.badges.ascii)
                .to_string(),
        );
        let git = app.screen.is_none() && app.plan_review.is_none() && app.alerts_focus.is_none();
        return Body {
            lead: Some(lead),
            hints: list.iter().map(|(k, w, p)| hint(k, w, *p)).collect(),
            separator: " · ",
            git: git && app.tree_input.is_none(),
        };
    } else if menu_open(app) {
        (vec![esc_back()], false)
    } else if let Some(Screen::Profile(screen)) = &app.screen {
        (crate::ui::profile::hints(screen), false)
    } else if let Some(Screen::Settings(screen)) = &app.screen {
        (
            crate::ui::settings::hints(screen, app.settings.badges.ascii),
            false,
        )
    } else if let Some(Screen::Stats(_)) = &app.screen {
        (crate::ui::stats::hints(), false)
    } else if app.plan_review.is_some() {
        (review_hints(app), false)
    } else if app.alerts_focus.is_some() {
        (alerts_view_hints(app), false)
    } else if app.tree_input == Some(TreeInput::Navigate) {
        (navigate_hints(app), false)
    } else if app.tree_input.is_some() {
        (vec![], false)
    } else {
        let p = &app.settings.prefix_label;
        (
            vec![
                hint(&format!("{p} ?"), "help", 9),
                hint(&format!("{p} c"), "new shell", 7),
                hint(&format!("{p} t"), "tree", 6),
                hint(&format!("{p} j/k"), "switch", 5),
                hint(&format!("{p} d"), "detach", 8),
            ],
            true,
        )
    };
    Body {
        lead: None,
        hints,
        separator: "  ",
        git,
    }
}

/// Tree navigation's hints: the project tree's, or the run view's (milestone 8c,
/// Interfaces "Status bar and overview title"), the plan gate's while the run awaits
/// approval.
fn navigate_hints(app: &App) -> Vec<Hint> {
    let Some(view) = &app.run_view else {
        return vec![
            hint("j/k", "move", 6),
            hint("⏎", "focus", 8),
            hint(".", "actions", 7),
            hint("space", "fold", 4),
            hint("/", "filter", 5),
            esc_back(),
        ];
    };
    let filter = format!("filter: {}", crate::app::filter_label(view.filter));
    let run = app.runs.runs.iter().find(|run| run.run_id == view.run_id);
    let state = run.map(|run| run.state);
    // Milestone 9: a planning run's submit, and a run's awaiting holds.
    let holds = run.is_some_and(|run| crate::tree::awaiting_holds(run).next().is_some());
    if state == Some(proto::RunState::AwaitingApproval) {
        vec![
            hint("a", "approve", 9),
            hint("x", "reject", 9),
            hint("e", "edit", 6),
            hint("d", "remove", 5),
            hint("p", "review", 8),
            hint("⏎", "open", 4),
            hint(".", "actions", 7),
            hint("f", &filter, 3),
            esc_back(),
        ]
    } else if state == Some(proto::RunState::Planning) {
        // Task 7 note: `j/k` keeps the run view's 6; `p` has nothing to review yet.
        vec![
            hint("s", "submit", 9),
            hint("j/k", "move", 6),
            hint("⏎", "open", 4),
            hint(".", "actions", 7),
            hint("f", &filter, 3),
            esc_back(),
        ]
    } else if holds {
        vec![
            hint("a", "approve hold", 9),
            hint("x", "reject hold", 9),
            hint("p", "review", 8),
            hint("⏎", "open", 4),
            hint(".", "actions", 7),
            hint("f", &filter, 3),
            esc_back(),
        ]
    } else {
        vec![
            hint("j/k", "move", 6),
            hint("h/l", "tier", 4),
            hint("⏎", "open", 8),
            hint(".", "actions", 7),
            hint("PgDn", "panel", 5),
            hint("space", "fold", 3),
            hint("f", &filter, 2),
            hint("/", "find", 1),
            esc_back(),
        ]
    }
}

/// Milestone 9.0.7 decision 11 and Interfaces "Hint priorities": the Alerts view's
/// keys for the selected alert — Enter's entry, `.` where the alert has a node, `o`
/// but on a proposal (Enter opens its screen) — or `esc` alone with none selected.
fn alerts_view_hints(app: &App) -> Vec<Hint> {
    use crate::app::AlertKey;
    use crate::app::alerts_view::{alert_node, enter_label};
    let back = esc_back();
    let Some(key) = app.alerts_focus.as_ref().and_then(|f| f.selected.as_ref()) else {
        return vec![back];
    };
    let mut hints = vec![hint("j/k", "move", 6), hint("⏎", &enter_label(app, key), 9)];
    if alert_node(key).is_some() {
        hints.push(hint(".", "actions", 8));
    }
    if !matches!(key, AlertKey::Proposal(_)) {
        hints.push(hint("o", "open", 7));
    }
    hints.push(back);
    hints
}

/// Milestone 9.0.5 decision 13: the plan review's keys, at the gate or for a hold.
fn review_hints(app: &App) -> Vec<Hint> {
    let tail = [
        hint("j/k", "task", 4),
        hint("PgUp/PgDn", "scroll", 3),
        esc_back(),
    ];
    let head = match app.plan_review.as_ref().map(|review| &review.target) {
        Some(ReviewTarget::Hold(_)) => {
            vec![hint("a", "approve hold", 9), hint("x", "reject hold", 9)]
        }
        _ => vec![
            hint("a", "approve", 9),
            hint("x", "reject", 9),
            hint("e", "edit", 6),
            hint("d", "drop", 5),
        ],
    };
    head.into_iter().chain(tail).collect()
}
