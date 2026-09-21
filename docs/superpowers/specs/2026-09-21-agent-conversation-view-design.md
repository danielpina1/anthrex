# anthrex design: the agent conversation view

Written 2026-09-21. Supersedes nothing; amends milestone 9's brief in §9 and milestone 8's
in §8.

## 1. What changes

Today a Claude Code or Codex window is a mirrored terminal: a grid of cells with no
structure. You can read it, but anthrex cannot — it cannot fold a tool call, show a diff,
tell you what an agent said thirty turns ago, or put four agents' results side by side.

This milestone gives the daemon a structured model of each agent's conversation, built
from the hooks it already receives and enriched from the runtime's own transcript. The TUI
renders that model as a conversation view. The model is transport-neutral, so a browser UI
can render the same data later without reimplementing anything.

Sub-agents are **visible but not addressable**: you can read any sub-agent's conversation
and state, and you intervene through the orchestrator or by typing into a window, not
through this view. That keeps one authority over a run.

## 2. Terms

- **Conversation** — the structured record of one window's session: an ordered list of turns.
- **Turn** — one party speaking: a user prompt, or an assistant's reply with everything it
  did during that reply.
- **Block** — a piece of a turn: prose, a tool call, a sub-agent spawn, a notice.
- **Hook timeline** — the ordering and state derived from hook events alone. Authoritative.
- **Enrichment** — prose and tool detail read from the runtime's transcript. Optional.
- **Degraded** — a conversation whose enrichment is unavailable. It still renders.

## 3. Where the content comes from

Two sources with different reliability, and the difference is the design.

**Hooks are already plumbed.** `ClientMsg::HookEvent { window_id, source, payload }` carries
the runtime's raw hook JSON. The payloads give ordering, timing, tool names, tool inputs,
tool results, the session id, and — the fact that makes this milestone possible —
`transcript_path`. Nothing new is asked of the agent.

Hooks do not carry the assistant's prose. From hooks alone you see *"ran Edit on parse.rs"*
and never what the agent said about it.

**Transcripts carry the prose.** Each runtime writes a per-session JSONL at the path its
hooks report. It holds message records with content blocks, including the text.

That file is undocumented, unstable across runtime versions, and different for Claude and
Codex. It is not something to build a view's correctness on.

### Decision 1 — hooks are authoritative, transcripts enrich

The timeline, ordering, turn boundaries and tool state come from hooks. Transcript records
are matched onto that timeline by session id and, where the runtime provides one, tool-use
id. The transcript may add prose and tool detail. It may never create, reorder or remove a
turn.

### Decision 2 — a conversation degrades, it does not fail

If the transcript is missing, unreadable, or in a shape the parser does not recognise, the
conversation renders from hooks alone and is marked degraded in the view. A parse error is
logged once per session, not per line. No transcript problem may blank the view, panic the
daemon, or stall a run.

This is the pattern milestone 3 already uses for agent state — hooks with the
output-activity rule as a fallback — and it is chosen for the same reason. We do not
control the format we are reading.

### Decision 3 — one parser per runtime, version-tagged

`transcript::claude` and `transcript::codex`, each behind a `TranscriptParser` trait with a
`detect(first_line) -> Option<Version>`. An unrecognised version degrades rather than
guessing. Adding a runtime is adding a module.

## 4. The conversation model

It lives in `proto` so the daemon, the TUI and any future client agree on it.

```rust
pub struct Conversation {
    pub window_id: u32,
    pub session_id: Option<String>,
    pub runtime: Runtime,
    pub rev: u64,              // bumped on every change; clients resume from it
    pub degraded: Option<DegradeReason>,
    pub dropped_turns: u32,    // trimmed from the front by the cap in decision 7
    pub turns: Vec<Turn>,
}

pub struct Turn {
    pub id: u64,
    pub role: Role,            // User | Assistant | System
    pub at: SystemTime,
    pub state: TurnState,      // Running | Complete
    pub blocks: Vec<Block>,
}

pub enum Block {
    Text { text: String },
    ToolCall {
        id: Option<String>,
        name: String,
        summary: String,               // always present
        input: Option<serde_json::Value>,
        result: Option<ToolResult>,
        state: ToolState,              // Pending | Ok | Failed | Denied
    },
    SubagentSpawn {
        agent_id: String,              // joins the milestone 3 sub-agent record
        kind: String,
        label: String,
        model: Option<String>,
    },
    Notice { kind: NoticeKind, text: String },   // PermissionRequest | Error | Compaction
}

pub struct ToolResult {
    pub ok: bool,
    pub summary: String,
    pub detail: Option<String>,
    pub truncated: bool,
}
```

### Decision 4 — blocks, not lines

A line-oriented model cannot fold a tool call, render a diff, or let you select a sub-agent
spawn. Everything the view is for needs structure, so the model carries structure.

### Decision 5 — `summary` is always present; `input` and `detail` are not

The summary is derived daemon-side from the hook, which always carries the tool name and its
input. So a degraded conversation still reads as *"Edit parse.rs — 3 hunks"* rather than as a
blank. Full input and result detail arrive with enrichment when it is available.

### Decision 5a — the daemon does not currently receive tool results at all

Corrected on inspection, against an earlier draft of this document that assumed it did.
`crates/cli/src/hook.rs:97` removes `tool_response` from every payload before it reaches the
daemon, guarded by `HOOK_PAYLOAD_MAX` above it, and
`cli/tests/hook_command.rs::waits_for_hook_ack_and_strips_only_top_level_tool_response` pins
that behaviour. The strip exists for a good reason: a tool response can be megabytes, and
without it a large payload trips the size limit and the whole hook is dropped.

So the CLI replaces the strip with a **bounded summary** rather than removing the field: it
keeps `tool_response` truncated to `TOOL_RESULT_SUMMARY_MAX` (4 KiB) and adds
`tool_result_truncated: bool`. The size guard keeps working, the daemon gains enough for
`ToolState` and a one-line result summary, and full detail still comes from enrichment.

Without this, `ToolResult` would be enrichment-only and every tool call in a degraded
conversation would show as `Pending` forever — which is exactly the blank-looking failure
decision 2 exists to prevent.

### Decision 6 — a sub-agent spawn is a block in its parent's conversation

It carries the `agent_id` of the existing milestone 3 sub-agent record rather than
duplicating its state. Selecting it opens that sub-agent's own conversation. This is how
"see the sub-agents" is satisfied without a second tracking system, and it is why the model
needs no notion of nesting: nesting is a link, not a tree.

### Decision 7 — conversations are capped

`[conversation] max_turns` (default 500) and `max_bytes` (default 2 MiB) per window, from
`config.toml`. Turns drop from the front and `dropped_turns` counts them, which the view
shows. A tool result longer than `max_result_bytes` (default 16 KiB) is truncated at capture
with `truncated: true`.

An overnight orchestrated run produces transcripts in the tens of megabytes. Unbounded
growth here is a daemon that dies at 4 a.m., which is precisely the run this project exists
to support.

## 5. Protocol

`PROTO_VERSION` becomes **one more than the value on `main` when this milestone starts**,
matching the convention milestone 8's brief already uses.

It is deliberately not a fixed number here. The milestone briefs currently claim 3 (M6),
4 (M7), 5 (M8) and 6 (M9), and every one of those is stale: milestone 5 takes 5, so each
later brief is off by at least one. Hard-coding a sixth number into this spec would add a
fifth wrong answer. Whichever milestone lands next reads `main` and adds one, and the
briefs' numbers are corrected as each one starts.

```
ClientMsg::SubscribeConversation   { window_id, from_rev: Option<u64> }
ClientMsg::UnsubscribeConversation { window_id }

DaemonMsg::ConversationSnapshot { window_id, conversation }
DaemonMsg::ConversationDelta    { window_id, from_rev, to_rev, turns: Vec<TurnPatch> }
DaemonMsg::ConversationGone     { window_id, reason }
```

### Decision 8 — a revision counter, not an event stream

A client subscribing with `from_rev` gets a delta if the daemon still holds that revision,
and a full snapshot otherwise. A client that dies and reattaches — milestone 1's whole
premise — resumes without replaying a session from the beginning.

This is deliberately the same shape as milestone 9's proposed run-revision counter (pending
ruling 2). One mechanism for "what changed since I last looked", used twice.

### Decision 9 — one subscription per window, independent of the PTY subscription

Reading a conversation does not attach you to its terminal, and attaching does not start
parsing a transcript. The daemon parses a transcript only while at least one client is
subscribed to that conversation, plus a short linger so switching views does not re-parse.

## 6. The view

A new full-area view, opened with `C-b m` (for messages), showing the focused window's
conversation. During a run it opens on the orchestrator by default.

`C-b k` would have been the natural choice and is already bound. The free single letters at
the time of writing are `a b e f g h i m n p u v`; `m` is taken here.

```
┌─ ◆ orchestrator · claude-opus-5 ────────────── rev 214 ─┐
│                                                          │
│  you                                            09:42    │
│  ▸ refactor the parser into its own module               │
│                                                          │
│  ◆ assistant                                    09:42    │
│    I'll map the call sites first, then split the         │
│    module in two.                                        │
│    ▸ Grep  "parse_" → 34 matches            ✓ 0.3s       │
│    ▾ Edit  crates/parse/src/lib.rs          ✓ 1.1s       │
│         12  -fn parse_all(src: &str) {                   │
│         12  +fn parse_all(src: &str) -> Result<Ast> {    │
│    ⟐ spawned  Explore · find every call site   ◆ haiku   │
│                                                          │
│  ⚠ transcript unavailable — timeline only                │
└──────────────────────────────────────────────────────────┘
```

- **Tool calls fold.** One line by default — name, summary, state, duration. `Enter` or
  `o` unfolds to show input and result.
- **Edit and Write unfold as diffs**, tinted by the existing theme. The hook's `tool_input`
  already carries old and new strings, so this works while degraded.
- **Sub-agent spawns are selectable.** `Enter` on one opens that sub-agent's conversation;
  `Esc` returns. A breadcrumb shows where you are.
- **Search** with `/`, matching prose and tool summaries, `n`/`N` to walk hits.
- **Degradation is visible**, never silent — the footer says so, and the view still works.

### Decision 10 — runtime badges, not brand marks

Each conversation and each sub-agent carries a two-cell badge: a glyph plus a colour.
Claude `◆`, Codex `◇`, shell `$`, fake-agent `·`. `[C]`, `[X]`, `[$]` when the terminal
reports no unicode support, and colour alone never carries meaning.

Real Claude and Codex logos are not renderable in a terminal grid, and shipping third-party
brand marks in a distributed tool carries trademark risk we have no reason to take. A future
browser UI can use real marks if that is wanted; the model already says which runtime it is,
so nothing has to change here for that to happen.

Glyphs and colours are `[conversation.badges]` keys in `config.toml`.

### Decision 11 — the view is read-only

No send, no steer, no approve. It observes. Typing goes to a window through the existing
attach path; intervention in a run goes through the orchestrator.

The reason is authority: a run whose tasks can be redirected from two places at once has no
single account of why it did what it did, and milestone 8's engine is built on being able to
give that account. This is a deliberate limit and the cheapest one to relax later if it
proves wrong.

## 7. Where it lives

```
crates/proto/src/conversation.rs      the model, the messages
crates/daemon/src/conversation.rs     per-window state, revisions, caps
crates/daemon/src/conversation/
    build.rs                          hook events → turns and blocks
    enrich.rs                         transcript records → prose and detail
crates/daemon/src/transcript.rs       the parser trait, version detection
crates/daemon/src/transcript/
    claude.rs  codex.rs               one module per runtime
crates/tui/src/conversation.rs        view state: folding, selection, search
crates/tui/src/ui/conversation.rs     rendering
crates/tui/src/ui/badge.rs            runtime badges
```

`build.rs` is pure: hook payloads in, turns out, no I/O. `enrich.rs` is pure over parsed
records. All file reading sits in `transcript.rs`, on the same hardened `subprocess`/timeout
discipline the worktree code uses — a transcript is an untrusted file that another process
is writing while we read it.

## 8. What milestone 8 gains

Milestone 8's scope is unchanged except that run and task rows gain a conversation. The
engine already knows which window belongs to which task; the view already renders any
window. No new engine concept.

`anthrex run status` stays the CLI surface. The conversation view is how a human watches;
the CLI is how a script does.

## 9. The plan gate, amending milestone 9

This settles pending ruling 5, which the milestone 9 refresh had cut.

### Decision 12 — `submit_plan` blocks until the user answers

The orchestrator calls `submit_plan(plan)` and the call does not return. The daemon marks the
run `AwaitingApproval` and raises the plan view. The user accepts, edits or rejects, and the
call returns that verdict with the plan as edited.

No seventh MCP tool. The gate is the absence of a return value, which is a thing MCP already
does well.

### Decision 13 — the plan is editable in three fields

Per task: `runtime`, `model`, `brief` — the same three pending ruling 4 scopes `revise_task`
to. Tasks may also be removed before accepting. Adding a task is not offered: a plan the user
writes half of is neither the orchestrator's plan nor reviewable as one.

### Decision 14 — `run start --yes` skips the gate

An unattended overnight run cannot block on a human who is asleep, and a gate nobody can
answer is a hang. `--yes` accepts whatever plan the orchestrator submits and is recorded in
the run report, so a run that was never approved says so.

This is the one place the two things asked for — a plan you approve, and runs that go all
night — genuinely conflict. A flag is the honest resolution.

### Decision 15 — an unanswered gate survives a restart

`AwaitingApproval` persists like any other run state (milestone 6). A daemon restart leaves
the run Paused and awaiting approval rather than losing the plan.

## 10. Milestones and order

| | | |
|--|--|--|
| **M6** | persistence and config | unchanged; `config.toml` lands here and this design uses it |
| **M6.5** | **agent conversation view** | this spec, §§3-7 |
| **M8** | orchestration engine | unchanged, plus §8 |
| **M9** | orchestrator agent | as briefed, amended by §9 |
| M7 | split panes | deferred; nothing here depends on it |

M6.5 is inserted rather than appended, matching how 4.5, 4.6 and 4.7 were handled.

**M8 depends on M6 and cannot precede it** — its own acceptance criterion is that a state
file written by milestone 6 loads with no warning. M6.5 is placed before M8 because the
conversation view is how a run is watched; built afterwards, the first orchestrated runs
would be observed through raw terminal panes, which is the problem this solves.

## 11. Testing

- **`build.rs` and `enrich.rs` are pure**, so every turn-boundary and tool-state rule is a
  plain-value test with no daemon and no files.
- **Golden transcripts** per runtime and per known format version, as fixtures. A new runtime
  version arrives as a new fixture.
- **Degradation is tested directly**: absent file, truncated final line, unknown version,
  a line that is valid JSON but the wrong shape, and a file being appended to while read.
  Each must yield a rendering conversation with `degraded` set.
- **`fake-agent` writes a transcript**, so the whole pipeline — hook to model to view — is
  exercised with no model and no network. This is what makes the milestone testable in CI.
- **Cap behaviour** is tested at the boundary: the turn that trips `max_turns`, the result
  that trips `max_result_bytes`.
- **One smoke stage**: start a fake agent, open the view, assert its turns and a folded tool
  call render, unfold it, assert the detail appears.

## 12. Risks

1. **Transcript formats change without warning.** The largest risk and the reason for
   decisions 1 and 2: when it happens we lose prose and keep the timeline. The failure is
   visible, bounded and not a crash. Golden fixtures make the change obvious rather than
   silent.
2. **Reading a file another process is writing.** A partial final line is normal, not an
   error. The parser must treat a truncated tail as "not yet", never as corruption, and must
   never hold a lock that blocks the agent writing it.
3. **Memory.** Decision 7 caps it. Without that cap an overnight run is a daemon that dies
   before morning.
4. **Parsing cost on a busy run.** Ten agents each appending is ten files tailed. Decision 9
   limits parsing to subscribed conversations, so the resting cost of a run nobody is
   watching is zero.
5. **The read-only limit may chafe.** If watching without being able to redirect turns out to
   be the wrong call, decision 11 is the cheapest thing in this design to reverse — the
   protocol already addresses windows by id.
