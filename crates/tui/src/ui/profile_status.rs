//! The Profile screen's status line (decision 23, its first row, right-aligned and in
//! its tone's role) and its footer (the daemon's last text, then a failed set-up's
//! reason). Split from `ui/profile.rs` by responsibility (`AGENTS.md` rule 8). Pure.

use super::{ProfileScreen, plain};
use crate::app::App;
use crate::profile_words::{self, StatusTone};
use crate::theme::{Palette, Role, ellipsis, role};
use crate::ui::kit::{cut, wrap_words};
use proto::{ProposalOrigin, ProposalState};
use ratatui::text::{Line, Span};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

/// `text` cut to its last `max` columns, starting with the ellipsis when cut.
fn cut_left(text: &str, max: usize, ell: &str) -> String {
    if text.width() <= max {
        return text.to_string();
    }
    let room = max.saturating_sub(ell.width());
    let mut kept = Vec::new();
    let mut used = 0;
    for g in text.graphemes(true).rev() {
        if used + g.width() > room {
            break;
        }
        used += g.width();
        kept.push(g);
    }
    kept.reverse();
    if max >= ell.width() {
        format!("{ell}{}", kept.concat())
    } else {
        kept.concat()
    }
}

/// Decision 23's role for a status line's tone: `Done` ready, `Working` setting up or
/// re-checking, `Attention` review and out of date, `Failed` unreadable.
fn status_role(tone: StatusTone) -> Role {
    match tone {
        StatusTone::Unreadable => Role::Failed,
        StatusTone::Working => Role::Working,
        StatusTone::Attention => Role::Attention,
        StatusTone::Ready => Role::Done,
        StatusTone::NotSetUp => Role::Muted,
    }
}

/// The first row: the status line, right-aligned, cut from its left.
pub(super) fn status_line(app: &App, s: &ProfileScreen, width: usize, p: Palette) -> Line<'static> {
    let (text, r) = match (&s.status, &s.status_failed) {
        (Some(status), _) => {
            let (line, tone) = profile_words::status_line_tone(status, app.run_now());
            (plain(&line, p), status_role(tone))
        }
        // No reply will come for the last `Status` (minor 2): why, until one does.
        (None, Some(why)) => (plain(why, p), Role::Failed),
        (None, None) => (format!("loading{}", ellipsis(p)), Role::Muted),
    };
    let text = cut_left(&text, width, ellipsis(p));
    let lead = " ".repeat(width.saturating_sub(text.width()));
    Line::from(vec![Span::raw(lead), Span::styled(text, role(r, p))])
}

/// Decision 23: a failed set-up's reason, the screen's error row.
fn setup_failure(s: &ProfileScreen) -> Option<String> {
    let proposal = s.status.as_ref()?.proposal.as_ref()?;
    match (&proposal.state, &proposal.origin) {
        (_, ProposalOrigin::Edit { .. }) => None,
        (ProposalState::Failed { reason }, _) => Some(format!("setting up failed: {reason}")),
        _ => None,
    }
}

/// The footer: the daemon's last text (a refusal in `Failed`, else a `Done` muted),
/// then a failed set-up's reason in `Failed`; each at most three lines, neither hiding
/// the other.
pub(super) fn footer(s: &ProfileScreen, width: usize, p: Palette) -> Vec<Line<'static>> {
    let last = match (&s.error, &s.message) {
        (Some(e), _) => Some((e.clone(), Role::Failed)),
        (None, Some(m)) => Some((m.clone(), Role::Muted)),
        _ => None,
    };
    let failed = setup_failure(s).map(|f| (f, Role::Failed));
    last.into_iter()
        .chain(failed)
        .flat_map(|(text, r)| footer_text(&text, r, width, p))
        .collect()
}

/// One footer text, wrapped to at most three lines, what is cut marked.
fn footer_text(text: &str, r: Role, width: usize, p: Palette) -> Vec<Line<'static>> {
    let w = width.max(1);
    let mut lines = wrap_words(&plain(text, p), w);
    if lines.len() > 3 {
        lines.truncate(3);
        // What is cut is marked (principle 6).
        let last = format!("{} {}", lines[2], ellipsis(p));
        lines[2] = cut(&last, w, ellipsis(p));
    }
    lines
        .into_iter()
        .map(|l| Line::styled(l, role(r, p)))
        .collect()
}
