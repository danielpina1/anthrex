//! The conversation view's rendering (task M6.5.13, spec §6's mock). Takes `&App` and
//! does no I/O: every row comes from `ConversationView::rows()`, which already wraps
//! prose to the width drawn here, so the cursor and search walk what is on screen. Every
//! string that comes from the agent passes through `conversation::clean` first.

pub mod diff;

use crate::app::{App, region::KeyRegion};
use crate::conversation::{DetailKind, Row, SUBAGENT_FOOTER, TEXT_INDENT, clean};
use crate::conversation_label::user_turn_label;
use crate::theme::{self, Role::*, role};
use crate::ui::badge::Badge;
use proto::{Block as ConvBlock, DropCause, Role, ToolState, Turn};
use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use std::collections::HashMap;

/// Where rows other than prose start, in columns from the interior's left edge.
const ROW_INDENT: u16 = 2;
/// Where a tool call's detail rows start.
const DETAIL_INDENT: u16 = 6;

/// The view's own glyphs, unicode or ASCII (decision A5 switches both together with the
/// badges, so a screen is never half one and half the other).
struct Glyphs {
    /// Between the window's name and its model, and a spawn's kind and label.
    sep: &'static str,
    /// Whether `punct` folds the unicode punctuation in the view's own text and the
    /// daemon's summaries (`—`, `·`, `…`) to ASCII.
    ascii: bool,
    folded: &'static str,
    unfolded: &'static str,
    spawn: &'static str,
    warn: &'static str,
    more: &'static str,
    crumb: &'static str,
    ok: &'static str,
    failed: &'static str,
    denied: &'static str,
}

/// The view's glyphs, the theme's where they mean the same (final fix wave M5): fold
/// marks, the warning, the tool verdicts. `spawn` and `more` are the view's own
/// (decision 3's conversation glyphs).
const fn glyphs(ascii: bool) -> Glyphs {
    use theme::{Glyph as G, glyph as g};
    Glyphs {
        sep: if ascii { "-" } else { "·" },
        ascii,
        folded: g(G::Collapsed, ascii),
        unfolded: g(G::Expanded, ascii),
        spawn: if ascii { "*" } else { "⟐" },
        warn: g(G::Warning, ascii),
        more: if ascii { "..." } else { "⋯" },
        crumb: if ascii { " > " } else { " › " },
        ok: g(G::Passed, ascii),
        failed: g(G::Failed, ascii),
        denied: g(G::Blocked, ascii),
    }
}

const UNICODE: Glyphs = glyphs(false);
const ASCII: Glyphs = glyphs(true);

impl Glyphs {
    /// Review M4: in ASCII mode, the punctuation the view's own footers and the daemon's
    /// summaries (`crates/daemon/src/conversation/summary.rs`) use, as ASCII. Only these
    /// three: the agent's own prose and names are its text, not the view's drawing.
    fn punct(&self, text: &str) -> String {
        if !self.ascii {
            return text.to_owned();
        }
        text.replace(['—', '·'], "-").replace('…', "...")
    }
}

/// Everything a row needs besides the row itself.
struct Ctx<'a> {
    app: &'a App,
    glyphs: &'a Glyphs,
    badge: &'a Badge,
    turns: HashMap<u64, &'a Turn>,
}

pub fn render(frame: &mut Frame, app: &App, area: Rect) {
    let view = &app.conversation;
    let Some((window_id, _)) = view.key() else {
        return;
    };
    let glyphs = if app.settings.badges.ascii {
        &ASCII
    } else {
        &UNICODE
    };
    let window = app.windows.iter().find(|w| w.id == window_id);
    let conversation = view.conversation();
    let p = app.palette();
    // A sub-agent runs inside its parent's window, so the window's runtime is the badge
    // for every level of the trail.
    let runtime = window
        .map(|w| w.runtime)
        .or(conversation.map(|c| c.runtime))
        .unwrap_or(proto::Runtime::Shell);
    let badge = app.settings.badges.for_runtime(runtime);

    let mut place = window
        .map(|w| w.name.clone())
        .unwrap_or_else(|| format!("window {window_id}"));
    for crumb in view.trail() {
        place.push_str(glyphs.crumb);
        place.push_str(&clean(&crumb.label));
    }
    let mut title = vec![
        Span::styled(badge.text.clone(), Style::default().fg(badge.color)),
        Span::raw(" "),
        Span::raw(place),
    ];
    if let Some(model) = window.and_then(|w| w.model.as_deref()) {
        title.push(Span::raw(format!(" {} {}", glyphs.sep, clean(model))));
    }
    let keys_here = app.key_region() == KeyRegion::Conversation;
    // Milestone 9.0.7 decision 30: no ` rev N `; the revision is the daemon's, not the user's.
    let mut block = super::kit::pane_frame(Line::from(title), keys_here, app.palette());
    if let Some(search) = view.search().filter(|s| s.typing) {
        block = block.title_bottom(Line::from(format!(" /{} ", search.query)).centered());
    }
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let buf = frame.buffer_mut();
    let Some(conversation) = conversation else {
        let waiting = Line::styled("  waiting for the conversation", role(Muted, p));
        buf.set_line(inner.x, inner.y, &waiting, inner.width);
        return;
    };
    let ctx = Ctx {
        app,
        glyphs,
        badge,
        turns: conversation.turns.iter().map(|t| (t.id, t)).collect(),
    };
    let rows = view.rows();
    draw_rows(buf, inner, &ctx, &rows, view.selected_row(&rows));
}

/// Lays the rows out one screen line each, with a blank line above every turn header but
/// the first line, and scrolls so the selected row sits mid-view where it can.
fn draw_rows(buf: &mut Buffer, inner: Rect, ctx: &Ctx, rows: &[Row], selected: Option<usize>) {
    let mut lines = Vec::with_capacity(rows.len());
    let mut line = 0usize;
    for row in rows {
        if matches!(row, Row::TurnHeader { .. }) && line > 0 {
            line += 1;
        }
        lines.push(line);
        line += 1;
    }
    let total = line;
    let height = inner.height as usize;
    let offset = match selected {
        Some(sel) if total > height => lines[sel].saturating_sub(height / 2).min(total - height),
        _ => 0,
    };
    let mut user_turn = false;
    for (index, row) in rows.iter().enumerate() {
        if let Row::TurnHeader { role, .. } = row {
            user_turn = *role == Role::User;
        }
        let Some(y) = lines[index].checked_sub(offset).filter(|y| *y < height) else {
            continue;
        };
        let area = Rect {
            y: inner.y + y as u16,
            height: 1,
            ..inner
        };
        let (indent, left, right) = row_spans(ctx, row, user_turn);
        draw_line(buf, area, indent, left, right);
        if selected == Some(index) {
            buf.set_style(area, Modifier::REVERSED);
            // Final fix wave M3: the bar on the frame's left border, beside the row.
            let x = inner.x.saturating_sub(1);
            super::kit::selection_bar(buf, x, area.y, ctx.app.palette());
        }
    }
}

/// `left` from `indent`, `right` right-aligned with one column to spare; the left part
/// gives way to the right one, and both give way to the interior's edge.
fn draw_line(buf: &mut Buffer, area: Rect, indent: u16, left: Vec<Span>, right: Vec<Span>) {
    let right = Line::from(right);
    let right_width = right.width() as u16;
    let mut left_end = area.right();
    if right_width > 0 && right_width + 2 <= area.width {
        let x = area.right() - right_width - 1;
        buf.set_line(x, area.y, &right, right_width);
        left_end = x.saturating_sub(1);
    }
    let x = area.x.saturating_add(indent);
    if x < left_end {
        buf.set_line(x, area.y, &Line::from(left), left_end - x);
    }
}

type Spans = Vec<Span<'static>>;

/// A row's indent, left spans and right spans.
fn row_spans(ctx: &Ctx, row: &Row, user_turn: bool) -> (u16, Spans, Spans) {
    let g = ctx.glyphs;
    let p = ctx.app.palette();
    let (attention, muted) = (role(Attention, p), role(Muted, p));
    match row {
        Row::Dropped { count, cause } => {
            let key = match cause {
                DropCause::Turns => "conversation.max_turns",
                DropCause::Bytes => "conversation.max_bytes",
            };
            let text = format!("{} {count} earlier turns dropped ({key})", g.more);
            (ROW_INDENT, vec![Span::styled(text, muted)], vec![])
        }
        Row::TurnHeader {
            role,
            at_unix_secs,
            turn_id,
        } => {
            let bold = Style::default().add_modifier(Modifier::BOLD);
            let left = match role {
                // Milestone 9 decision 42i: a delivered message names its sender.
                Role::User => vec![Span::styled(
                    user_turn_label(ctx.turns.get(turn_id).copied()),
                    bold,
                )],
                Role::Assistant => vec![badge(ctx.badge), Span::styled(" assistant", bold)],
                Role::System => vec![Span::styled("system", bold)],
            };
            let time = clock(*at_unix_secs, ctx.app.utc_offset_secs);
            (ROW_INDENT, left, vec![Span::styled(time, muted)])
        }
        Row::Text { line, text, .. } => {
            if user_turn && *line == 0 {
                // Decision 30: the sequence separator `›`; `▸` means collapsed only.
                let lead = theme::glyph(theme::Glyph::Separator, g.ascii);
                let spans = vec![Span::raw(format!("{lead} ")), Span::raw(text.clone())];
                return (TEXT_INDENT as u16 - 2, spans, vec![]);
            }
            (TEXT_INDENT as u16, vec![Span::raw(text.clone())], vec![])
        }
        Row::Tool { turn_id, block } => tool_spans(ctx, *turn_id, *block),
        Row::ToolDetail {
            text, kind, number, ..
        } => (
            DETAIL_INDENT,
            detail_spans(ctx, text, *kind, *number),
            vec![],
        ),
        Row::Spawn { turn_id, block, .. } => {
            let Some(ConvBlock::SubagentSpawn {
                kind, label, model, ..
            }) = block_at(ctx, *turn_id, *block)
            else {
                return (TEXT_INDENT as u16, vec![], vec![]);
            };
            let (kind, label) = (clean(kind), clean(label));
            let left = vec![Span::raw(format!(
                "{} spawned  {kind} {} {label}",
                g.spawn, g.sep
            ))];
            let mut right = vec![badge(ctx.badge)];
            if let Some(model) = model {
                right.push(Span::raw(format!(" {}", clean(model))));
            }
            (TEXT_INDENT as u16, left, right)
        }
        Row::Notice { turn_id, block } => {
            let text = match block_at(ctx, *turn_id, *block) {
                Some(ConvBlock::Notice { text, .. }) => clean(text),
                _ => String::new(),
            };
            let span = Span::styled(format!("{} {text}", g.warn), attention);
            (TEXT_INDENT as u16, vec![span], vec![])
        }
        Row::Degraded { reason } => {
            let text = g.punct(reason.message());
            let span = Span::styled(format!("{} {text}", g.warn), attention);
            (ROW_INDENT, vec![span], vec![])
        }
        Row::SubagentFooter => {
            let text = g.punct(SUBAGENT_FOOTER);
            let span = Span::styled(format!("{} {text}", g.warn), attention);
            (ROW_INDENT, vec![span], vec![])
        }
    }
}

fn block_at<'a>(ctx: &Ctx<'a>, turn_id: u64, block: usize) -> Option<&'a ConvBlock> {
    ctx.turns.get(&turn_id)?.blocks.get(block)
}

fn badge(badge: &Badge) -> Span<'static> {
    Span::styled(badge.text.clone(), Style::default().fg(badge.color))
}

/// `HH:MM` in the zone `offset` seconds from UTC.
fn clock(at_unix_secs: u64, offset: i64) -> String {
    let secs = (at_unix_secs as i64)
        .saturating_add(offset)
        .rem_euclid(86_400);
    format!("{:02}:{:02}", secs / 3600, secs % 3600 / 60)
}

/// `▸ Name  summary` left, `✓ 0.3s` right (spec §6); `▾` once unfolded.
fn tool_spans(ctx: &Ctx, turn_id: u64, block: usize) -> (u16, Spans, Spans) {
    let Some(ConvBlock::ToolCall {
        name,
        summary,
        state,
        duration_ms,
        ..
    }) = block_at(ctx, turn_id, block)
    else {
        return (TEXT_INDENT as u16, vec![], vec![]);
    };
    let g = ctx.glyphs;
    let fold = if ctx.app.conversation.is_unfolded(turn_id, block) {
        g.unfolded
    } else {
        g.folded
    };
    let left = vec![
        Span::raw(format!("{fold} ")),
        Span::styled(clean(name), Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(format!("  {}", g.punct(&clean(summary)))),
    ];
    let (glyph, r) = match state {
        ToolState::Pending => (theme::spinner(ctx.app.spinner_frame, g.ascii), Working),
        ToolState::Ok => (g.ok, Done),
        ToolState::Failed => (g.failed, Failed),
        // A denied call is history, not a request (milestone 9.0.7 ruling).
        ToolState::Denied => (g.denied, Muted),
    };
    let mut right = vec![Span::styled(glyph, role(r, ctx.app.palette()))];
    // Decision 30: a duration that would read `0.0s` (under 50 ms) says nothing.
    if let Some(ms) = duration_ms.filter(|ms| *ms >= 50) {
        right.push(Span::raw(format!(" {:.1}s", f64::from(ms) / 1000.0)));
    }
    (TEXT_INDENT as u16, left, right)
}

/// A diff line under a dimmed number column, tinted by its sign; anything else muted.
fn detail_spans(ctx: &Ctx, text: &str, kind: DetailKind, number: Option<usize>) -> Spans {
    let p = ctx.app.palette();
    let muted = role(Muted, p);
    let tinted = |sign: &str, style: Style| {
        let number = number
            .map(|n| format!("{n:>3}"))
            .unwrap_or_else(|| "   ".into());
        vec![
            Span::styled(number, muted),
            Span::raw("  "),
            Span::styled(format!("{sign}{text}"), style),
        ]
    };
    match kind {
        DetailKind::Removed => tinted("-", role(Failed, p)),
        DetailKind::Added => tinted("+", role(Done, p)),
        DetailKind::Context => tinted(" ", Style::default()),
        DetailKind::Plain => vec![Span::styled(text.to_owned(), muted)],
        DetailKind::Truncated => {
            vec![Span::styled(format!("{} {text}", ctx.glyphs.more), muted)]
        }
    }
}

#[cfg(test)]
#[path = "conversation_tests.rs"]
mod tests;
