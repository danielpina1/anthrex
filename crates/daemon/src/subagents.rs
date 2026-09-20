//! Pure per-window sub-agent state derived from agent hook events.

use crate::hooks::{HookKind, ParsedHook};
use proto::{Runtime, SubagentInfo, SubagentState};
use std::collections::VecDeque;
use std::time::{Duration, Instant};

pub const SUBAGENT_RETENTION: Duration = Duration::from_secs(300);
pub const MAX_SUBAGENTS: usize = 50;
pub const MAX_PENDING_SPAWNS: usize = 50;
pub const LABEL_MAX_CHARS: usize = 60;
pub const CLAUDE_SPAWN_TOOL: &str = "Agent";
pub const CODEX_SPAWN_TOOL: &str = "spawn_agent";
pub const CODEX_SPAWN_TOOL_ALIAS: &str = "collaborationspawn_agent";
pub const CODEX_SPAWN_TYPE_KEY: Option<&str> = None;
pub const CODEX_SPAWN_LABEL_KEYS: (Option<&str>, Option<&str>) = (Some("task_name"), None);
pub const CODEX_SPAWN_MODEL_KEY: Option<&str> = Some("model");

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingSpawn {
    pub parent_id: Option<String>,
    pub subagent_type: Option<String>,
    pub label: Option<String>,
    pub model: Option<String>,
}

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

pub fn spawn_request(runtime: Runtime, hook: &ParsedHook) -> Option<PendingSpawn> {
    if hook.kind != HookKind::PreToolUse {
        return None;
    }
    let (type_key, label_keys, model_key) = match runtime {
        Runtime::Claude if hook.tool_name.as_deref() == Some(CLAUDE_SPAWN_TOOL) => (
            Some("subagent_type"),
            (Some("description"), Some("name"), Some("prompt")),
            Some("model"),
        ),
        Runtime::Codex
            if matches!(
                hook.tool_name.as_deref(),
                Some(CODEX_SPAWN_TOOL | CODEX_SPAWN_TOOL_ALIAS)
            ) =>
        {
            (
                CODEX_SPAWN_TYPE_KEY,
                // Codex has no description-like field; its `name` key slots
                // into make_label's `name` position, unchanged from before.
                (None, CODEX_SPAWN_LABEL_KEYS.0, CODEX_SPAWN_LABEL_KEYS.1),
                CODEX_SPAWN_MODEL_KEY,
            )
        }
        _ => return None,
    };
    let input = hook
        .tool_input
        .as_ref()
        .and_then(serde_json::Value::as_object);
    let string = |field: Option<&str>| {
        field.and_then(|field| {
            input
                .and_then(|object| object.get(field))
                .and_then(serde_json::Value::as_str)
        })
    };
    Some(PendingSpawn {
        parent_id: hook.agent_id.clone(),
        subagent_type: string(type_key).map(str::to_owned),
        label: make_label(
            string(label_keys.0),
            string(label_keys.1),
            string(label_keys.2),
        ),
        model: string(model_key).map(str::to_owned),
    })
}

/// Builds a sub-agent label from candidates tried in order: `description`,
/// then `name`, both used whole, then `prompt`, whose first line only is
/// used since it is the sole candidate that can span multiple lines.
pub fn make_label(
    description: Option<&str>,
    name: Option<&str>,
    prompt: Option<&str>,
) -> Option<String> {
    fn non_empty(value: Option<&str>) -> Option<&str> {
        value.filter(|value| !value.is_empty())
    }
    let label = non_empty(description)
        .or_else(|| non_empty(name))
        .or_else(|| {
            prompt
                .and_then(|value| value.lines().next())
                .filter(|value| !value.is_empty())
        })?;
    if label.chars().count() <= LABEL_MAX_CHARS {
        return Some(label.to_owned());
    }
    Some(
        label
            .chars()
            .take(LABEL_MAX_CHARS - 1)
            .chain(std::iter::once('…'))
            .collect(),
    )
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hooks::{HookKind, ParsedHook};
    use proto::{HookSource, Runtime, SubagentState};
    use serde_json::json;
    use std::time::{Duration, Instant};

    fn hook(kind: HookKind) -> ParsedHook {
        ParsedHook {
            source: HookSource::Claude,
            kind,
            session_id: None,
            agent_id: None,
            agent_type: None,
            tool_name: None,
            tool_input: None,
            notification_type: None,
        }
    }

    fn spawn(parent_id: Option<&str>, subagent_type: Option<&str>, name: &str) -> ParsedHook {
        let mut hook = hook(HookKind::PreToolUse);
        hook.agent_id = parent_id.map(str::to_owned);
        hook.tool_name = Some(CLAUDE_SPAWN_TOOL.into());
        hook.tool_input = Some(json!({
            "subagent_type": subagent_type,
            "name": name,
            "model": "haiku",
            "prompt": format!("prompt for {name}")
        }));
        hook
    }

    fn start(id: &str, kind: Option<&str>) -> ParsedHook {
        let mut hook = hook(HookKind::SubagentStart);
        hook.agent_id = Some(id.into());
        hook.agent_type = kind.map(str::to_owned);
        hook
    }

    fn stop(id: &str) -> ParsedHook {
        let mut hook = hook(HookKind::SubagentStop);
        hook.agent_id = Some(id.into());
        hook
    }

    fn apply(tracker: &mut SubagentTracker, hook: &ParsedHook, now: Instant) -> bool {
        tracker.apply(Runtime::Claude, hook, now)
    }

    #[test]
    fn start_and_stop_pair_by_agent_id() {
        let base = Instant::now();
        let mut tracker = SubagentTracker::default();

        assert!(apply(&mut tracker, &start("a1", Some("Explore")), base));
        assert_eq!(tracker.infos(base)[0].state, SubagentState::Running);
        assert!(apply(
            &mut tracker,
            &stop("a1"),
            base + Duration::from_secs(2)
        ));
        let info = &tracker.infos(base + Duration::from_secs(2))[0];
        assert_eq!(info.id, "a1");
        assert_eq!(info.kind, "Explore");
        assert_eq!(info.state, SubagentState::Done);
        assert_eq!(info.ended_secs, Some(0));
    }

    #[test]
    fn the_oldest_matching_pending_spawn_wins() {
        let base = Instant::now();
        let mut tracker = SubagentTracker::default();
        for (offset, request) in [
            spawn(None, Some("Explore"), "a"),
            spawn(None, Some("Explore"), "b"),
            spawn(None, Some("Plan"), "p"),
        ]
        .into_iter()
        .enumerate()
        {
            assert!(apply(
                &mut tracker,
                &request,
                base + Duration::from_secs(offset as u64)
            ));
        }

        assert!(apply(
            &mut tracker,
            &start("p1", Some("Plan")),
            base + Duration::from_secs(3)
        ));
        assert!(apply(
            &mut tracker,
            &start("e1", Some("Explore")),
            base + Duration::from_secs(4)
        ));
        assert!(apply(
            &mut tracker,
            &start("e2", Some("Explore")),
            base + Duration::from_secs(5)
        ));
        let infos = tracker.infos(base + Duration::from_secs(5));
        assert_eq!(infos[0].label.as_deref(), Some("p"));
        assert_eq!(infos[1].label.as_deref(), Some("a"));
        assert_eq!(infos[2].label.as_deref(), Some("b"));
    }

    #[test]
    fn an_untyped_pending_spawn_matches_any_type() {
        let base = Instant::now();
        let mut tracker = SubagentTracker::default();
        assert!(apply(&mut tracker, &spawn(None, None, "fallback"), base));
        assert!(apply(
            &mut tracker,
            &start("a1", Some("Plan")),
            base + Duration::from_secs(1)
        ));
        assert_eq!(
            tracker.infos(base + Duration::from_secs(1))[0]
                .label
                .as_deref(),
            Some("fallback")
        );
    }

    #[test]
    fn no_match_means_the_session_is_the_parent() {
        let base = Instant::now();
        let mut tracker = SubagentTracker::default();
        apply(
            &mut tracker,
            &spawn(Some("a0"), Some("Explore"), "scout"),
            base,
        );
        apply(
            &mut tracker,
            &start("a1", Some("Plan")),
            base + Duration::from_secs(1),
        );
        let info = &tracker.infos(base + Duration::from_secs(1))[0];
        assert_eq!(info.parent_id, None);
        assert_eq!(info.label, None);
        assert_eq!(info.model, None);
    }

    #[test]
    fn a_spawn_from_inside_a_sub_agent_sets_the_parent() {
        let base = Instant::now();
        let mut tracker = SubagentTracker::default();
        apply(
            &mut tracker,
            &spawn(Some("a1"), Some("Explore"), "nested"),
            base,
        );
        apply(
            &mut tracker,
            &start("a2", Some("Explore")),
            base + Duration::from_secs(1),
        );
        assert_eq!(
            tracker.infos(base + Duration::from_secs(1))[0]
                .parent_id
                .as_deref(),
            Some("a1")
        );
    }

    #[test]
    fn claude_label_prefers_the_description() {
        assert_eq!(
            make_label(Some("described"), Some("named"), Some("ignored prompt")),
            Some("described".into())
        );
    }

    #[test]
    fn claude_label_falls_back_to_name() {
        assert_eq!(
            make_label(None, Some("named"), Some("ignored prompt")),
            Some("named".into())
        );
    }

    #[test]
    fn claude_label_falls_back_to_the_prompts_first_line() {
        let long = "é".repeat(70);
        assert_eq!(
            make_label(None, None, Some("first line\nsecond line")),
            Some("first line".into())
        );
        assert_eq!(
            make_label(None, None, Some(&format!("{long}\nignored"))),
            Some(format!("{}…", "é".repeat(59)))
        );
        assert_eq!(make_label(None, None, None), None);
    }

    #[test]
    fn an_empty_description_is_skipped() {
        assert_eq!(
            make_label(Some(""), Some("named"), Some("ignored prompt")),
            Some("named".into())
        );
        assert_eq!(make_label(Some(""), Some(""), Some("")), None);
    }

    #[test]
    fn codex_labels_are_unchanged() {
        for tool_name in ["spawn_agent", "collaborationspawn_agent"] {
            let mut request = hook(HookKind::PreToolUse);
            request.source = HookSource::CodexHook;
            request.tool_name = Some(tool_name.into());
            request.tool_input = Some(json!({
                "task_name": "list_filenames",
                "fork_turns": "none",
                "model": "gpt-6-astra",
                "message": "opaque runtime value"
            }));

            assert_eq!(
                spawn_request(Runtime::Codex, &request),
                Some(PendingSpawn {
                    parent_id: None,
                    subagent_type: None,
                    label: Some("list_filenames".into()),
                    model: Some("gpt-6-astra".into()),
                })
            );
        }

        let mut missing_task_name = hook(HookKind::PreToolUse);
        missing_task_name.source = HookSource::CodexHook;
        missing_task_name.tool_name = Some("spawn_agent".into());
        missing_task_name.tool_input = Some(json!({
            "type": "unverified type",
            "agent_type": "unverified agent type",
            "message": "never expose this opaque message"
        }));
        assert_eq!(
            spawn_request(Runtime::Codex, &missing_task_name),
            Some(PendingSpawn {
                parent_id: None,
                subagent_type: None,
                label: None,
                model: None,
            })
        );
    }

    #[test]
    fn tool_is_set_by_pre_and_cleared_by_post_tool_use() {
        let base = Instant::now();
        let mut tracker = SubagentTracker::default();
        apply(&mut tracker, &start("a1", None), base);
        let mut pre = hook(HookKind::PreToolUse);
        pre.agent_id = Some("a1".into());
        pre.tool_name = Some("Grep".into());
        assert!(apply(&mut tracker, &pre, base));
        assert_eq!(tracker.infos(base)[0].tool.as_deref(), Some("Grep"));
        let mut post = hook(HookKind::PostToolUse);
        post.agent_id = Some("a1".into());
        assert!(apply(&mut tracker, &post, base));
        assert_eq!(tracker.infos(base)[0].tool, None);
    }

    #[test]
    fn permission_request_marks_the_asking_sub_agent() {
        let base = Instant::now();
        let mut tracker = SubagentTracker::default();
        apply(&mut tracker, &start("a1", None), base);
        let mut permission = hook(HookKind::PermissionRequest);
        permission.agent_id = Some("a1".into());
        assert!(apply(&mut tracker, &permission, base));
        assert!(tracker.infos(base)[0].needs_permission);
        let mut pre = hook(HookKind::PreToolUse);
        pre.agent_id = Some("a1".into());
        pre.tool_name = Some("Read".into());
        assert!(apply(&mut tracker, &pre, base));
        assert!(!tracker.infos(base)[0].needs_permission);
        apply(&mut tracker, &permission, base);
        assert!(tracker.input_sent());
        assert!(!tracker.infos(base)[0].needs_permission);
    }

    #[test]
    fn child_exit_fails_running_entries() {
        let base = Instant::now();
        let mut tracker = SubagentTracker::default();
        apply(&mut tracker, &start("running", None), base);
        apply(&mut tracker, &start("done", None), base);
        apply(&mut tracker, &stop("done"), base);
        assert!(tracker.child_exited(base + Duration::from_secs(4)));
        let infos = tracker.infos(base + Duration::from_secs(4));
        assert_eq!(infos[0].state, SubagentState::Failed);
        assert_eq!(infos[0].ended_secs, Some(0));
        assert_eq!(infos[1].state, SubagentState::Done);
    }

    #[test]
    fn finished_entries_expire_after_300_seconds() {
        let base = Instant::now();
        let mut tracker = SubagentTracker::default();
        apply(&mut tracker, &start("a1", None), base);
        apply(&mut tracker, &stop("a1"), base);
        assert!(!tracker.prune(base + Duration::from_secs(299)));
        assert_eq!(tracker.infos(base + Duration::from_secs(299)).len(), 1);
        assert!(tracker.prune(base + Duration::from_secs(301)));
        assert!(tracker.infos(base + Duration::from_secs(301)).is_empty());
    }

    #[test]
    fn at_most_50_entries_oldest_finished_first() {
        let base = Instant::now();
        let mut tracker = SubagentTracker::default();
        for index in 0..50 {
            apply(
                &mut tracker,
                &start(&format!("a{index}"), None),
                base + Duration::from_secs(index),
            );
        }
        apply(&mut tracker, &stop("a20"), base + Duration::from_secs(50));
        apply(
            &mut tracker,
            &start("a50", None),
            base + Duration::from_secs(51),
        );
        let infos = tracker.infos(base + Duration::from_secs(51));
        assert_eq!(infos.len(), 50);
        assert!(infos.iter().all(|info| info.id != "a20"));
        assert_eq!(infos.first().unwrap().id, "a0");

        let mut all_running = SubagentTracker::default();
        for index in 0..51 {
            apply(
                &mut all_running,
                &start(&format!("r{index}"), None),
                base + Duration::from_secs(index),
            );
        }
        let infos = all_running.infos(base + Duration::from_secs(51));
        assert_eq!(infos.len(), 50);
        assert_eq!(infos.first().unwrap().id, "r1");
    }

    #[test]
    fn pending_spawns_are_capped_and_expire() {
        let base = Instant::now();
        let mut tracker = SubagentTracker::default();
        for index in 0..51 {
            apply(
                &mut tracker,
                &spawn(None, Some("Explore"), &format!("p{index}")),
                base + Duration::from_secs(index),
            );
        }
        apply(
            &mut tracker,
            &start("first", Some("Explore")),
            base + Duration::from_secs(51),
        );
        assert_eq!(
            tracker.infos(base + Duration::from_secs(51))[0]
                .label
                .as_deref(),
            Some("p1")
        );
        assert!(tracker.prune(base + Duration::from_secs(351)));
        apply(
            &mut tracker,
            &start("expired", Some("Explore")),
            base + Duration::from_secs(352),
        );
        assert_eq!(
            tracker.infos(base + Duration::from_secs(352))[1].label,
            None
        );
    }

    #[test]
    fn duplicate_start_and_unknown_stop_are_ignored() {
        let base = Instant::now();
        let mut tracker = SubagentTracker::default();
        assert!(apply(&mut tracker, &start("a1", None), base));
        assert!(!apply(
            &mut tracker,
            &start("a1", Some("Plan")),
            base + Duration::from_secs(1)
        ));
        assert!(!apply(&mut tracker, &stop("unknown"), base));
        let infos = tracker.infos(base + Duration::from_secs(1));
        assert_eq!(infos.len(), 1);
        assert_eq!(infos[0].kind, "agent");
    }

    #[test]
    fn infos_report_seconds_since_start_and_end() {
        let base = Instant::now();
        let mut tracker = SubagentTracker::default();
        apply(&mut tracker, &start("a1", None), base);
        apply(&mut tracker, &stop("a1"), base + Duration::from_secs(3));
        let info = &tracker.infos(base + Duration::from_secs(8))[0];
        assert_eq!(info.started_secs, 8);
        assert_eq!(info.ended_secs, Some(5));
    }

    #[test]
    fn codex_spawn_request_reads_the_verified_fields() {
        for tool_name in ["spawn_agent", "collaborationspawn_agent"] {
            let mut request = hook(HookKind::PreToolUse);
            request.source = HookSource::CodexHook;
            request.tool_name = Some(tool_name.into());
            request.tool_input = Some(json!({
                "task_name": "list_filenames",
                "fork_turns": "none",
                "model": "gpt-6-astra",
                "message": "opaque runtime value"
            }));

            assert_eq!(
                spawn_request(Runtime::Codex, &request),
                Some(PendingSpawn {
                    parent_id: None,
                    subagent_type: None,
                    label: Some("list_filenames".into()),
                    model: Some("gpt-6-astra".into()),
                })
            );
        }

        let mut missing_task_name = hook(HookKind::PreToolUse);
        missing_task_name.source = HookSource::CodexHook;
        missing_task_name.tool_name = Some("spawn_agent".into());
        missing_task_name.tool_input = Some(json!({
            "type": "unverified type",
            "agent_type": "unverified agent type",
            "message": "never expose this opaque message"
        }));
        assert_eq!(
            spawn_request(Runtime::Codex, &missing_task_name),
            Some(PendingSpawn {
                parent_id: None,
                subagent_type: None,
                label: None,
                model: None,
            })
        );

        let mut unrelated = missing_task_name;
        unrelated.tool_name = Some("namespace_spawn_agent".into());
        assert_eq!(spawn_request(Runtime::Codex, &unrelated), None);
    }
}
