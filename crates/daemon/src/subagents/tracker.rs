//! `SubagentTracker`: the state machine that turns a stream of hook events
//! into `SubagentInfo` rows.
//!
//! A `PreToolUse` spawn call queues a `PendingSpawn` (see `super::spawn`);
//! the `SubagentStart` that follows matches it against the oldest queued
//! spawn whose type agrees (or has none), and every later hook for that
//! agent id updates the matching `Entry` in place.

use super::spawn::{PendingSpawn, spawn_request};
use crate::hooks::{HookKind, ParsedHook};
use proto::{Runtime, SubagentInfo, SubagentState};
use std::collections::VecDeque;
use std::time::{Duration, Instant};

pub const SUBAGENT_RETENTION: Duration = Duration::from_secs(300);
pub const MAX_SUBAGENTS: usize = 50;
pub const MAX_PENDING_SPAWNS: usize = 50;

#[derive(Debug)]
struct PendingEntry {
    spawn: PendingSpawn,
    created: Instant,
}

#[derive(Debug)]
struct Entry {
    id: String,
    parent_id: Option<String>,
    kind: String,
    label: Option<String>,
    model: Option<String>,
    state: SubagentState,
    tool: Option<String>,
    started: Instant,
    ended: Option<Instant>,
    needs_permission: bool,
}

#[derive(Debug, Default)]
pub struct SubagentTracker {
    entries: Vec<Entry>,
    pending: VecDeque<PendingEntry>,
}

impl SubagentTracker {
    pub fn apply(&mut self, runtime: Runtime, hook: &ParsedHook, now: Instant) -> bool {
        let mut changed = false;
        if let Some(spawn) = spawn_request(runtime, hook) {
            if self.pending.len() == MAX_PENDING_SPAWNS {
                self.pending.pop_front();
            }
            self.pending.push_back(PendingEntry {
                spawn,
                created: now,
            });
            changed = true;
        }

        match hook.kind {
            HookKind::SubagentStart => {
                let Some(id) = hook.agent_id.as_ref() else {
                    return changed;
                };
                if self.entries.iter().any(|entry| entry.id == *id) {
                    return changed;
                }
                let matched = self.pending.iter().position(|pending| {
                    pending.spawn.subagent_type.is_none()
                        || pending.spawn.subagent_type.as_ref() == hook.agent_type.as_ref()
                });
                let pending = matched.and_then(|index| self.pending.remove(index));
                if self.entries.len() == MAX_SUBAGENTS {
                    let index = self.oldest_entry_index(true).unwrap_or_else(|| {
                        self.oldest_entry_index(false)
                            .expect("a full tracker has an oldest entry")
                    });
                    self.entries.remove(index);
                }
                self.entries.push(Entry {
                    id: id.clone(),
                    parent_id: pending
                        .as_ref()
                        .and_then(|entry| entry.spawn.parent_id.clone()),
                    kind: hook.agent_type.clone().unwrap_or_else(|| "agent".into()),
                    label: pending.as_ref().and_then(|entry| entry.spawn.label.clone()),
                    model: pending.as_ref().and_then(|entry| entry.spawn.model.clone()),
                    state: SubagentState::Running,
                    tool: None,
                    started: now,
                    ended: None,
                    needs_permission: false,
                });
                true
            }
            HookKind::SubagentStop => {
                let Some(entry) = self.entry_mut(hook.agent_id.as_deref()) else {
                    return changed;
                };
                if entry.state != SubagentState::Running {
                    return changed;
                }
                entry.state = SubagentState::Done;
                entry.tool = None;
                entry.ended = Some(now);
                entry.needs_permission = false;
                true
            }
            HookKind::PreToolUse => {
                let Some(entry) = self.entry_mut(hook.agent_id.as_deref()) else {
                    return changed;
                };
                let next_tool = hook.tool_name.clone();
                let entry_changed = entry.tool != next_tool || entry.needs_permission;
                entry.tool = next_tool;
                entry.needs_permission = false;
                changed || entry_changed
            }
            HookKind::PostToolUse => {
                let Some(entry) = self.entry_mut(hook.agent_id.as_deref()) else {
                    return changed;
                };
                let entry_changed = entry.tool.is_some() || entry.needs_permission;
                entry.tool = None;
                entry.needs_permission = false;
                changed || entry_changed
            }
            HookKind::PermissionRequest => {
                let Some(entry) = self.entry_mut(hook.agent_id.as_deref()) else {
                    return changed;
                };
                let entry_changed = !entry.needs_permission;
                entry.needs_permission = true;
                changed || entry_changed
            }
            _ => changed,
        }
    }

    pub fn child_exited(&mut self, now: Instant) -> bool {
        let mut changed = false;
        for entry in &mut self.entries {
            if entry.state == SubagentState::Running {
                entry.state = SubagentState::Failed;
                entry.tool = None;
                entry.ended = Some(now);
                entry.needs_permission = false;
                changed = true;
            }
        }
        changed
    }

    pub fn input_sent(&mut self) -> bool {
        let mut changed = false;
        for entry in &mut self.entries {
            changed |= entry.needs_permission;
            entry.needs_permission = false;
        }
        changed
    }

    pub fn prune(&mut self, now: Instant) -> bool {
        let old_entries = self.entries.len();
        let old_pending = self.pending.len();
        self.entries.retain(|entry| {
            entry
                .ended
                .is_none_or(|ended| now.saturating_duration_since(ended) < SUBAGENT_RETENTION)
        });
        self.pending
            .retain(|entry| now.saturating_duration_since(entry.created) < SUBAGENT_RETENTION);
        self.entries.len() != old_entries || self.pending.len() != old_pending
    }

    pub fn infos(&self, now: Instant) -> Vec<SubagentInfo> {
        let mut entries: Vec<&Entry> = self.entries.iter().collect();
        entries.sort_by_key(|entry| entry.started);
        entries
            .into_iter()
            .map(|entry| SubagentInfo {
                id: entry.id.clone(),
                parent_id: entry.parent_id.clone(),
                kind: entry.kind.clone(),
                label: entry.label.clone(),
                model: entry.model.clone(),
                state: entry.state,
                tool: entry.tool.clone(),
                started_secs: now.saturating_duration_since(entry.started).as_secs(),
                ended_secs: entry
                    .ended
                    .map(|ended| now.saturating_duration_since(ended).as_secs()),
                needs_permission: entry.needs_permission,
            })
            .collect()
    }

    fn entry_mut(&mut self, id: Option<&str>) -> Option<&mut Entry> {
        let id = id?;
        self.entries.iter_mut().find(|entry| entry.id == id)
    }

    fn oldest_entry_index(&self, finished_only: bool) -> Option<usize> {
        self.entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| !finished_only || entry.state != SubagentState::Running)
            .min_by_key(|(_, entry)| entry.started)
            .map(|(index, _)| index)
    }
}
