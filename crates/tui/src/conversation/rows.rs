//! Rows derived from a conversation, and the cursor that selects one. Pure.

use proto::{Block, Conversation, DegradeReason, DropCause, Role, ToolResult};
use std::collections::BTreeSet;

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
}

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

/// Every row of `conversation`, top to bottom, with the blocks in `unfolded` expanded.
pub(super) fn rows(conversation: &Conversation, unfolded: &BTreeSet<(u64, usize)>) -> Vec<Row> {
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
            block_rows(&mut rows, turn.id, block, content, unfolded);
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
) {
    match content {
        Block::Text { text } => {
            for (line, text) in lines(text).enumerate() {
                rows.push(Row::Text {
                    turn_id,
                    block,
                    line,
                    text: text.to_owned(),
                });
            }
        }
        Block::ToolCall { input, result, .. } => {
            rows.push(Row::Tool { turn_id, block });
            if unfolded.contains(&(turn_id, block)) {
                let detail = detail_lines(input.as_ref(), result.as_ref());
                rows.extend(
                    detail
                        .into_iter()
                        .enumerate()
                        .map(|(line, text)| Row::ToolDetail {
                            turn_id,
                            block,
                            line,
                            text,
                            kind: DetailKind::Plain,
                        }),
                );
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

/// An unfolded call's detail: the input as pretty JSON, then the result's summary and
/// detail. Plain rows only; task M6.5.13 turns `Edit`/`Write`/`MultiEdit` into diffs.
fn detail_lines(input: Option<&serde_json::Value>, result: Option<&ToolResult>) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(input) = input {
        let pretty = serde_json::to_string_pretty(input).unwrap_or_else(|_| input.to_string());
        out.extend(pretty.lines().map(str::to_owned));
    }
    if let Some(result) = result {
        out.push(result.summary.clone());
        if let Some(detail) = &result.detail {
            out.extend(lines(detail).map(str::to_owned));
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
