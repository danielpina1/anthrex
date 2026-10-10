//! Milestone 9.10 decision 28 (SP §4.2): the review card. Its bold title, its rows as
//! the profile draws them with the card's cell (`✓ 12s`, `✗ 3m10s`, `checking…` while a
//! row edit verifies), a changed row as `<old> → <new>` (`—` for none), and one
//! `couldn't verify: <label> (<command>) — <reason>` line per dropped command in
//! `Attention`. Split from `ui/profile.rs` by responsibility (`AGENTS.md` rule 8).
//! Pure; every string a profile, a check or the scout wrote passes `safe_text`.

use super::{Parts, plain, value_text};
use crate::app::profile_screen::{ProfileScreen, Shown, Side};
use crate::profile_view::{Row, card_cell};
use crate::profile_words;
use crate::theme::{Palette, Role, ellipsis, role};
use crate::ui::kit::{cut, wrap_words};
use proto::RowEditState;
use ratatui::style::{Modifier, Style};
use ratatui::text::Line;

/// Decision 28's two titles, bold.
pub(super) fn title(s: &ProfileScreen, width: usize, p: Palette) -> Line<'static> {
    let text = if s.has_stored() {
        "anthrex found changes in how to work in this repo"
    } else {
        "anthrex learned how to work in this repo"
    };
    let text = cut(text, width.saturating_sub(1), ellipsis(p));
    Line::styled(
        format!(" {text}"),
        Style::default().add_modifier(Modifier::BOLD),
    )
}

/// The card's cell: `checking…` while the row's edit verifies, `✗ <took>` after a ✗,
/// else its check's `✓ 12s` / `✗ 3m10s`.
fn cell(s: &ProfileScreen, row: &Row, p: Palette) -> Option<(String, Role)> {
    match s.edit_of(&row.key) {
        Some(RowEditState::Verifying) => Some(("checking…".into(), Role::Working)),
        Some(RowEditState::Failed { secs, .. }) => Some((
            format!(
                "{} {}",
                crate::theme::glyph(crate::theme::Glyph::Failed, p.ascii),
                profile_words::took(*secs)
            ),
            Role::Failed,
        )),
        None => row.check.as_ref().map(|c| {
            let r = if c.ok { Role::Done } else { Role::Failed };
            (card_cell(c, p.ascii), r)
        }),
    }
}

/// A card row's parts: on the changes card (a stored profile shown), `<old> → <new>`.
pub(super) fn parts(s: &ProfileScreen, row: &Row, head: bool, p: Palette) -> Parts {
    let new_side = match &s.proposal {
        Side::Ready(shown) => Some(shown.as_ref()),
        _ => None,
    };
    let (value, value_role) = if row.mark.is_some() {
        let old_side: Option<&Shown> = match &s.stored {
            Side::Ready(shown) => Some(shown.as_ref()),
            _ => None,
        };
        let (old, _) = value_text(&row.key, row.old.as_deref(), old_side);
        let (new, _) = value_text(&row.key, row.value.as_deref(), new_side);
        (format!("{old} → {new}"), None)
    } else {
        value_text(&row.key, row.value.as_deref(), new_side)
    };
    Parts {
        head,
        label: row.label.clone(),
        value,
        value_role,
        cell: cell(s, row, p),
        key: row.key.clone(),
    }
}

/// One `couldn't verify: <label> (<command>) — <reason>` line per dropped command,
/// wrapped, in `Attention`.
pub(super) fn dropped_lines(shown: &Shown, width: usize, p: Palette) -> Vec<Line<'static>> {
    let attention = role(Role::Attention, p);
    let mut out = Vec::new();
    for d in &shown.dropped {
        let text = format!(
            "couldn't verify: {} ({}) — {}",
            profile_words::label(&d.key),
            d.command,
            d.reason
        );
        for part in wrap_words(&plain(&text, p), width.saturating_sub(1).max(1)) {
            out.push(Line::styled(format!(" {part}"), attention));
        }
    }
    out
}
