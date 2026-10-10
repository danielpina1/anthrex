//! The Profile screen's status line (decision 23, its first row, right-aligned and in
//! its tone's role) and its footer (the daemon's last text, then a failed set-up's
//! reason). Split from `ui/profile.rs` by responsibility (`AGENTS.md` rule 8). Pure.

use super::{ProfileScreen, plain};
use crate::app::App;
use crate::app::profile_screen::Side;
use crate::profile_view::{VERIFIED_KEYS, check_of};
use crate::profile_words::{self, StatusTone};
use crate::theme::{Palette, Role, ellipsis, role};
use crate::ui::kit::{cut, wrap_words};
use proto::{ProfileStatus, ProposalOrigin, ProposalState};
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

/// The status line while the card shows over an unreadable stored profile.
const UNREADABLE_CARD: &str = "Needs review — the profile file can't be read; ⏎ replaces it";

/// The first row: the status line, right-aligned, cut from its left.
pub(super) fn status_line(app: &App, s: &ProfileScreen, width: usize, p: Palette) -> Line<'static> {
    let (text, r) = match (&s.status, &s.status_failed) {
        // Final review M6: on the card Enter is **Use this**, which replaces the file
        // that cannot be read; the line says so in place of `⏎ shows it`.
        (Some(status), _) if status.unparseable.is_some() && s.showing_card() => (
            plain(UNREADABLE_CARD, p),
            status_role(StatusTone::Attention),
        ),
        (Some(status), _) => {
            let (line, tone) = profile_words::status_line_tone(status, app.run_now());
            match saved_failing(s).filter(|_| tone == StatusTone::Ready) {
                Some(key) => {
                    let line = saved_failing_line(status, key, app.run_now());
                    (plain(&line, p), status_role(StatusTone::Attention))
                }
                None => (plain(&line, p), status_role(tone)),
            }
        }
        // No reply will come for the last `Status` (minor 2): why, until one does.
        (None, Some(why)) => (plain(why, p), Role::Failed),
        (None, None) => (format!("loading{}", ellipsis(p)), Role::Muted),
    };
    let text = cut_left(&text, width, ellipsis(p));
    let lead = " ".repeat(width.saturating_sub(text.width()));
    Line::from(vec![Span::raw(lead), Span::styled(text, role(r, p))])
}

/// Final re-review N3: the first command the stored profile keeps although its check
/// failed (**Save anyway**); never on the card.
fn saved_failing(s: &ProfileScreen) -> Option<&'static str> {
    let Side::Ready(shown) = &s.stored else {
        return None;
    };
    let v = shown.verification.as_ref().filter(|_| !s.showing_card())?;
    VERIFIED_KEYS
        .into_iter()
        .find(|key| check_of(v, key).is_some_and(|c| !c.ok))
}

/// `Ready · <label> saved failing its check · verified <age> ago`. The age is the
/// daemon's `verified_at` (the last verification of the commands that passed).
fn saved_failing_line(status: &ProfileStatus, key: &str, now: u64) -> String {
    let line = format!(
        "Ready · {} saved failing its check",
        profile_words::label(key)
    );
    match status.verified_at.or(status.confirmed_at) {
        Some(at) => format!(
            "{line} · verified {} ago",
            profile_words::age(now.saturating_sub(at))
        ),
        None => line,
    }
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

/// Final review C-I1: a ✗ row edit, named with the value that failed, whatever row is
/// selected. On the card it says that **Use this** leaves the edit out (the held ✗
/// does not block it); on the profile, that the stored profile kept its value.
fn edit_notice(s: &ProfileScreen) -> Option<String> {
    let edit = s.failed_row_edit()?;
    let what = format!(
        "your edit of {} ({}) failed its check",
        profile_words::label(&edit.key),
        crate::profile_view::tried_value(&edit.key, edit.value.as_deref())
    );
    if s.showing_card() {
        Some(format!("{what}; ⏎ uses this proposal without it"))
    } else if !s.review_proposal() {
        Some(format!("{what}; the profile is unchanged"))
    } else {
        None
    }
}

/// How many of the daemon's dropped goals (it keeps five) the footer shows.
const DROPPED_SHOWN: usize = 3;
/// A dropped goal's goal is cut to this many characters (the brief's dropped-goal text).
const GOAL_CHARS: usize = 60;

/// Final review D-I2: the last queued goals that could not start, as `profile status`
/// lists them: `dropped the queued goal "<goal>": <reason>`. Final re-review N2: only
/// those recorded since the screen opened or since the last **Use this**.
fn dropped_goals(s: &ProfileScreen) -> Vec<String> {
    let Some(status) = &s.status else {
        return vec![];
    };
    let list: Vec<_> = status
        .dropped_goals
        .iter()
        .filter(|d| d.at >= s.drops_since)
        .collect();
    list[list.len().saturating_sub(DROPPED_SHOWN)..]
        .iter()
        .map(|d| {
            let goal = if d.goal.chars().count() > GOAL_CHARS {
                let kept: String = d.goal.chars().take(GOAL_CHARS - 1).collect();
                format!("{kept}…")
            } else {
                d.goal.clone()
            };
            format!("dropped the queued goal \"{goal}\": {}", d.reason)
        })
        .collect()
}

/// The footer: a ✗ row edit's notice in `Attention`, the daemon's last text (a refusal
/// in `Failed`, else a `Done` muted), a failed set-up's reason, then the last dropped
/// goals, in `Failed`; each at most three lines, none hiding another.
pub(super) fn footer(s: &ProfileScreen, width: usize, p: Palette) -> Vec<Line<'static>> {
    let notice = edit_notice(s).map(|n| (n, Role::Attention));
    let last = match (&s.error, &s.message) {
        (Some(e), _) => Some((e.clone(), Role::Failed)),
        (None, Some(m)) => Some((m.clone(), Role::Muted)),
        _ => None,
    };
    let failed = setup_failure(s).map(|f| (f, Role::Failed));
    let dropped = dropped_goals(s).into_iter().map(|d| (d, Role::Failed));
    notice
        .into_iter()
        .chain(last)
        .chain(failed)
        .chain(dropped)
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
