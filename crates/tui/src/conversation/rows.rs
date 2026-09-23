//! Rows derived from a conversation, and the cursor that selects one. Pure.

use crate::ui::conversation::diff;
use proto::{Block, Conversation, DegradeReason, DropCause, Role, ToolResult};
use std::collections::BTreeSet;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

/// Columns between the view's interior edge and a `Text` row's first character. Prose is
/// wrapped to the interior width less this, and the renderer draws it at this indent, so
/// the rows the cursor and search walk are exactly the lines on screen.
pub const TEXT_INDENT: usize = 4;

/// Decision A11: the selection, stored as what it points at rather than a row number.
/// `Line(turn, block, n)` is the `n`th row *within* a block (`n >= 1`): a later line of a
/// multi-line `Text` block, or an unfolded tool call's `n`th detail row. Row 0 of every
/// block is `Block(turn, block)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cursor {
    Turn(u64),
    Block(u64, usize),
    Line(u64, usize, usize),
    Dropped,
    Degraded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetailKind {
    Added,
    Removed,
    Context,
    Plain,
    /// The last detail row of a result that `max_result_bytes` (or the hook's own
    /// summary cap) cut short (decision A9). The renderer adds the `⋯` glyph.
    Truncated,
}

/// The text of a `DetailKind::Truncated` row, after its glyph (decision A9).
pub const TRUNCATED: &str = "truncated (conversation.max_result_bytes)";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Row {
    Dropped {
        count: u32,
        cause: DropCause,
    },
    TurnHeader {
        turn_id: u64,
        role: Role,
        at_unix_secs: u64,
    },
    Text {
        turn_id: u64,
        block: usize,
        line: usize,
        text: String,
    },
    Tool {
        turn_id: u64,
        block: usize,
    },
    ToolDetail {
        turn_id: u64,
        block: usize,
        line: usize,
        text: String,
        kind: DetailKind,
        /// A diff line's 1-based position on its own side of its hunk; `None` for
        /// everything else. `Edit` carries no file line numbers, so this is the most
        /// the client can know.
        number: Option<usize>,
    },
    Spawn {
        turn_id: u64,
        block: usize,
        agent_id: String,
    },
    Notice {
        turn_id: u64,
        block: usize,
    },
    Degraded {
        reason: DegradeReason,
    },
}

impl Row {
    /// The cursor that selects this row. Every row has a distinct one, so `j`/`k` can
    /// stop on each row, detail rows and later text lines included.
    pub fn cursor(&self) -> Cursor {
        match self {
            Row::Dropped { .. } => Cursor::Dropped,
            Row::TurnHeader { turn_id, .. } => Cursor::Turn(*turn_id),
            Row::Text {
                turn_id,
                block,
                line,
                ..
            } => line_cursor(*turn_id, *block, *line),
            Row::ToolDetail {
                turn_id,
                block,
                line,
                ..
            } => line_cursor(*turn_id, *block, line + 1),
            Row::Tool { turn_id, block }
            | Row::Spawn { turn_id, block, .. }
            | Row::Notice { turn_id, block } => Cursor::Block(*turn_id, *block),
            Row::Degraded { .. } => Cursor::Degraded,
        }
    }
}

fn line_cursor(turn_id: u64, block: usize, offset: usize) -> Cursor {
    if offset == 0 {
        Cursor::Block(turn_id, block)
    } else {
        Cursor::Line(turn_id, block, offset)
    }
}

impl Cursor {
    /// Where this cursor sits in row order, whether or not its row still exists. Rows
    /// follow turn ids, and the daemon only ever hands out rising ids, so a dropped
    /// turn's cursor still has a well-defined "next row" (decision A11).
    pub(super) fn order(&self) -> (u8, u64, usize, usize) {
        match self {
            Cursor::Dropped => (0, 0, 0, 0),
            Cursor::Turn(turn) => (1, *turn, 0, 0),
            Cursor::Block(turn, block) => (1, *turn, block + 1, 0),
            Cursor::Line(turn, block, offset) => (1, *turn, block + 1, *offset),
            Cursor::Degraded => (2, 0, 0, 0),
        }
    }
}

/// Every row of `conversation`, top to bottom, with the blocks in `unfolded` expanded and
/// prose wrapped to `wrap` display columns (0: not wrapped, before the first size report).
pub(super) fn rows(
    conversation: &Conversation,
    unfolded: &BTreeSet<(u64, usize)>,
    wrap: usize,
) -> Vec<Row> {
    let mut rows = Vec::new();
    if conversation.dropped_turns > 0 {
        rows.push(Row::Dropped {
            count: conversation.dropped_turns,
            cause: conversation.dropped_by.unwrap_or(DropCause::Turns),
        });
    }
    for turn in &conversation.turns {
        rows.push(Row::TurnHeader {
            turn_id: turn.id,
            role: turn.role,
            at_unix_secs: turn.at_unix_secs,
        });
        for (block, content) in turn.blocks.iter().enumerate() {
            block_rows(&mut rows, turn.id, block, content, unfolded, wrap);
        }
    }
    if let Some(reason) = conversation.degraded {
        rows.push(Row::Degraded { reason });
    }
    rows
}

fn block_rows(
    rows: &mut Vec<Row>,
    turn_id: u64,
    block: usize,
    content: &Block,
    unfolded: &BTreeSet<(u64, usize)>,
    wrap: usize,
) {
    match content {
        Block::Text { text } => {
            let wrapped = lines(text).flat_map(|line| wrap_line(line, wrap));
            for (line, text) in wrapped.enumerate() {
                rows.push(Row::Text {
                    turn_id,
                    block,
                    line,
                    text,
                });
            }
        }
        Block::ToolCall {
            name,
            input,
            result,
            ..
        } => {
            rows.push(Row::Tool { turn_id, block });
            if unfolded.contains(&(turn_id, block)) {
                let detail = detail_lines(name, input.as_ref(), result.as_ref());
                rows.extend(detail.into_iter().enumerate().map(
                    |(line, Detail { text, kind, number })| Row::ToolDetail {
                        turn_id,
                        block,
                        line,
                        text,
                        kind,
                        number,
                    },
                ));
            }
        }
        Block::SubagentSpawn { agent_id, .. } => rows.push(Row::Spawn {
            turn_id,
            block,
            agent_id: agent_id.clone(),
        }),
        Block::Notice { .. } => rows.push(Row::Notice { turn_id, block }),
    }
}

/// A text's lines, and always at least one so an empty block still has a row to select.
fn lines(text: &str) -> impl Iterator<Item = &str> {
    let empty = text.lines().next().is_none();
    text.lines().chain(empty.then_some(""))
}

/// `line` broken into rows of at most `width` display columns (0: unbroken). Breaks at
/// spaces, dropping the space it breaks at; a word wider than `width` is broken between
/// graphemes. Widths are display columns, never chars, so a 2-column character counts
/// twice. Every row but a lone empty line is non-empty.
fn wrap_line(line: &str, width: usize) -> Vec<String> {
    if width == 0 || line.width() <= width {
        return vec![line.to_owned()];
    }
    let mut out = Vec::new();
    let mut current = String::new();
    let mut current_width = 0;
    let mut started = false;
    for word in line.split(' ') {
        let word_width = word.width();
        let needed = if started {
            current_width + 1 + word_width
        } else {
            word_width
        };
        if needed <= width {
            if started {
                current.push(' ');
            }
            current.push_str(word);
            current_width = needed;
            started = true;
            continue;
        }
        if started {
            out.push(std::mem::take(&mut current));
            current_width = 0;
        }
        // The word starts a row of its own, broken wherever it is wider than a row.
        for grapheme in word.graphemes(true) {
            let w = grapheme.width();
            if current_width + w > width && current_width > 0 {
                out.push(std::mem::take(&mut current));
                current_width = 0;
            }
            current.push_str(grapheme);
            current_width += w;
        }
        started = true;
    }
    if started {
        out.push(current);
    }
    out
}

/// One detail row before it is placed.
struct Detail {
    text: String,
    kind: DetailKind,
    number: Option<usize>,
}

fn plain(text: impl Into<String>) -> Detail {
    Detail {
        text: text.into(),
        kind: DetailKind::Plain,
        number: None,
    }
}

/// An unfolded call's detail. An `Edit`, `MultiEdit` or `Write` whose input
/// `diff::from_tool_input` reads is shown as its diff — each hunk's old lines removed,
/// then its new lines added; any other input as pretty JSON. The result's summary and
/// detail follow either way, so a failed edit still says why, and a truncated result
/// ends with decision A9's row.
fn detail_lines(
    name: &str,
    input: Option<&serde_json::Value>,
    result: Option<&ToolResult>,
) -> Vec<Detail> {
    let mut out = Vec::new();
    match input.map(|input| (input, diff::from_tool_input(name, input))) {
        Some((_, Some(diff))) => {
            for hunk in &diff.hunks {
                let side = |text: &str, kind| {
                    text.lines()
                        .enumerate()
                        .map(move |(n, line)| Detail {
                            text: line.to_owned(),
                            kind,
                            number: Some(n + 1),
                        })
                        .collect::<Vec<_>>()
                };
                out.extend(side(&hunk.old, DetailKind::Removed));
                out.extend(side(&hunk.new, DetailKind::Added));
            }
        }
        Some((input, None)) => {
            let pretty = serde_json::to_string_pretty(input).unwrap_or_else(|_| input.to_string());
            out.extend(pretty.lines().map(plain));
        }
        None => {}
    }
    if let Some(result) = result {
        out.push(plain(result.summary.clone()));
        if let Some(detail) = &result.detail {
            out.extend(lines(detail).map(plain));
        }
        if result.truncated {
            out.push(Detail {
                text: TRUNCATED.to_owned(),
                kind: DetailKind::Truncated,
                number: None,
            });
        }
    }
    out
}

/// Spec §6: search covers prose (`Text.text`) and tool summaries (`ToolCall.summary`)
/// only — never inputs, results or notices. One hit per matching block, in row order.
pub(super) fn search_hits(conversation: &Conversation, query: &str) -> Vec<Cursor> {
    if query.is_empty() {
        return vec![];
    }
    let needle = query.to_lowercase();
    let mut hits = Vec::new();
    for turn in &conversation.turns {
        for (block, content) in turn.blocks.iter().enumerate() {
            let haystack = match content {
                Block::Text { text } => text,
                Block::ToolCall { summary, .. } => summary,
                _ => continue,
            };
            if haystack.to_lowercase().contains(&needle) {
                hits.push(Cursor::Block(turn.id, block));
            }
        }
    }
    hits
}
