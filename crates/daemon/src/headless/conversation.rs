//! Session events to milestone 6.5's two conversation inputs (decision 27, the
//! Interfaces mapping table). Pure.

use super::SessionEvent;
use crate::conversation::align::{Alignment, Prompt, alignment};
use crate::hooks::{HookKind, ParsedHook};
use crate::transcript::Record;
use proto::{HookSource, Runtime};
use serde_json::json;
use std::collections::{HashMap, VecDeque};

/// What `map` carries from one event to the next of the same session.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StreamCursor {
    /// Turns the daemon has sent in this session; the next one's ordinal.
    prompts_sent: u32,
    /// Whether the latest sent turn is still open: prose after its `TurnEnded` belongs
    /// to a turn the daemon did not send.
    turn_open: bool,
    /// A turn has ended and no turn has been sent since: an `Init` now starts a turn
    /// Claude Code began by itself. False at the session's start, so its first `Init`
    /// is never mistaken for one.
    after_turn_end: bool,
    /// Top-level tool names by id, for the synthesised `PostToolUse`.
    tool_names: HashMap<String, String>,
    session_id: Option<String>,
    /// Content mode (ruling T7-N1): set by the first `UserPromptSubmit` the manager
    /// passes to `observe_hook`. From then on the prompts the hooks report, not the
    /// timing of `Init`, number the turns.
    hook_feed: bool,
    /// Prompts observed through `observe_hook`; the next one's ordinal. It counts the
    /// same hooks M6.5 builds `User` turns from, so the two never disagree.
    prompts_seen: u32,
    /// Texts the daemon sent that no observed prompt has matched yet, oldest first.
    pending_sent: VecDeque<String>,
    /// An observed prompt no sent text matched, in case its `sent_turn` comes late.
    last_unmatched: Option<String>,
    /// The observed prompts whose turns have not ended yet, oldest first: `true` for the
    /// daemon's. Empty: prose now belongs to a turn whose prompt hook was lost, which has
    /// no `User` turn to land on. Counting open prompts rather than flagging the last
    /// `TurnEnded` keeps a prompt hook applied before the previous turn's `result` from
    /// dropping its own turn's prose (M8a.7 re-review 2, m2).
    open_prompts: VecDeque<bool>,
    /// The latest `TurnEnded` ended a turn Claude Code started by itself while a turn
    /// the daemon sent was still waiting to run ([`StreamCursor::ended_unprompted`]).
    ended_unprompted: bool,
}

impl StreamCursor {
    /// A cursor in content mode from the start, for a window whose real prompt hooks the
    /// manager feeds to [`observe_hook`] (a headless Claude window). Its first
    /// `sent_turn` then records no timing-based prompt, so a first prompt hook that is
    /// lost, or that belongs to a background turn, cannot misalign the rest of the
    /// session (M8a.7 re-review 2, m1).
    pub fn fed() -> Self {
        StreamCursor {
            hook_feed: true,
            ..Self::default()
        }
    }

    /// Whether the `TurnEnded` just mapped ended a turn Claude Code started by itself (a
    /// background sub-agent's notification) while a turn the daemon sent was still
    /// waiting for its prompt: that `result` is not the delivered turn's end. Judged by
    /// content in a fed cursor only; always false otherwise.
    pub fn ended_unprompted(&self) -> bool {
        self.ended_unprompted
    }
}

/// Milestone 6.5's two inputs: hooks for `WindowManager::conversation_hook`, records
/// for `ConversationSet::enrich`, applied in that order.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ConversationInput {
    pub hooks: Vec<ParsedHook>,
    pub records: Vec<Record>,
}

/// The conversation inputs for one session event, per the Interfaces table. Hooks are
/// synthesised only when `hooks_fire` is false (every Codex session; Claude only if its
/// turn hooks did not fire in `-p`, which M8a.1 found they do), with `source: Stream`,
/// `session_source: None` and the session id from `Init`.
///
/// Refinements of the table, from M8a.1's recordings and M8a.7's review:
///
/// - Claude repeats `Init` every turn and Codex every process, so `SessionStart` is
///   synthesised only when the session id changes.
/// - A turn Claude Code starts by itself (a background sub-agent finishing) begins with
///   an `Init` after a `TurnEnded`, with no turn sent in between. The caller must apply
///   `sent_turn` before it writes the turn's message, or the `Init` that message starts
///   could be taken for such a turn. With `hooks_fire`, its own `UserPromptSubmit`
///   hook builds a `User` turn for it, so the cursor counts it as a prompt (ruling
///   T7-I1) and later ordinals keep matching the hook-built turns. Its prose gets no
///   record, since no prompt text was sent for it. Without hooks nothing builds that
///   turn, so it is not counted.
/// - Codex numbers the items of every process from `item_0`, so one session repeats ids
///   across turns. Codex tool ids are namespaced as `t<k>:<id>`, `k` the latest sent
///   turn's ordinal, in both the hooks and the records.
pub fn map(
    runtime: Runtime,
    hooks_fire: bool,
    event: &SessionEvent,
    cursor: &mut StreamCursor,
) -> ConversationInput {
    let mut input = ConversationInput::default();
    match event {
        SessionEvent::Init { session_id, .. } => {
            if hooks_fire && !cursor.turn_open && cursor.after_turn_end {
                cursor.prompts_sent = cursor.prompts_sent.saturating_add(1);
                cursor.after_turn_end = false;
            }
            if cursor.session_id.as_deref() != Some(session_id.as_str()) {
                cursor.session_id = Some(session_id.clone());
                input.hooks.push(cursor.hook(HookKind::SessionStart));
            }
        }
        SessionEvent::AssistantText { text, parent: None } => {
            let ordinal = if hooks_fire && cursor.hook_feed {
                cursor
                    .prompts_seen
                    .checked_sub(1)
                    .filter(|_| !cursor.open_prompts.is_empty())
            } else {
                cursor
                    .prompts_sent
                    .checked_sub(1)
                    .filter(|_| cursor.turn_open)
            };
            if let Some(ordinal) = ordinal {
                input.records.push(Record::AssistantText {
                    session_id: cursor.session_id.clone(),
                    ordinal,
                    text: text.clone(),
                });
            }
        }
        SessionEvent::ToolUse {
            id: raw_id,
            name,
            input: tool_input,
            parent,
        } => {
            let id = &cursor.tool_id(runtime, raw_id);
            input.records.push(Record::ToolDetail {
                tool_use_id: id.clone(),
                input: Some(tool_input.clone()),
                detail: None,
                ok: None,
            });
            if parent.is_none() {
                if cursor.tool_names.len() < TOOL_NAMES_MAX {
                    cursor.tool_names.insert(id.clone(), name.clone());
                }
                input.hooks.push(ParsedHook {
                    tool_name: Some(name.clone()),
                    tool_input: Some(tool_input.clone()),
                    tool_use_id: Some(id.clone()),
                    ..cursor.hook(HookKind::PreToolUse)
                });
            }
        }
        SessionEvent::ToolResult {
            id: raw_id,
            text,
            ok,
            parent,
        } => {
            let id = &cursor.tool_id(runtime, raw_id);
            input.records.push(Record::ToolDetail {
                tool_use_id: id.clone(),
                input: None,
                detail: Some(text.clone()),
                ok: Some(*ok),
            });
            if parent.is_none() {
                let key = if *ok { "output" } else { "error" };
                let (response, truncated, stringified) =
                    proto::conversation::bound_tool_response_value(json!({ key: text }));
                input.hooks.push(ParsedHook {
                    tool_name: cursor.tool_names.remove(id),
                    tool_use_id: Some(id.clone()),
                    tool_response: Some(response),
                    tool_result_truncated: Some(truncated),
                    tool_result_stringified: Some(stringified),
                    ..cursor.hook(HookKind::PostToolUse)
                });
            }
        }
        SessionEvent::TurnEnded { .. } => {
            cursor.turn_open = false;
            cursor.after_turn_end = true;
            cursor.ended_unprompted = hooks_fire && cursor.hook_feed && cursor.end_turn();
            cursor.tool_names.clear();
            input.hooks.push(cursor.hook(HookKind::Stop));
        }
        _ => {}
    }
    if hooks_fire {
        input.hooks.clear();
    }
    input
}

/// The conversation inputs for a turn the daemon starts with `text`: its prompt, as the
/// `k`-th (0-based) turn sent in this session (M6.5 ruling R1). `human: true` because a
/// hook-built prompt is exactly `text`, so M6.5's exact-match alignment applies.
///
/// In content mode (`hooks_fire`, and the manager feeds hooks to `observe_hook`) the
/// prompt's own hook numbers the turn, so this only remembers `text` for matching and
/// returns nothing.
pub fn sent_turn(
    _runtime: Runtime,
    hooks_fire: bool,
    text: &str,
    cursor: &mut StreamCursor,
) -> ConversationInput {
    if hooks_fire {
        let already = cursor
            .last_unmatched
            .as_deref()
            .is_some_and(|seen| same_prompt(seen, text));
        if already {
            cursor.last_unmatched = None;
        } else {
            if cursor.pending_sent.len() >= PENDING_SENT_MAX {
                cursor.pending_sent.pop_front();
            }
            cursor.pending_sent.push_back(text.to_owned());
        }
        if cursor.hook_feed {
            return ConversationInput::default();
        }
    }
    let ordinal = cursor.prompts_sent;
    cursor.prompts_sent = cursor.prompts_sent.saturating_add(1);
    cursor.turn_open = true;
    cursor.after_turn_end = false;
    let hooks = if hooks_fire {
        Vec::new()
    } else {
        vec![ParsedHook {
            prompt: Some(text.to_owned()),
            ..cursor.hook(HookKind::UserPromptSubmit)
        }]
    };
    ConversationInput {
        hooks,
        records: vec![Record::UserText {
            session_id: cursor.session_id.clone(),
            ordinal,
            text: text.to_owned(),
            human: true,
        }],
    }
}

/// A real hook the manager has just applied to this headless window's conversation
/// (through `WindowManager::conversation_hook`, under the same lock). Only a
/// `UserPromptSubmit` matters: it starts a turn, and its prompt says whose turn it is
/// (ruling T7-N1).
///
/// - The prompt is numbered by the count of prompts observed, which is the count of
///   `User` turns M6.5 built from the same hooks, whoever sent them.
/// - It is the daemon's turn (`human: true`) when it equals, as M6.5's alignment has
///   it, a text the daemon sent and no earlier prompt matched. Otherwise Claude Code
///   started it itself (a background sub-agent's notification), and it is `human: false`.
///   Either way its prose lands on its own turn.
///
/// Claude runs command hooks before it acts on the prompt, and `anthrex hook` waits for
/// the daemon's ack, so a turn's prompt is observed before any of its prose is read.
pub fn observe_hook(
    _runtime: Runtime,
    hook: &ParsedHook,
    cursor: &mut StreamCursor,
) -> ConversationInput {
    if hook.kind != HookKind::UserPromptSubmit {
        return ConversationInput::default();
    }
    cursor.hook_feed = true;
    let text = hook.prompt.clone().unwrap_or_default();
    let matched = cursor
        .pending_sent
        .iter()
        .position(|sent| same_prompt(&text, sent));
    let human = match matched {
        Some(at) => {
            // Older texts never got a prompt of their own: their hooks were lost.
            cursor.pending_sent.drain(..=at);
            cursor.last_unmatched = None;
            true
        }
        None => {
            cursor.last_unmatched = Some(text.clone());
            false
        }
    };
    let ordinal = cursor.prompts_seen;
    cursor.prompts_seen = cursor.prompts_seen.saturating_add(1);
    if cursor.open_prompts.len() >= PENDING_SENT_MAX {
        cursor.open_prompts.pop_front();
    }
    cursor.open_prompts.push_back(human);
    if hook.session_id.is_some() {
        cursor.session_id.clone_from(&hook.session_id);
    }
    ConversationInput {
        hooks: Vec::new(),
        records: vec![Record::UserText {
            session_id: cursor.session_id.clone(),
            ordinal,
            text,
            human,
        }],
    }
}

/// Whether an observed prompt is the text the daemon sent: M6.5's alignment for a typed
/// prompt, equal up to surrounding whitespace.
fn same_prompt(observed: &str, sent: &str) -> bool {
    let sent = Prompt {
        session_id: None,
        text: sent,
        human: true,
    };
    alignment(observed, &sent) == Alignment::Exact
}

/// The most sent texts a cursor holds unmatched. Deliveries are one at a time, so more
/// than a few means their hooks are being lost, and the oldest is dropped.
const PENDING_SENT_MAX: usize = 16;

/// The most tool names a cursor remembers. A turn's names are dropped when it ends, so
/// this only bounds a turn with calls whose results never arrive.
const TOOL_NAMES_MAX: usize = 1024;

impl StreamCursor {
    /// Closes the oldest open turn a `TurnEnded` can belong to, and returns whether it
    /// was a turn Claude Code started by itself while a sent turn waits.
    ///
    /// - When an open prompt is the daemon's, the `result` ends it, with every older
    ///   open prompt: an unprompted prompt observed before it was taken into the same
    ///   turn (a notification mid-turn, case D).
    /// - Otherwise the oldest open prompt is an unprompted turn's. Its `result` is
    ///   unprompted when a text the daemon sent is still waiting for its prompt (the N1
    ///   order); with nothing waiting, it is an ordinary turn end.
    /// - With no open prompt (its hook was lost) it is an ordinary turn end.
    fn end_turn(&mut self) -> bool {
        match self.open_prompts.iter().position(|&human| human) {
            Some(at) => {
                self.open_prompts.drain(..=at);
                false
            }
            None => self.open_prompts.pop_front().is_some() && !self.pending_sent.is_empty(),
        }
    }

    /// The tool id the conversation uses: Codex's namespaced by the latest sent turn.
    fn tool_id(&self, runtime: Runtime, id: &str) -> String {
        match runtime {
            Runtime::Codex => format!("t{}:{id}", self.prompts_sent.saturating_sub(1)),
            _ => id.to_owned(),
        }
    }

    /// A synthesised hook of `kind` with every optional field empty.
    fn hook(&self, kind: HookKind) -> ParsedHook {
        ParsedHook {
            source: HookSource::Stream,
            kind,
            session_id: self.session_id.clone(),
            agent_id: None,
            agent_type: None,
            tool_name: None,
            tool_input: None,
            notification_type: None,
            transcript_path: None,
            tool_use_id: None,
            tool_response: None,
            tool_result_truncated: None,
            tool_result_stringified: None,
            prompt: None,
            session_source: None,
        }
    }
}

#[cfg(test)]
#[path = "conversation_tests.rs"]
mod tests;
