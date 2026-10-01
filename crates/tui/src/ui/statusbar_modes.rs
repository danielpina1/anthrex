//! Which badge the status bar wears and which hints it lists (milestone 9.0.6
//! decision 6). Pure: both are read off `&App`; `statusbar.rs` only draws them.

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

/// The mode badge, in decision 6's precedence: ` PREFIX ` first, then ` MENU `, then
/// the screens' (` PROFILE `), above ` PLAN `.
pub(super) fn badge(app: &App) -> Option<&'static str> {
    if app.keymap.pending() {
        Some(" PREFIX ")
    } else if matches!(app.modal, Some(crate::app::Modal::Action(_))) {
        Some(" MENU ")
    } else if matches!(app.screen, Some(Screen::Profile(_))) {
        Some(" PROFILE ")
    } else if app.plan_review.is_some() {
        Some(" PLAN ")
    } else if app.alerts_focus.is_some() {
        Some(" ALERTS ")
    } else if app.conversation.is_open() {
        Some(" CHAT ")
    } else {
        match app.tree_input {
            Some(TreeInput::Navigate) if app.run_view.is_some() => Some(" RUN "),
            Some(TreeInput::Navigate) => Some(" TREE "),
            Some(TreeInput::Filter) => Some(" FILTER "),
            None => None,
        }
    }
}

/// `true` while a filter is being typed: the bar shows `/text`, not hints.
pub(super) fn filtering(app: &App) -> bool {
    !app.keymap.pending()
        && app.screen.is_none()
        && app.plan_review.is_none()
        && app.alerts_focus.is_none()
        && app.tree_input == Some(TreeInput::Filter)
}

/// The hints to list. The pending prefix wins, then the review, the alerts box, tree
/// navigation, then the default bar.
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
    } else if let Some(Screen::Profile(screen)) = &app.screen {
        (crate::ui::profile::hints(screen), false)
    } else if app.plan_review.is_some() {
        (review_hints(app), false)
    } else if app.alerts_focus.is_some() {
        (
            vec![
                hint("j/k", "move", 5),
                hint("⏎", "go", 5),
                hint("esc", "back", 5),
            ],
            false,
        )
    } else if app.tree_input == Some(TreeInput::Navigate) {
        (navigate_hints(app), false)
    } else if app.tree_input.is_some() {
        (vec![], false)
    } else {
        let p = &app.settings.prefix_label;
        (
            vec![
                hint(&format!("{p} ?"), "help", 5),
                hint(&format!("{p} c"), "new shell", 5),
                hint(&format!("{p} t"), "tree", 5),
                hint(&format!("{p} j/k"), "switch", 5),
                hint(&format!("{p} d"), "detach", 5),
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
    let h = |k: &str, w: &str| hint(k, w, 5);
    let Some(view) = &app.run_view else {
        return vec![
            h("j/k", "move"),
            h("⏎", "focus"),
            h("space", "fold"),
            h("/", "filter"),
            h("esc", "back"),
        ];
    };
    let filter = format!("filter: {}", crate::app::filter_label(view.filter));
    let run = app.runs.runs.iter().find(|run| run.run_id == view.run_id);
    let state = run.map(|run| run.state);
    // Milestone 9: a planning run's submit, and a run's awaiting holds.
    let holds = run.is_some_and(|run| crate::tree::awaiting_holds(run).next().is_some());
    if state == Some(proto::RunState::AwaitingApproval) {
        vec![
            h("a", "approve"),
            h("x", "reject"),
            h("e", "edit"),
            h("d", "remove"),
            h("p", "review"),
            h("⏎", "open"),
            h("f", &filter),
            h("esc", "back"),
        ]
    } else if state == Some(proto::RunState::Planning) {
        vec![
            h("s", "submit"),
            h("j/k", "move"),
            h("⏎", "open"),
            h("f", &filter),
            h("esc", "back"),
        ]
    } else if holds {
        vec![
            h("a", "approve hold"),
            h("x", "reject hold"),
            h("p", "review"),
            h("⏎", "open"),
            h("f", &filter),
            h("esc", "back"),
        ]
    } else {
        vec![
            h("j/k", "move"),
            h("h/l", "tier"),
            h("⏎", "open"),
            h("space", "fold"),
            h("f", &filter),
            h("/", "find"),
            h("esc", "back"),
        ]
    }
}

/// Milestone 9.0.5 decision 13: the plan review's keys, at the gate or for a hold.
fn review_hints(app: &App) -> Vec<Hint> {
    let h = |k: &str, w: &str| hint(k, w, 5);
    let tail = [h("j/k", "task"), h("PgUp/PgDn", "scroll"), h("esc", "back")];
    let head = match app.plan_review.as_ref().map(|review| &review.target) {
        Some(ReviewTarget::Hold(_)) => vec![h("a", "approve hold"), h("x", "reject hold")],
        _ => vec![
            h("a", "approve"),
            h("x", "reject"),
            h("e", "edit"),
            h("d", "drop"),
        ],
    };
    head.into_iter().chain(tail).collect()
}
