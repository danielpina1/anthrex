# Milestone 3: Agent status and sub-agent tracking from hooks

## Header

| Field | Value |
|-------|-------|
| Status | `done` |
| Depends on | Milestone 2 (CI) |
| Spec sections | Product spec (`docs/superpowers/specs/2026-09-18-anthrex-product-design.md`) sections 4 (all), 10.1 (the version-2 row), 10.4 (the environment variables), 11.1 to 11.4, 11.6 (the session-id part), 12 (the fake agent and the `ANTHREX_CLAUDE_BIN` / `ANTHREX_CODEX_BIN` overrides). Core spec (`docs/superpowers/specs/2026-09-17-anthrex-design.md`) sections 3.2 (launchers with hooks) and 3.4 (the full status table). |
| Branch | `m3-agent-status` |
| Protocol version | 2 (from 1). Rule (product spec 10.1): set `PROTO_VERSION` to one more than the value on `main` when you start; 2 assumes roadmap order. |

## Goal

Every Claude Code and Codex window reports its real state: working, the tool it is running, waiting for approval, finished, or idle. The state comes from the agents' own hooks, with the milestone-1 output-activity rule as a fallback until the first hook arrives. Every window also records its session id, its model, and the sub-agents its session has spawned, with their parent, kind, label, model, state and current tool. The project tree (milestone 4) and orchestration (milestones 8 and 9) are built on these records. Killing a window now ends its whole process group within about a second.

## Scope

In:

- The `--settings` hooks for Claude and the notify, title, bell and lifecycle-hook flags for Codex.
- The hidden `anthrex hook` command and the daemon's `HookEvent` handling.
- The complete core spec 3.4 status table, including the fallback rule.
- Session id, model and sub-agent tracking in `WindowInfo`, protocol version 2.
- The `crates/fake-agent` test binary and the `ANTHREX_CLAUDE_BIN` / `ANTHREX_CODEX_BIN` environment overrides.
- The Codex verification tasks required by product spec 11.1, 11.3 and 11.4.
- The tool name as the third line of a sidebar card (core spec 6.1).
- `anthrex ls --json`, so a human can inspect sub-agents before the tree exists.
- The follow-ups assigned to milestone 3 (see "Follow-ups handled").

Out:

- The project tree and any rendering of sub-agents in the client. That is milestone 4.
- The config file and its `runtimes.*.command` and `runtimes.codex.bypass_hook_trust` keys. That is milestone 6.
- Resume, restart and persistence of session ids. That is milestone 6. This milestone only records the id.
- The `mcp_call` fake-agent step. It needs the MCP server of milestone 8.
- Claude "agent teams" (product spec 4.2). They are never shown as children.
- Toast behaviour: the client's Attention and Done toasts stay exactly as milestone 1 implemented them in `App::replace_windows`.

## Design decisions

### Files and modules

1. Hook payload parsing lives in a new `crates/daemon/src/hooks.rs`. It turns a `(HookSource, serde_json::Value)` into a typed `ParsedHook` and performs no I/O.
2. The per-window sub-agent tracker lives in a new `crates/daemon/src/subagents.rs`. It is pure: every method takes `now: Instant` as a parameter, so tests control time.
3. The per-window agent facts (session id, tool, which signals have been seen, the last title, the sub-agent tracker) live in a new `crates/daemon/src/agent_state.rs` as `AgentState`. `manager.rs` owns one per `Entry` and calls it; this keeps `manager.rs` under the 600-line limit.
4. `status::next` stays a pure function. It gains a `StatusContext` argument and the new `StatusEvent` variants listed under Interfaces. It never records a session id or a tool; `AgentState` does that.
5. `crates/daemon/src/launch.rs` becomes a directory module: `launch/mod.rs` (`LaunchPlan`, `LaunchContext`, `plan`, `shell_quote`, `hook_command`), `launch/claude.rs` (the settings JSON) and `launch/codex.rs` (Codex flags, trust key, trust hash). The public path `daemon::launch::{plan, LaunchPlan, LaunchContext}` does not change.
6. The `anthrex hook` command lives in a new `crates/cli/src/hook.rs`.
7. The fake agent is a new workspace member `crates/fake-agent`, package `anthrex-fake-agent`, binary `fake-agent`, `publish = false`.

### Hook delivery

8. `anthrex hook --window <id> --source <claude|codex-notify|codex-hook> [payload]` is dispatched in `main` before `Cli::parse()` runs: when `argv[1] == "hook"`, `main` calls `hook::run(args)` and then `std::process::exit(0)`. It parses its own arguments leniently, so a bad argument never produces a clap error or a non-zero exit.
9. The payload is the last argument when there is one after the flags, otherwise stdin. Stdin is read on a separate thread, capped at 8 MiB (`HOOK_PAYLOAD_MAX`); a larger payload is dropped.
10. The whole command runs under one deadline, `HOOK_DEADLINE = 1 s`, measured from process start: stdin read, connect, `Hello` with `ClientKind::Hook`, `Welcome`, `HookEvent`, and the wait for the reply. When the deadline passes the command exits 0 at once.
11. The command waits for the daemon's `Ack { request: "hook" }` (or `Error`) before exiting. Claude runs hooks one after another and waits for each, so this guarantees the daemon applies `PreToolUse` before the matching `PostToolUse`.
12. The command prints nothing on stdout or stderr, installs a silent panic hook, runs its work in a spawned task whose `JoinError` is ignored, and always exits 0. It never starts a daemon. The socket comes from `proto::paths::socket_path()`, which honours the `ANTHREX_SOCKET` the launcher passes.
13. Before sending, the command removes the top-level `tool_response` key from the payload. It can be megabytes and the daemon never reads it. Every other key is forwarded unchanged.
14. The daemon answers every well-formed `HookEvent` with `Ack { request: "hook" }`, including events it ignores. It answers `Error { request: "hook", message: "no window with id N" }` only for an unknown window.

### Launch

15. `<exe>` is the running binary from `std::env::current_exe()` in the real daemon. `ManagerConfig` carries it explicitly so tests can point it at `target/debug/anthrex`.
16. `shell_quote(s)` wraps `s` in single quotes and replaces every `'` inside with `'\''`. It is used only where the exe lands inside a shell command string: the Claude hook commands and the Codex lifecycle-hook commands. Codex `notify` is an argv array and is never shell-quoted.
17. `hook_command(exe, id, source)` is exactly `format!("{} hook --window {id} --source {label}", shell_quote(exe))`, where `label` is `claude`, `codex-notify` or `codex-hook`.
18. The Claude argv is `--name <name> --settings <json> [--model <model>] [-- <prompt>]`. The `--` before the prompt stays, as milestone 1 added it. `<json>` is `serde_json::to_string` of the value built by `launch::claude::settings(exe, id)`, which is compact with keys sorted. It registers the ten events of product spec 4.2, in this exact shape (pretty-printed here; exe `/opt/anthrex/bin/anthrex`, window 4):

    ```json
    {
      "hooks": {
        "Notification":      [{"hooks": [{"command": "'/opt/anthrex/bin/anthrex' hook --window 4 --source claude", "type": "command"}], "matcher": ""}],
        "PermissionRequest": [ same ],
        "PostToolUse":       [ same ],
        "PreToolUse":        [ same ],
        "SessionEnd":        [ same ],
        "SessionStart":      [ same ],
        "Stop":              [ same ],
        "SubagentStart":     [ same ],
        "SubagentStop":      [ same ],
        "UserPromptSubmit":  [ same ]
      },
      "preferredNotifChannel": "terminal_bell"
    }
    ```

19. The Codex argv follows product spec 11.2 in this order: `-C <cwd>`, the five `-c` flags for `notify`, `tui.terminal_title`, `tui.notifications`, `tui.notification_method`, `tui.notification_condition`, then the lifecycle-hook flags of decision 21 when enabled, then `[-m <model>]`, then `[-- <prompt>]`. Every TOML string inside a `-c` value is written with `launch::codex::toml_string(s)`, which returns a TOML basic string: `"`, then `s` with backslash as `\\`, `"` as `\"`, newline `\n`, carriage return `\r`, tab `\t`, backspace `\b`, form feed `\f`, and every other character below U+0020 and U+007F as `\uXXXX` with uppercase hex, then `"`. Everything else is copied. For text without control characters this equals `serde_json::to_string(s)`, and every escape it writes is also valid JSON. Milestones 8 and 9 reuse this function for role instructions; do not add a second TOML string helper. The `notify` value is therefore `notify=["<exe>","hook","--window","<id>","--source","codex-notify"]`, which is also valid JSON, and the fake agent relies on that.
20. The program comes from `ManagerConfig.claude_bin` and `ManagerConfig.codex_bin`. `ManagerConfig::from_env` reads `ANTHREX_CLAUDE_BIN` and `ANTHREX_CODEX_BIN`, defaulting to `claude` and `codex`. An empty value counts as unset.
21. Codex lifecycle hooks, when enabled (decision 23), are the eight events `SessionStart`, `UserPromptSubmit`, `PreToolUse`, `PermissionRequest`, `PostToolUse`, `SubagentStart`, `SubagentStop`, `Stop`, in that order, with snake labels `session_start`, `user_prompt_submit`, `pre_tool_use`, `permission_request`, `post_tool_use`, `subagent_start`, `subagent_stop`, `stop`. For each event, two flags follow each other:
    - `-c hooks.<Event>=[{hooks=[{type="command",command=<toml_string(cmd)>}]}]`, where `cmd` is `hook_command(exe, id, CodexHook)`;
    - `-c hooks.state.<toml_string(key)>.trusted_hash="<hash>"`, where `key = trust_key(source, label, 0, 0)` and `hash = trust_hash(label, cmd)`.
    Task M3.9 may change the config key spelling, the key's source path or the canonical hash form to what the installed Codex actually uses. It must update this decision's tests and record the change under "Implementation notes".
22. `trust_key(source, label, group, handler)` is `format!("{source}:{label}:{group}:{handler}")`. `trust_hash(label, command)` is `"sha256:"` followed by the lowercase hex SHA-256 of `canonical_json` of `{"event_name": label, "matcher": null, "hooks": [{"type": "command", "command": command, "timeout": 600}]}`. `canonical_json` sorts object keys recursively itself, whatever serde_json features are enabled, and writes compact JSON with no spaces. Test vectors, with command `'/opt/anthrex/bin/anthrex' hook --window 7 --source codex-hook`:
    - label `pre_tool_use`: canonical text `{"event_name":"pre_tool_use","hooks":[{"command":"'/opt/anthrex/bin/anthrex' hook --window 7 --source codex-hook","timeout":600,"type":"command"}],"matcher":null}`, hash `sha256:ab7d40f6d23df8495cf24c13782e8df327ca7d298c7f298f840a6c436baf93c9`;
    - label `session_start`: hash `sha256:6c98ad65309ebdb9cfaa45a29c0fe280b05f9b7462e2cfc84189479028f9ba37`.
23. `ManagerConfig.codex_hook_source: Option<String>` switches Codex lifecycle hooks on and names the trust key's source path. `None` means Codex windows launch without lifecycle hooks, as the product spec 11.3 decision says. `ManagerConfig::from_env` fills it from `launch::codex::default_hook_source()`, whose body is fixed by task M3.9. If M3.9 ends in the fallback branch, that function returns `None`. anthrex never passes `--dangerously-bypass-hook-trust`.
24. At startup the daemon runs `<codex_bin> --version` once on `spawn_blocking` with a 5-second timeout. `launch::codex::parse_version` takes the last whitespace-separated token that parses as `X.Y.Z`. Output `codex-cli 0.155.0` was verified against the installed 0.155.0. The daemon logs a warning below 0.135.0 and at debug level when the version cannot be read. Nothing else depends on it.

### Status

25. `AgentState` keeps two flags. `signals_seen` becomes true on the first accepted hook event of any source or the first recognised Codex title. `hooks_seen` becomes true on the first accepted `claude` or `codex-hook` event. The manager computes `StatusContext` from the flags before it applies the event, then updates the flags. "Before" matters for the `SessionStart` row.
26. `StatusContext.focused` is true while at least one client is subscribed to the window. `Entry` gains `viewers: u32`. `WindowManager::focus` increments it and applies `Focused`. A new `WindowManager::unfocus` decrements it, saturating at 0. The server calls `unfocus` when a client's subscription moves to another window, on `Unsubscribe`, and when the client disconnects.
27. Source filtering happens before parsing: Claude windows accept only `claude`, Codex windows accept `codex-notify` and `codex-hook`, and Shell windows accept nothing. A rejected event is logged at debug level and acknowledged.
28. `status::next(current, event, runtime, ctx)` is the core spec 3.4 table plus the refinements below. The first matching row wins:

    | Runtime | Event | Result |
    |---------|-------|--------|
    | all | current is `Exited`, or event `Exited` | `Exited` |
    | Claude with `ctx.hooks_seen` | `Bell` | unchanged (decision 29) |
    | all others | `Bell` | `Attention` |
    | all | `Focused` | `Done` becomes `Idle`, others unchanged |
    | all | `InputSent` | `Attention` becomes `Working`, others unchanged |
    | Shell, or Claude/Codex without `ctx.signals_seen` | `Output` | `Starting`, `Idle`, `Done` become `Working` |
    | same | `Quiet` | `Working` becomes `Idle` |
    | Claude/Codex with `ctx.signals_seen` | `Output`, `Quiet` | unchanged |
    | Claude, Codex | `SessionStart` | `Idle` if not `ctx.signals_seen` and current is not `Attention`, else unchanged |
    | Claude, Codex | `UserPromptSubmit` | `Working` |
    | Claude, Codex | `PreToolUse` | `Working`, except that `Attention` stays `Attention` (decision 30) |
    | Claude, Codex | `PostToolUse` | unchanged |
    | Claude, Codex | `PermissionRequest`, `PermissionPrompt` | `Attention` |
    | Claude | `IdlePrompt` | `Starting` and `Working` become `Idle`; `Done`, `Attention`, `Idle` unchanged (decision 31) |
    | Claude, Codex | `Stop` | `Done` if not `ctx.focused`, else `Idle` |
    | Codex without `ctx.hooks_seen` | `CodexNotify` | `Done` if not `ctx.focused`, else `Idle` |
    | Codex with `ctx.hooks_seen` | `CodexNotify` | from `Working` only: `Done` if not focused, else `Idle` |
    | Codex without `ctx.hooks_seen` | `Title(Starting)` | `Starting` |
    | same | `Title(Working)`, `Title(Thinking)` | `Working` |
    | same | `Title(Waiting)` | `Attention` |
    | same | `Title(Ready)` | from `Working`: `Done` if not focused, else `Idle`; `Done` stays `Done`; others become `Idle` (decision 32) |
    | Codex with `ctx.hooks_seen` | `Title(Waiting)` | `Attention` |
    | same | `Title(Ready)` | from `Working` only, as above; others unchanged |
    | same | `Title(Starting / Working / Thinking)` | unchanged (decision 33) |
    | any other combination | | unchanged |

29. A Claude window that has seen a hook ignores `Bell`. `preferredNotifChannel: "terminal_bell"` makes Claude ring on every notification, including `idle_prompt`, and the hooks already report permission prompts exactly. Without this rule every idle notification would turn a finished window into Attention. Before the first hook, the bell stays the backup signal.
30. `PreToolUse` does not clear `Attention`. With parallel sub-agents, one can be waiting for approval while another keeps calling tools. `Attention` is cleared by `InputSent`, `UserPromptSubmit` or `Stop`.
31. `IdlePrompt` does not clear `Done`. `Done` means "finished, not yet looked at" and is cleared by `Focused`. Clearing it after Claude's idle timer would lose that.
32. A Codex `Ready` title does not demote `Done` to `Idle`. The manager also drops a title identical to the previous one from the same window (`AgentState.last_title`), because a program may re-send an unchanged title.
33. Once lifecycle hooks run in a Codex window, the hooks report working states more precisely than the title. `Waiting` still means Attention. `Ready` from `Working` still ends a turn that fired no `Stop`, for example an interrupted one.
34. `WindowInfo.tool` is the session's own current tool: set by `PreToolUse` and cleared by `PostToolUse` when the event has no `agent_id`, and cleared by `Stop`. Events with an `agent_id` update that sub-agent's `tool` instead. The window's status still follows them per the table.
35. A Claude window's session id is the `session_id` of the latest `SessionStart`. If it is still `None` when any other accepted Claude event arrives, that event's `session_id` fills it.
36. A Codex window's session id is written once (product spec 11.4 and 11.6). The first `codex-hook` `SessionStart` without `agent_id` sets it; otherwise the first notify `thread-id` sets it. It is never overwritten after that, so a sub-agent's own thread id can never replace it. A notify whose `thread-id` differs from a known session id is a sub-agent's turn: it yields no status event and changes nothing. A `codex-hook` `Stop` that carries an `agent_id` is ignored for status.
37. `WindowInfo.model` is the model the window was launched with, `WindowSpec.model`.

### Sub-agents

38. The spawning tool is `Agent` for Claude (product spec 4.2) and `spawn_agent` for Codex (product spec 11.4). A `PreToolUse` for it pushes a `PendingSpawn` with `parent_id` = the event's `agent_id`, `subagent_type`, `label` and `model` from `tool_input`. For Claude, `label` is `tool_input.name`, else the first line of `tool_input.prompt`. For Codex, the `tool_input` field names are unknown until task M3.11. Until then a Codex pending spawn has `subagent_type`, `label` and `model` all `None`, so the label falls back to `agent_type` as the spec says.
39. A label longer than 60 characters is cut to 59 characters plus `…`. Characters are `char`s, not bytes.
40. `SubagentStart` takes the oldest pending spawn whose `subagent_type` equals the event's `agent_type`, or is `None`, and removes it (first in, first out). The new entry inherits its `parent_id`, `label` and `model`. With no match, `parent_id` is `None` and `label` and `model` are `None`. `kind` is `agent_type`, or `"agent"` when the payload has none.
41. A `SubagentStart` with an `agent_id` that is already tracked is ignored. A `SubagentStop` for an unknown id is ignored.
42. Pending spawns are capped at 50 per window, dropping the oldest, and pruned after 300 seconds.
43. Finished entries (`Done` or `Failed`) are dropped 300 seconds after they ended, checked in `WindowManager::tick`. When an insert would make 51 entries, the oldest finished entry is dropped; if none is finished, the oldest entry is dropped. `WindowInfo.subagents` lists entries by start time, oldest first.
44. When the child exits, every `Running` entry becomes `Failed` with `ended_secs` starting at 0.
45. Product spec 4.4 rule 4 needs the sub-agent row to show which sub-agent asked for permission, and `SubagentInfo` has no field for it. `SubagentInfo` gains `needs_permission: bool`. A `PermissionRequest` with an `agent_id` sets it on that entry. The next `PreToolUse`, `PostToolUse` or `SubagentStop` for that entry clears it, and `InputSent` to the window clears it on every entry. Product spec 4.4 carries this field, and milestone 4 draws it as `◆` on the sub-agent row.

### Process groups and input

46. portable-pty 0.9 calls `setsid()` in the child before exec (verified in `portable-pty-0.9.0/src/unix.rs`), and anthrex's `/bin/sh -c 'exec "$0" "$@"'` keeps the pid. The child's pid is therefore its process-group id. `Window::signal_group(sig)` calls `libc::killpg(pid, sig)`. `ESRCH` counts as success, meaning the group is already gone.
47. Kill escalation: SIGHUP to the group at once, SIGTERM to the group after `HUP_GRACE = 1 s`, SIGKILL to the group at `KILL_GRACE = 3 s` from the start. The escalation stops early when `killpg(pid, 0)` returns `ESRCH`. `WindowManager::kill`, `WindowManager::shutdown` and the parser-panic path all use it. `WindowManager::remove` sends SIGKILL to the group at once.
48. The manager decides whether to signal a window by a new `Entry.child_alive` flag, cleared by the real child `Exited` event, not by `status == Exited`. After a parser panic the status is `Exited` while the child still runs.
49. The input queue is capped by bytes as well as by chunks: `INPUT_QUEUE_BYTES = 1 MiB` (1 048 576). `write_input` rejects a single chunk larger than the cap with `input too large (N bytes, max 1 MiB)`. It rejects a chunk that would push the queued total over the cap with the existing `input queue full; the program is not reading input`. The writer thread subtracts a chunk's length only after `write_all` and `flush` return, so a chunk blocked in the PTY still counts. `INPUT_QUEUE_CAPACITY = 256` chunks stays.
50. A caught parser panic sends a new `WindowEvent::ParserPanicked(String)` instead of `Exited`. The manager sets `exit = ExitInfo { code: None, reason: "screen parser panicked: <msg>" }` and status `Exited`, publishes, and starts the kill escalation. When the real `Exited` arrives later, the manager keeps the first `exit` and only clears `child_alive`. In general, once `exit` is set it is never overwritten.

### Client

51. The client strips `ESC [ 2 0 1 ~` and `ESC [ 2 0 0 ~` from text that crossterm has already delivered as one paste event, before encoding it and, when bracketed-paste mode is enabled, wrapping it in bracketed-paste markers. This is a pure sanitization boundary for a delivered paste event, in both bracketed and unbracketed mode; it is not an end-to-end clipboard-boundary guarantee. Crossterm can split terminal input at an embedded end marker before the helper runs, after which the intended clipboard boundary is unavailable to the client. A reliable terminal-input policy is deferred to M7 rather than approximated here with timing filters or changed key semantics.
52. A sidebar card gets a third line when `status == Working` and `tool` is `Some`: two spaces and the tool name, muted, cut with `…` to the sidebar's inner width. Cards now have a height of 2 or 3 rows. `sidebar::card_height(&WindowInfo) -> u16` replaces the `CARD_HEIGHT` constant, and rendering and `hit_test` share one row-layout helper.

    ```
    ╭ anthrex ───────────────────╮
    │▎⠹ 1 api-worker             │
    │▎  claude · working · 12s   │
    │▎  Bash                     │
    │ ◆ 2 tests                  │
    │   codex · attention · 1m   │
    ```

53. `anthrex ls --json` prints the window list as pretty JSON in the `WindowInfo` serde shape. It is how the manual check and tests inspect sub-agents until milestone 4 draws them.

### Fake agent

54. Mode detection from argv: `--settings <json>` present means Claude mode. Any `-c notify=...` or `-c hooks....` present means Codex mode. Everything else is ignored, including `--name`, `--model`, `-m`, `-C`, and the prompt after `--`.
55. Claude hooks: `settings.hooks.<Event>[0].hooks[0].command`. Codex notify: the text after `notify=` parsed as a JSON array of strings. Codex hooks: for `-c hooks.<Event>=...` where `<Event>` has no `.`, the command is the TOML string after the first `command=`, read with serde_json's streaming string deserializer. `hooks.state...` flags are ignored.
56. A `hook` step runs its command with `/bin/sh -c <command>`, the payload JSON on stdin, and waits for it to exit with a 5-second timeout. A `notify` step runs the argv directly with the payload JSON appended as the last argument and stdin at `/dev/null`, and waits the same way. A step whose event has no registered command is skipped silently.
57. Payload filling: a `hook` payload gets `hook_event_name` = the step's event and `session_id` = `fake-session-<ANTHREX_WINDOW_ID>` (or `fake-session-<pid>` when unset) when absent, and `cwd` = the current directory when absent. A `notify` payload gets `type` = `agent-turn-complete` and `thread-id` = the same default session id when absent. Keys the script sets are never overwritten.
58. `title` writes `ESC ] 0 ; <text> ESC \`, terminated with ST and not BEL so that it never counts as a bell. `bell` writes `0x07`. `print` writes the text as given. `read_line` reads one line from stdin. `git_commit` writes the file, then runs `git add` and `git -c user.name=fake-agent -c user.email=fake-agent@example.invalid commit -m <message>`. `mcp_call` prints `fake-agent: mcp_call arrives in milestone 8` and exits 3.
59. Steps run in order, one at a time, with no delay between them except `wait_ms`. After the last step, unless it was `exit`, the fake agent reads stdin until EOF and then exits 0, so the window stays alive like an interactive agent.
60. `FAKE_AGENT_SCRIPT` unset or unreadable: the fake agent prints `fake-agent: no script (<reason>)` and waits for EOF. A line that does not parse: it prints `fake-agent: bad step on line N: <error>` and exits 2.
61. `--version` anywhere in argv prints `codex-cli 0.155.0` and exits 0 before anything else, so the daemon's startup version probe works against it.
62. When `FAKE_AGENT_ARGS_FILE` is set, the fake agent first writes its argv, without argv[0], as a JSON array to that file. Tests use it to assert the exact launch arguments end to end.

### Tests

63. End-to-end status tests live in `crates/cli/tests/`, where `CARGO_BIN_EXE_anthrex` is available. Each test starts its own real daemon, `anthrex daemon start --foreground`, as a child process with `ANTHREX_SOCKET` and `ANTHREX_DATA_DIR` in a fresh tempdir under `/tmp` (`tempfile::Builder::new().prefix("ax-").tempdir_in("/tmp")`, because macOS limits socket paths to 104 bytes), `ANTHREX_CLAUDE_BIN` and `ANTHREX_CODEX_BIN` set to the fake agent, and `FAKE_AGENT_SCRIPT` set to a script file in the tempdir. No test calls `std::env::set_var`.
64. The fake agent binary is found at `Path::new(env!("CARGO_BIN_EXE_anthrex")).with_file_name("fake-agent")`. `cargo test --workspace` builds it because `crates/fake-agent` has integration tests. If it is missing, the helper panics with `fake-agent not built; run cargo build -p anthrex-fake-agent`.

## Interfaces

### proto (`crates/proto`)

```rust
pub const PROTO_VERSION: u32 = 2;

/// New. One sub-agent inside a window's session. Product spec 4.4.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubagentInfo {
    pub id: String,
    pub parent_id: Option<String>,
    pub kind: String,
    pub label: Option<String>,
    pub model: Option<String>,
    pub state: SubagentState,
    pub tool: Option<String>,
    pub started_secs: u64,
    pub ended_secs: Option<u64>,
    pub needs_permission: bool,   // decision 45
}

/// New.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SubagentState { Running, Done, Failed }

/// Changed: `has_session: bool` is removed; `session_id`, `model`, `subagents` are new.
pub struct WindowInfo {
    pub id: u32,
    pub name: String,
    pub runtime: Runtime,
    pub cwd: PathBuf,
    pub branch: Option<String>,
    pub status: Status,
    pub tool: Option<String>,
    pub since_secs: u64,
    pub last_output_secs: u64,
    pub session_id: Option<String>,
    pub model: Option<String>,
    pub subagents: Vec<SubagentInfo>,
    pub exit: Option<ExitInfo>,
}

/// Changed: `CodexHook` is new. Serialized as "claude", "codex-notify", "codex-hook".
#[serde(rename_all = "kebab-case")]
pub enum HookSource { Claude, CodexNotify, CodexHook }
```

`lib.rs` re-exports `SubagentInfo` and `SubagentState`. The shape of `ClientMsg::HookEvent { window_id, source, payload }` does not change. The daemon now replies to it with `DaemonMsg::Ack { request: "hook" }`.

### daemon (`crates/daemon`)

```rust
// status.rs
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusEvent {
    Output, Quiet, Bell, Focused, InputSent, Exited,            // existing
    SessionStart, UserPromptSubmit, PreToolUse, PostToolUse,    // new
    PermissionRequest, PermissionPrompt, IdlePrompt, Stop,      // new
    CodexNotify,                                                // new
    Title(CodexTitle),                                          // new
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodexTitle { Starting, Working, Thinking, Waiting, Ready }
impl CodexTitle { pub fn parse(title: &str) -> Option<Self>; } // exact word after trim, case-sensitive
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StatusContext { pub focused: bool, pub signals_seen: bool, pub hooks_seen: bool }
/// (doc comment required: table of decision 28, purity, where the side effects live)
pub fn next(current: Status, event: StatusEvent, runtime: Runtime, ctx: StatusContext) -> Status;

// hooks.rs
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookKind {
    SessionStart, UserPromptSubmit, PreToolUse, PostToolUse, PermissionRequest,
    Notification, Stop, SubagentStart, SubagentStop, SessionEnd,
    TurnComplete, // codex-notify "agent-turn-complete"
}
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedHook {
    pub source: HookSource,
    pub kind: HookKind,
    pub session_id: Option<String>,        // "session_id", or "thread-id" for notify
    pub agent_id: Option<String>,
    pub agent_type: Option<String>,
    pub tool_name: Option<String>,
    pub tool_input: Option<serde_json::Value>,
    pub notification_type: Option<String>,
}
pub fn accepts(runtime: Runtime, source: HookSource) -> bool;
pub fn parse(source: HookSource, payload: &serde_json::Value) -> Option<ParsedHook>;
impl ParsedHook { pub fn status_event(&self) -> Option<StatusEvent>; }

// subagents.rs
pub const SUBAGENT_RETENTION: Duration = Duration::from_secs(300);
pub const MAX_SUBAGENTS: usize = 50;
pub const MAX_PENDING_SPAWNS: usize = 50;
pub const LABEL_MAX_CHARS: usize = 60;
pub const CLAUDE_SPAWN_TOOL: &str = "Agent";
pub const CODEX_SPAWN_TOOL: &str = "spawn_agent";
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingSpawn { pub parent_id: Option<String>, pub subagent_type: Option<String>, pub label: Option<String>, pub model: Option<String> }
pub fn spawn_request(runtime: Runtime, hook: &ParsedHook) -> Option<PendingSpawn>;
pub fn make_label(name: Option<&str>, prompt: Option<&str>) -> Option<String>;
#[derive(Debug, Default)]
pub struct SubagentTracker { /* entries, pending */ }
impl SubagentTracker {
    pub fn apply(&mut self, runtime: Runtime, hook: &ParsedHook, now: Instant) -> bool; // true if anything changed
    pub fn child_exited(&mut self, now: Instant) -> bool;
    pub fn input_sent(&mut self) -> bool;         // clears needs_permission
    pub fn prune(&mut self, now: Instant) -> bool;
    pub fn infos(&self, now: Instant) -> Vec<SubagentInfo>;
}

// agent_state.rs
#[derive(Debug, Default)]
pub struct AgentState {
    pub session_id: Option<String>,
    pub tool: Option<String>,
    pub signals_seen: bool,
    pub hooks_seen: bool,
    pub subagents: SubagentTracker,
    last_title: Option<String>,
}
pub struct HookOutcome { pub status_event: Option<StatusEvent>, pub changed: bool }
impl AgentState {
    pub fn context(&self, focused: bool) -> StatusContext;
    pub fn on_hook(&mut self, runtime: Runtime, hook: &ParsedHook, now: Instant) -> HookOutcome;
    pub fn on_title(&mut self, runtime: Runtime, title: &str) -> Option<StatusEvent>;
}

// launch/mod.rs
pub struct LaunchContext<'a> {
    pub window_id: u32,
    pub name: &'a str,
    pub socket_path: &'a Path,
    pub shell: &'a str,
    pub exe: &'a Path,                        // new
    pub claude_bin: &'a str,                  // new
    pub codex_bin: &'a str,                   // new
    pub codex_hook_source: Option<&'a str>,   // new; None = no Codex lifecycle hooks
}
pub fn plan(spec: &WindowSpec, ctx: &LaunchContext<'_>) -> LaunchPlan;
pub fn shell_quote(s: &str) -> String;
pub fn hook_command(exe: &Path, window_id: u32, source: HookSource) -> String;
// launch/claude.rs
pub fn settings(exe: &Path, window_id: u32) -> serde_json::Value;
pub const HOOK_EVENTS: [&str; 10];
// launch/codex.rs
pub const HOOK_EVENTS: [(&str, &str); 8];   // (event, snake label), decision 21
pub fn args(spec: &WindowSpec, ctx: &LaunchContext<'_>) -> Vec<String>;
pub fn toml_string(s: &str) -> String;
pub fn canonical_json(v: &serde_json::Value) -> String;
pub fn trust_key(source: &str, label: &str, group: usize, handler: usize) -> String;
pub fn trust_hash(label: &str, command: &str) -> String;
pub fn default_hook_source() -> Option<String>;
pub fn parse_version(output: &str) -> Option<(u64, u64, u64)>;
pub const MIN_CODEX_VERSION: (u64, u64, u64) = (0, 135, 0);

// window.rs
pub const INPUT_QUEUE_BYTES: usize = 1 << 20;
pub enum WindowEvent { Output, Bell, Title(String), Exited { code: Option<i32>, signal: Option<String> }, ParserPanicked(String) }
impl Window { pub fn signal_group(&self, sig: i32) -> anyhow::Result<()>; }

// manager.rs
pub const HUP_GRACE: Duration = Duration::from_secs(1);
pub const KILL_GRACE: Duration = Duration::from_secs(3);   // now measured from SIGHUP
#[derive(Debug, Clone)]
pub struct ManagerConfig {
    pub socket_path: PathBuf,
    pub shell: String,
    pub exe: PathBuf,
    pub claude_bin: String,
    pub codex_bin: String,
    pub codex_hook_source: Option<String>,
}
impl ManagerConfig {
    /// exe = "anthrex", bins "claude"/"codex", codex_hook_source None.
    pub fn new(socket_path: PathBuf, shell: String) -> Self;
    /// Pure: reads ANTHREX_CLAUDE_BIN / ANTHREX_CODEX_BIN through `var`.
    pub fn from_vars(socket_path: PathBuf, shell: String, exe: PathBuf, var: impl Fn(&str) -> Option<String>) -> Self;
    /// `from_vars` with `std::env::var`, `current_exe()`, and `codex::default_hook_source()`.
    pub fn from_env(socket_path: PathBuf, shell: String) -> anyhow::Result<Self>;
}
impl WindowManager {
    pub fn new(config: ManagerConfig) -> (Arc<Self>, mpsc::UnboundedReceiver<(u32, WindowEvent)>); // changed signature
    pub fn handle_hook(&self, id: u32, source: HookSource, payload: &serde_json::Value) -> anyhow::Result<()>;
    pub fn unfocus(&self, id: u32);
    pub fn child_pid(&self, id: u32) -> anyhow::Result<Option<u32>>;
}
```

### CLI

```
anthrex hook --window <id> --source <claude|codex-notify|codex-hook> [payload]   (hidden; decisions 8 to 13)
anthrex ls [--json]                                                               (--json is new)
```

### Environment variables

| Variable | Read by | Meaning |
|----------|---------|---------|
| `ANTHREX_CLAUDE_BIN` | daemon | Program for Claude windows. Default `claude`. |
| `ANTHREX_CODEX_BIN` | daemon | Program for Codex windows and the version probe. Default `codex`. |
| `FAKE_AGENT_SCRIPT` | fake-agent | Path of the JSON-lines script (product spec 12). |
| `FAKE_AGENT_ARGS_FILE` | fake-agent | If set, argv is written there as JSON (decision 62). |

### Fake-agent script steps

One JSON object per line, as in product spec 12: `print`, `hook` (+ `payload`), `notify`, `title`, `bell`, `wait_ms`, `read_line`, `mcp_call` (decision 58), `git_commit`, `exit`. Blank lines are skipped. This milestone owns the base script format and these ten steps. Milestone 8 implements `mcp_call` and adds `read_message`, `sh` and per-role scripts; milestone 9 adds `mcp_wait`, `expect` and `FAKE_AGENT_LOG` (product spec 12 lists every step with its owner).

## Tasks

### M3.1 Process-group kill, input byte cap, parser-panic exit

**Files.** Modify `crates/daemon/src/window.rs`, `crates/daemon/src/manager.rs`, `crates/daemon/tests/window.rs`, `crates/daemon/tests/manager.rs`, `AGENTS.md`.

**Tests first.**
- `window.rs::signal_group_reaches_background_children`: spawn `sh -c 'sleep 300 & echo bg=$!; wait'`. Wait until the screen shows `bg=<pid>` and parse the pid. Call `signal_group(libc::SIGTERM)`. Assert an `Exited` event arrives, and that within 3 s `libc::kill(pid, 0)` returns -1 with `ESRCH`.
- `window.rs::input_queue_is_capped_by_bytes`: spawn `sh -c 'stty raw -echo; sleep 5'` and sleep 300 ms. Two `write_input` calls of 512 KiB succeed. A third fails with a message containing `input queue full`. A single `write_input` of `INPUT_QUEUE_BYTES + 1` bytes fails with a message containing `too large`. End with `signal_group(SIGKILL)`.
- `manager.rs::kill_ends_an_interactive_shell_within_a_second`: create a Shell window (`/bin/sh`), wait until it is not `Starting`, call `kill`, and assert it is `Exited` in under 1 s.
- `manager.rs::shutdown_ends_every_window`: add the assertion that `shutdown().await` returns in under 1.5 s.
- `manager.rs::a_parser_panic_ends_the_child_and_keeps_its_reason`: create a Shell window, take `child_pid`, subscribe to `watch()`, and call `handle_event(id, WindowEvent::ParserPanicked("boom".into()))`. Assert the watch fired, the status is `Exited`, and `exit.reason == "screen parser panicked: boom"`. Within 3 s `kill(pid, 0)` gives `ESRCH`. After 300 ms more, the reason is unchanged.

**Change.** Decisions 46 to 50. Add `Window::signal_group`, the byte counter (an `Arc<AtomicUsize>` shared with the writer thread), `WindowEvent::ParserPanicked`, `Entry.child_alive`, `HUP_GRACE`, the escalation helper used by `kill`, `shutdown` and the panic path, and `child_pid`. Keep `Window::signal` for the existing tests. `handle_event(Exited)` never overwrites an existing `exit`. In `AGENTS.md`, replace the sentence about tests taking about 3 seconds, and the milestone-1 fact about interactive `sh` and the 3-second grace, with one sentence: kill signals the process group with SIGHUP first, so an interactive shell exits at once.

**Acceptance.** The new tests pass. `kill_terminates_and_remove_forgets`, `shutdown_ends_every_window` and `server.rs::create_subscribe_input_and_kill_flow` each finish in under 1.5 s. No test in the workspace waits for the SIGKILL escalation any more.

### M3.2 Protocol version 2

**Files.** Modify `crates/proto/src/types.rs`, `crates/proto/src/messages.rs`, `crates/proto/src/lib.rs`, `crates/daemon/src/manager.rs` (`Entry::info`), `crates/daemon/tests/server.rs`, and the `WindowInfo` test helpers in `crates/cli/src/client.rs`, `crates/tui/src/app.rs` and `crates/tui/src/ui/mod.rs`.

**Tests first.**
- `types.rs::window_info_round_trips_through_json`: now with `session_id: Some("s1")`, `model: Some("opus")`, and one `SubagentInfo` with every optional field set. Assert that the JSON contains `"state":"running"` and has no `has_session`.
- `types.rs::subagent_state_serializes_lowercase`: `Running`, `Done` and `Failed` serialize as `"running"`, `"done"` and `"failed"`.
- `messages.rs::hook_source_serializes_kebab_case`: JSON of the three variants is `"claude"`, `"codex-notify"`, `"codex-hook"`.
- `messages.rs::every_message_round_trips` replaces `every_daemon_message_round_trips`: every `ClientMsg` variant (all 13) and every `DaemonMsg` variant (all 8) survive `rmp_serde` named encoding. `HookEvent` uses `CodexHook`, and `WindowsChanged` carries a window with a sub-agent.
- `server.rs::a_version_1_client_is_rejected`: `Hello { proto_version: 1 }` gets `Error { request: "hello" }` with `anthrex daemon stop` in the message.

**Change.** The interfaces under "proto". `Entry::info` fills `session_id` from the entry's existing `session_id`, `model` from `spec.model`, and `subagents: vec![]` for now.

**Acceptance.** The workspace builds and every test passes with `PROTO_VERSION == 2`. `grep -rn has_session crates` finds nothing.

### M3.3 The fake agent

**Files.** Create `crates/fake-agent/Cargo.toml` (deps: `serde`, `serde_json`, `anyhow`, `libc`; dev-deps: `tempfile`), `crates/fake-agent/src/main.rs`, `crates/fake-agent/src/script.rs` (the `Step` enum and parsing), `crates/fake-agent/src/runtime.rs` (argv scanning and hook discovery, decisions 54 and 55), `crates/fake-agent/tests/script.rs`. Modify the root `Cargo.toml` `members`.

**Tests first.**
- Unit, `script.rs`: `parses_every_step_kind` (one line of each of the ten kinds); `bad_step_reports_its_line` (the error names line 3 for a bad third line).
- Unit, `runtime.rs`: `finds_claude_hook_commands_in_settings` (argv from `--name x --settings <the decision-18 JSON>` gives the `PreToolUse` command); `finds_codex_notify_and_hook_commands_in_config_overrides` (argv with the decision-19 notify flag and a decision-21 `hooks.PreToolUse` flag whose command contains `'`, a space and `"` gives the exact notify argv and command; `hooks.state...` flags are ignored); `unknown_flags_are_ignored`.
- Integration, `tests/script.rs`, running `env!("CARGO_BIN_EXE_fake-agent")` with piped stdio:
  - `prints_and_exits_with_the_scripted_code`: a `print` and then `exit 4` give that stdout and code 4.
  - `runs_the_claude_hook_with_the_payload_on_stdin`: the settings command is `cat > <tmp>/p.json`, `ANTHREX_WINDOW_ID=9`, and the step is `{"hook":"PreToolUse","payload":{"tool_name":"Bash"}}`. The file parses to an object with `hook_event_name == "PreToolUse"`, `session_id == "fake-session-9"` and `tool_name == "Bash"`.
  - `script_keys_are_not_overwritten`: a payload with `session_id: "mine"` keeps it.
  - `runs_codex_notify_with_the_payload_as_last_argument`: notify is `["/bin/sh","-c","printf '%s' \"$0\" > <tmp>/n.json"]`. The file has `type == "agent-turn-complete"` and `thread-id == "fake-session-9"`.
  - `runs_codex_hook_commands_from_config_overrides`: the same as the Claude case, through a `-c hooks.Stop=...` flag.
  - `steps_without_a_registered_hook_are_skipped`: exit code 0 and no error text.
  - `title_and_bell_write_their_bytes`: stdout contains `\x1b]0;Working\x1b\\` and then `\x07`.
  - `read_line_waits_for_input`: `read_line` then `print "after"`. `after` is absent before a line is written to stdin and present afterwards.
  - `writes_its_arguments_when_asked`: `FAKE_AGENT_ARGS_FILE` holds argv as JSON.
  - `version_prints_a_codex_style_version`.
  - `git_commit_creates_a_commit`: in a `git init` tempdir, `git log --oneline` shows the message.
  - `mcp_call_is_not_supported_yet`: exit code 3.

**Change.** Decisions 54 to 62. Keep `main.rs` small: parse argv, load the script, run the steps.

**Acceptance.** `cargo test -p anthrex-fake-agent` passes, and `target/debug/fake-agent` exists after `cargo test --workspace`.

### M3.4 Launchers and `ManagerConfig`

**Files.** Delete `crates/daemon/src/launch.rs`. Create `crates/daemon/src/launch/mod.rs` and `crates/daemon/src/launch/claude.rs`. Modify `crates/daemon/src/manager.rs`, `crates/daemon/src/lifecycle.rs`, `crates/daemon/Cargo.toml` (add `serde_json`), and the `WindowManager::new` call sites in `crates/daemon/tests/manager.rs`, `crates/daemon/tests/server.rs` and `crates/tui/tests/connection.rs`.

**Tests first**, in `launch/mod.rs` and `launch/claude.rs`, keeping the existing launch tests adapted to the new context:
- `shell_quote_wraps_and_escapes_single_quotes`: `/a b/x` gives `'/a b/x'`, and `/it's/x` gives `'/it'\''s/x'`.
- `hook_command_quotes_the_exe`: `hook_command(Path::new("/opt/anthrex/bin/anthrex"), 4, HookSource::Claude)` equals `'/opt/anthrex/bin/anthrex' hook --window 4 --source claude`.
- `claude_settings_json_is_exact`: `settings(...)` equals the `json!` value of decision 18, and the `--settings` argument string equals `serde_json::to_string` of that value, compact with sorted keys.
- `claude_gets_name_settings_model_and_prompt_in_order`: the args are `["--name","api","--settings",<json>,"--model","opus","--","fix the tests"]`.
- `programs_come_from_the_context`: `claude_bin` and `codex_bin` are used as `program`.
- `manager.rs` (unit, in `manager.rs`): `bin_overrides_come_from_the_environment_variables`. `from_vars` with a closure map gives the overrides; an empty string and absence give the defaults.

**Change.** Decisions 5, 15 to 18, 20. `lifecycle::run` builds `ManagerConfig::from_env`. Codex argv stays as milestone 1 built it until M3.10.

**Acceptance.** All tests pass. `claude` windows launched by the real daemon carry `--settings`.

### M3.5 Hook parsing and the status engine

**Files.** Create `crates/daemon/src/hooks.rs`. Modify `crates/daemon/src/status.rs`, `crates/daemon/src/lib.rs`, and the `Entry::apply` call in `crates/daemon/src/manager.rs`, which passes `StatusContext { focused: false, signals_seen: false, hooks_seen: false }` until M3.6.

**Tests first.**
- `hooks.rs`: `parses_a_claude_pre_tool_use_payload` (every field of `ParsedHook`); `parses_the_notification_type`; `sub_agent_fields_are_read` (`agent_id`, `agent_type`); `unknown_events_and_non_objects_are_none`; `missing_fields_degrade_to_none` (a `Stop` with only `hook_event_name` parses); `status_event_mapping` (each `HookKind` maps as in decision 28: `Notification` with `permission_prompt` gives `PermissionPrompt`, with `idle_prompt` gives `IdlePrompt`, with anything else gives `None`; `SubagentStart`, `SubagentStop` and `SessionEnd` give `None`); `source_acceptance` (decision 27 for all nine runtime and source pairs). In this task `parse` returns `None` for the two Codex sources. M3.10 adds them.
- `status.rs`: one test per row of decision 28 that does not involve Codex. Names: `session_start_makes_a_fresh_window_idle` (from `Starting` and from fallback `Working`; `Attention` stays), `session_start_after_signals_changes_nothing`, `prompt_submit_and_pre_tool_use_mean_working`, `pre_tool_use_keeps_attention`, `post_tool_use_changes_no_status`, `permission_request_and_prompt_mean_attention`, `idle_prompt_keeps_done_and_attention`, `stop_is_done_unless_viewed`, `bell_is_ignored_by_a_claude_window_with_hooks` (also: still `Attention` without hooks, and for Shell), `output_and_quiet_drive_agents_only_until_the_first_signal`, `hook_events_do_not_move_shell_windows`. The existing five tests get the new argument.

**Change.** Decisions 4, 27, 28, 29 to 31. Write the `next` doc comment: what it is, that it is pure, where session id, tool and sub-agents are handled, and a pointer to decision 28 and core spec 3.4. This closes the follow-up.

**Acceptance.** All tests pass. `status.rs` has no `_runtime` parameter.

### M3.6 The daemon applies hooks and tracks viewers

**Files.** Create `crates/daemon/src/agent_state.rs` and `crates/daemon/tests/agent_hooks.rs`. Modify `crates/daemon/src/manager.rs`, `crates/daemon/src/server.rs`, `crates/daemon/src/lib.rs` and `crates/daemon/tests/server.rs`.

**Tests first.** `agent_hooks.rs` uses a stub agent: the test writes `<tmp>/stub.sh` with `#!/bin/sh` and `exec sleep 300`, or with an output loop where noted, makes it executable, and sets it as `claude_bin` in `ManagerConfig`. Payloads go through `handle_hook` directly.
- `claude_hooks_drive_status_tool_and_session`: `SessionStart {session_id:"s1"}` gives `Idle` and `session_id == Some("s1")`. `UserPromptSubmit` gives `Working`. `PreToolUse {tool_name:"Bash"}` sets `tool == Some("Bash")`. `PostToolUse` sets `tool == None`. `Stop` gives `Done`.
- `stop_on_a_viewed_window_is_idle`: `focus(id)`, `Stop` gives `Idle`. `unfocus(id)`, then `UserPromptSubmit` and `Stop` give `Done`.
- `sub_agent_tool_events_do_not_touch_the_window_tool`: a `PreToolUse` with `agent_id` leaves `tool == None`.
- `the_first_hook_disables_the_output_fallback`: the stub prints `tick` every 200 ms. The window becomes `Working` by fallback. `SessionStart` gives `Idle`. After 800 ms it is still `Idle`.
- `unknown_window_is_an_error`, `wrong_source_is_ignored` (a `codex-notify` payload to a Claude window changes nothing and returns `Ok`), `unparseable_payload_is_ignored`.
- `bell_after_hooks_does_not_raise_attention`: `handle_event(id, WindowEvent::Bell)` after `SessionStart` leaves `Idle`.
- `server.rs::hook_events_are_acknowledged`: `HookEvent` for a Shell window gets `Ack { request: "hook" }`, and one for id 99 gets `Error { request: "hook" }`.
- `server.rs::a_subscription_marks_the_window_viewed_until_the_client_leaves`: a stub Claude window. Client A subscribes. Client B sends `Stop` and the status is `Idle`. A disconnects. B sends `UserPromptSubmit` and `Stop` and the status is `Done`.

**Change.** Decisions 3, 14, 25, 26, 34, 35, 37. `handle_hook` order: find the entry; check `accepts`; `parse`; `ctx = state.context(viewers > 0)`; `outcome = state.on_hook(...)`; apply `outcome.status_event` through `next`; publish if the status or `outcome.changed` changed anything. `InputSent` also calls `state.subagents.input_sent()` (a no-op until M3.7). The server replaces the `HookEvent` arm and tracks the subscribed window id for `unfocus`. Payloads that fail to parse are logged at debug level, cut to 2 KB.

**Acceptance.** All tests pass. The server no longer answers `HookEvent` with `Error` for a known window.

### M3.7 `anthrex hook`, `ls --json`, and Claude end-to-end tests

**Files.** Create `crates/cli/src/hook.rs`, `crates/cli/tests/support/mod.rs`, `crates/cli/tests/hook_command.rs`, `crates/cli/tests/claude_status.rs`. Modify `crates/cli/src/main.rs` and `crates/cli/Cargo.toml` (add `serde_json`; dev-deps `tempfile`, `tokio`).

**Support harness** (`support/mod.rs`): `fake_agent_bin()` (decision 64). `TestDaemon::start(script: &[serde_json::Value])` writes the script, spawns the daemon as in decision 63, and waits up to 3 s for the socket. `TestDaemon::client()` does the handshake with `PROTO_VERSION`. `TestDaemon::anthrex(args: &[&str]) -> std::process::Output` runs the real binary with the daemon's `ANTHREX_SOCKET` and `ANTHREX_DATA_DIR`. `TestDaemon::data_dir() -> &Path`. `Client::create(runtime, name)` returns the id. `Client::wait_window(id, what, pred)` polls `ListWindows` every 25 ms with an 8 s deadline. `Client::subscribe(id)`. `Drop for TestDaemon` sends `Shutdown`, waits up to 5 s for the process, then kills it.

**Tests first.**
- `hook_command.rs::without_a_daemon_it_exits_zero_silently_and_fast`: `ANTHREX_SOCKET` points to a missing path and the payload is an argument. Exit 0, empty stdout and stderr, under 1 s.
- `hook_command.rs::a_silent_daemon_costs_at_most_a_second`: a test `UnixListener` accepts and never replies. Exit 0 in under 1.5 s.
- `hook_command.rs::bad_arguments_exit_zero`: `anthrex hook --window nope` and `anthrex hook` alone. Exit 0 and no output.
- `hook_command.rs::payload_from_stdin_and_from_the_last_argument`: a `TestDaemon` with a one-step script `{"read_line":true}` and one Claude window. `anthrex hook --window <id> --source claude` with a `SessionStart {session_id:"x1"}` payload on stdin sets `session_id`. With a `UserPromptSubmit` payload as the last argument, the status becomes `Working`.
- `claude_status.rs::a_turn_reports_working_the_tool_and_done`. Script: `SessionStart`, `read_line`, `UserPromptSubmit`, `PreToolUse {tool_name:"Bash"}`, `read_line`, `PostToolUse`, `Stop`. Assert `Idle` and `session_id == "fake-session-1"`. Send `go\r` and assert `Working` with tool `Bash`. Send `go\r` and assert `Done` with tool `None`.
- `claude_status.rs::a_viewed_window_ends_idle`: the same script, with the test subscribed. The final status is `Idle`.
- `claude_status.rs::permission_request_waits_for_input`: `SessionStart`, `PermissionRequest`, `read_line`. Assert `Attention`. Send `y\r` and assert `Working`.
- `claude_status.rs::idle_prompt_and_bell_after_hooks`: `SessionStart`, `Notification {notification_type:"idle_prompt"}`, `bell`. The status stays `Idle` for 500 ms after the bell.
- `claude_status.rs::fallback_until_the_first_hook`: `print "booting"`, `wait_ms 3500`, `hook SessionStart`, `print "more"`. It becomes `Working`, then `Idle` by `Quiet`, then stays `Idle` after `SessionStart` and the later output.
- `claude_status.rs::launch_arguments_reach_the_agent`: `FAKE_AGENT_ARGS_FILE` is set and the window is created with model `opus` and prompt `-x`. The args start with `--name`, contain `--settings` whose `SessionStart` command is `hook_command(<target/debug/anthrex>, id, Claude)`, and end with `--`, `-x`.
- `claude_status.rs::ls_json_lists_session_and_model`: `anthrex ls --json` against the test daemon parses as `Vec<WindowInfo>` with the session id and the model.

**Change.** Decisions 8 to 13 and 53. Add `--json` to `Command::Ls`.

**Acceptance.** All tests pass. `anthrex hook` never takes longer than 1 s and never exits non-zero in any test.

### M3.8 Sub-agent tracking

**Files.** Create `crates/daemon/src/subagents.rs` and `crates/cli/tests/claude_subagents.rs`. Modify `crates/daemon/src/agent_state.rs`, `crates/daemon/src/manager.rs` (`info`, `tick`, child exit, `InputSent`) and `crates/daemon/src/lib.rs`.

**Tests first.**
- `subagents.rs` unit tests, with `now` values built from one base `Instant`:
  - `start_and_stop_pair_by_agent_id`;
  - `the_oldest_matching_pending_spawn_wins` (two `Agent` calls of type `Explore` labelled `a` and `b`, one of type `Plan`: a `Plan` start takes the third, and the next two `Explore` starts take `a` then `b`);
  - `an_untyped_pending_spawn_matches_any_type`;
  - `no_match_means_the_session_is_the_parent`;
  - `a_spawn_from_inside_a_sub_agent_sets_the_parent`;
  - `label_prefers_name_then_first_prompt_line_and_is_cut_at_60` (a 70-character first line becomes 59 characters plus `…`; multibyte characters count as one);
  - `tool_is_set_by_pre_and_cleared_by_post_tool_use`;
  - `permission_request_marks_the_asking_sub_agent` (cleared by `PreToolUse` for it and by `input_sent`);
  - `child_exit_fails_running_entries`;
  - `finished_entries_expire_after_300_seconds` (present at 299 s, gone at 301 s);
  - `at_most_50_entries_oldest_finished_first`;
  - `pending_spawns_are_capped_and_expire`;
  - `duplicate_start_and_unknown_stop_are_ignored`;
  - `infos_report_seconds_since_start_and_end`.
- `claude_subagents.rs`, through `TestDaemon` and `ListWindows`:
  - `the_agent_tool_names_its_sub_agent`: the script sends `PreToolUse {tool_name:"Agent", tool_input:{subagent_type:"Explore", name:"scout", model:"haiku", prompt:"look"}}`, `SubagentStart {agent_id:"a1", agent_type:"Explore"}`, `PreToolUse {agent_id:"a1", tool_name:"Grep"}`, `read_line`, `PostToolUse {agent_id:"a1"}`, `SubagentStop {agent_id:"a1", agent_type:"Explore"}`. Assert the entry `{id:"a1", parent_id:None, kind:"Explore", label:"scout", model:"haiku", state:Running, tool:"Grep"}`, then `Done` with tool `None` after `go\r`.
  - `nested_sub_agents_record_their_parent`: an `Agent` call from inside `a1` (the payload carries `agent_id:"a1"`), then `SubagentStart a2`. `a2.parent_id == Some("a1")`.
  - `running_sub_agents_fail_when_the_agent_exits`: `SubagentStart a1` then `exit 1`. The window is `Exited` and `a1` is `Failed`.

**Change.** Decisions 38 to 45 for Claude. `spawn_request` returns `None` for Codex until M3.10 wires it.

**Acceptance.** All tests pass, and `WindowInfo.subagents` is filled for Claude windows.

### M3.9 Codex verification: flags, hook trust, titles

This is the first task that touches Codex, as product spec 11.1 and 11.3 require. It changes little code. Its main output is facts, recorded under "Implementation notes".

**Files.** Create `crates/daemon/src/launch/codex.rs` with `toml_string`, `canonical_json`, `trust_key`, `trust_hash`, `parse_version`, `HOOK_EVENTS` and `default_hook_source`. Add `sha2 = "0.10"` to the workspace and daemon dependencies; 0.10.9 is already in `Cargo.lock`. Modify this brief's "Implementation notes".

**Tests first** (unit, `codex.rs`):
- `canonical_json_sorts_keys_recursively_and_is_compact`.
- `trust_hash_matches_the_test_vectors`: the two vectors of decision 22, and the canonical text of the first.
- `trust_key_joins_its_parts`: `trust_key("/h/config.toml","pre_tool_use",0,0) == "/h/config.toml:pre_tool_use:0:0"`.
- `toml_string_escapes_quotes_and_backslashes`: also a newline, a tab, U+0001 and U+007F give `\n`, `\t`, `` and ``, and `é` is copied.
- `parse_version_reads_codex_cli_output`: `codex-cli 0.155.0` gives `(0,155,0)`, garbage gives `None`, and `0.134.9 < MIN_CODEX_VERSION`.

**Verification steps.** Record every result, with the exact command, under "Implementation notes". Never write trust entries into the user's real Codex configuration. First check whether the installed Codex supports a separate home directory. Look for `CODEX_HOME` in `codex --help`, the docs or the source; this is not verified. If it does, work in a temporary home. If it needs a login, copy only the auth file into it with mode 0600 and delete the directory at the end. If it does not, back up `~/.codex/config.toml` and restore it byte for byte afterwards.
1. Record `codex --version`. Record whether `codex --help` still lists `-C`, `-c`, `-m`, `-s` and `-a`. Record whether `codex features list` shows the hooks feature and `multi_agent` enabled (product spec 11.4).
2. Record the exact terminal titles Codex sets with `-c 'tui.terminal_title=["status"]'`. Run it under the anthrex daemon from M3.4 with `ANTHREX_LOG=debug`: the manager logs every `window title`. Confirm the five words of product spec 11.2 arrive as the whole title. If they carry a prefix, such as a spinner, change `CodexTitle::parse` in M3.10 to match and record it.
3. Write a logging hook script that appends its stdin, one line per call, to a file. Start Codex with `-c hooks.PreToolUse=[{hooks=[{type="command",command="<script>"}]}]` and without any trust entry. Record whether the startup review appears, whether it lists the hook, and the exact key and hash Codex stores when you trust it there. That stored pair is the ground truth for the key's source path (the `source` argument) and for the canonical form. If the source for the installed tag is reachable (`github.com/openai/codex`, tag `rust-v0.155.0`), read the hook trust code as well, and record the file and function.
4. Adjust `trust_key`'s source, `canonical_json`'s input shape, the `-c` key spelling (`hooks.PreToolUse` versus another form) and `default_hook_source()` until `trust_hash` reproduces the stored hash exactly. Update decision 21 and 22's test vectors in code and record the change.
5. Start Codex again with the hook and the `-c hooks.state.<quoted key>.trusted_hash="<hash>"` flag computed by anthrex. Confirm that no startup review appears and that the hook runs (the log file grows on a tool call). Confirm whether the payload arrives on stdin or as an argument, and that it has `hook_event_name` and `session_id`.
6. If the user's own configuration has hooks for the same event in `config.toml`, record whether the `-c` array replaces them or merges with them. Hooks from `hooks.json` are a separate source and should be unaffected; confirm that they still run without a review.

**Outcome.** Branch A: steps 3 to 5 work. `default_hook_source()` returns the verified source path, computed at call time from the environment exactly as Codex computes it. Branch B: they cannot be made to work in the time available, or the review prompt still appears. `default_hook_source()` returns `None` with a comment that points to the implementation notes, and M3.10 and M3.11 skip their branch-A parts. Either way the task ends with the workspace building and every test passing.

**Acceptance.** The facts above are recorded, the branch is named in "Implementation notes", and the unit tests pass.

### M3.10 Codex launch, Codex status rows, session id

**Files.** Modify `crates/daemon/src/launch/codex.rs`, `crates/daemon/src/launch/mod.rs`, `crates/daemon/src/hooks.rs`, `crates/daemon/src/status.rs`, `crates/daemon/src/agent_state.rs`, `crates/daemon/src/manager.rs` (the title event), `crates/daemon/src/lifecycle.rs` (the version probe). Create `crates/cli/tests/codex_status.rs`.

**Tests first.**
- `codex.rs::codex_args_without_hooks_are_exact`: exe `/opt/anthrex/bin/anthrex`, window 7, cwd `/tmp/repo`, model `gpt-5-codex`, prompt `hello`, `codex_hook_source: None`. The args are `["-C","/tmp/repo","-c","notify=[\"/opt/anthrex/bin/anthrex\",\"hook\",\"--window\",\"7\",\"--source\",\"codex-notify\"]","-c","tui.terminal_title=[\"status\"]","-c","tui.notifications=[\"approval-requested\"]","-c","tui.notification_method=\"bel\"","-c","tui.notification_condition=\"always\"","-m","gpt-5-codex","--","hello"]`.
- `codex.rs::codex_args_with_hooks_are_exact` (branch A): `codex_hook_source: Some("/tmp/codex-home/config.toml")`. The sixteen hook flags sit between the notify block and `-m`, in decision 21's order. The `SessionStart` pair is exactly `hooks.SessionStart=[{hooks=[{type="command",command="'/opt/anthrex/bin/anthrex' hook --window 7 --source codex-hook"}]}]` and `hooks.state."/tmp/codex-home/config.toml:session_start:0:0".trusted_hash="sha256:6c98ad65309ebdb9cfaa45a29c0fe280b05f9b7462e2cfc84189479028f9ba37"`, or the forms M3.9 established.
- `hooks.rs::parses_codex_notify` (`thread-id` becomes `session_id`, and `type` must be `agent-turn-complete`); `parses_codex_hook_payloads_like_claude_ones`.
- `status.rs`: one test per Codex row of decision 28: `codex_titles_without_hooks`, `ready_ends_a_working_turn_and_keeps_done`, `codex_notify_without_hooks`, `codex_notify_with_hooks_only_ends_a_working_turn`, `titles_after_codex_hooks_only_report_waiting_and_ready`, `unknown_titles_do_not_parse`.
- `agent_state.rs`: `codex_session_id_is_written_once` (a root `SessionStart` sets it, and a later `SessionStart` with a different id changes nothing); `notify_sets_the_session_when_no_hook_did`; `notify_from_another_thread_is_ignored`; `claude_session_id_follows_the_latest_session_start`; `repeated_titles_are_dropped`; `a_recognised_title_disables_the_fallback`.
- `codex_status.rs`, end to end with the fake agent as `ANTHREX_CODEX_BIN`:
  - `titles_and_notify_drive_a_codex_window`: `title Working` gives `Working`, `title Ready` gives `Done`, and `notify {thread-id:"t1"}` sets `session_id == "t1"`.
  - `waiting_title_and_bell_mean_attention`.
  - `lifecycle_hooks_drive_a_codex_window` (branch A; start the test daemon with a `codex_hook_source` by setting whatever environment `default_hook_source()` reads to a tempdir): `SessionStart {session_id:"root"}`, `UserPromptSubmit`, `PreToolUse {tool_name:"shell"}`, then `Stop`. Assert the statuses and that `session_id` stays `"root"` after a `SessionStart {session_id:"child"}` and a `notify {thread-id:"child"}`.
  - `launch_arguments_reach_the_codex_agent`: `FAKE_AGENT_ARGS_FILE` shows `-C <cwd>` first and the notify flag with the real exe path.

**Change.** Decisions 19, 21 to 24, 28 (Codex rows), 32, 33, 36. The title path in the manager: `WindowEvent::Title(t)` goes to `state.on_title(runtime, &t)`, which returns `None` for non-Codex runtimes, repeats and unknown titles (logged at debug level), and otherwise the event. The manager then applies it with the context taken before `on_title` updated `signals_seen`. `hooks.rs` now parses both Codex sources.

**Acceptance.** All tests pass. On branch B, the branch-A tests are not written, and "Implementation notes" says so.

### M3.11 Codex sub-agents (branch A only)

**Files.** Modify `crates/daemon/src/subagents.rs`. Create `crates/cli/tests/codex_subagents.rs`.

**Verification step first.** Run a real Codex window under the M3.10 daemon with `ANTHREX_LOG=debug`. Ask it to spawn a sub-agent, for example "use a sub-agent to list the files here". Record the full `PreToolUse` payload for `spawn_agent`, with any secrets cut, and the `SubagentStart` and `SubagentStop` payloads. Fill the constants `CODEX_SPAWN_TYPE_KEY`, `CODEX_SPAWN_LABEL_KEYS` (name first, then prompt-like field) and `CODEX_SPAWN_MODEL_KEY` with the field names seen, or leave a constant `None` when there is no such field. Record the result.

**Tests first.**
- `subagents.rs::codex_spawn_request_reads_the_verified_fields`: a `tool_input` shaped like the recorded one gives the expected `PendingSpawn`.
- `codex_subagents.rs::codex_sub_agents_pair_and_never_touch_the_session_id`: fake-agent Codex hooks send `SessionStart {session_id:"root"}`, `PreToolUse {tool_name:"spawn_agent", tool_input:<recorded shape>}`, `SubagentStart {agent_id:"c1", agent_type:<recorded>, session_id:"c1"}`, `PreToolUse {agent_id:"c1", session_id:"c1", tool_name:"shell"}`, `SubagentStop {agent_id:"c1", session_id:"c1"}`. Assert the entry pairs, its tool, `Done`, and `session_id == "root"` throughout.

**Change.** Decision 38 for Codex. If the verification cannot be done, for example because no account is available, leave the constants `None`, keep the label fallback to `agent_type`, write the end-to-end test with `tool_input: {}`, and record it.

**Acceptance.** All tests pass. Codex sub-agents appear in `anthrex ls --json`.

### M3.12 Client: tool line and paste sanitising

**Files.** Modify `crates/tui/src/ui/sidebar.rs`, `crates/tui/src/ui/mod.rs` (tests), `crates/tui/src/app.rs`.

**Tests first.**
- `app.rs::paste_strips_bracketed_paste_markers`: `a\x1b[201~b\x1b[200~c` with bracketed paste on gives `\x1b[200~abc\x1b[201~`, and with it off gives `abc`.
- `ui/mod.rs::a_working_card_shows_its_tool_on_a_third_line`: a Claude window, `Working`, tool `Bash`. The rendered sidebar has `Bash` on the row below `claude · working`. The same window `Idle` with a tool shows no third line.
- `ui/mod.rs::sidebar_hit_test_maps_rows_to_cards`, updated: with the first card three rows tall, rows 0 to 2 hit card 0 and rows 3 and 4 hit card 1.
- `ui/mod.rs::sidebar_hit_test_ignores_footer_and_undrawn_cards`, updated to use `card_height`, plus a case where a three-row card does not fully fit and so is not hit.
- `ui/mod.rs::long_tool_names_are_cut_to_the_sidebar`.

**Change.** Decisions 51 and 52. The toast code in `App::replace_windows` is not touched.

**Acceptance.** All tests pass. The sidebar still fits the 30-column width.

### M3.13 Smoke stage and documentation

**Files.** Modify `scripts/pty-smoke.py`, `AGENTS.md` (the "Where things are" table gains `crates/fake-agent`; one line under Commands says status tests use it through `ANTHREX_CLAUDE_BIN` and `ANTHREX_CODEX_BIN`), `docs/ROADMAP.md` (status, as `AGENTS.md` describes).

**Change.** `ensure_binary` also builds `target/debug/fake-agent` when it is missing. `ENV` gains `ANTHREX_CLAUDE_BIN` = that path and `FAKE_AGENT_SCRIPT` = `/tmp/anthrex-smoke-fake.jsonl`, which `reset_state` writes and the teardown removes. Add stage 8b after the paste stage:
1. `anthrex new --runtime claude --name fake-claude` through `run_cmd` while the client stays attached to another window.
2. The script is `SessionStart`, `UserPromptSubmit`, `PreToolUse {tool_name:"Bash"}`, `wait_ms 3000`, `PostToolUse`, `Stop`.
3. Wait for the screen to show `claude · working` and `Bash` on the fake-claude card.
4. Wait for the toast `fake-claude finished` and `claude · done`.
5. `anthrex ls --json` shows the window with `"status": "done"` and `"session_id": "fake-session-<id>"`.
6. `anthrex rm fake-claude`, so later stages see only the windows `shell-1` to `shell-4`.

**Acceptance.** `python3 scripts/pty-smoke.py` passes, with the new stage, and leaves no daemon running.

## Verification

Run the five commands from `AGENTS.md`:

```bash
cargo build --workspace --all-targets
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
python3 scripts/pty-smoke.py
```

Also check:

- `cargo test --workspace` runs no test that waits on the 3-second SIGKILL escalation. Compare the per-test times of `crates/daemon/tests/manager.rs` with milestone 1: the kill and shutdown tests drop from about 3 s to under 1.5 s.
- `grep -rn "has_session" crates` finds nothing, and `grep -rn "PROTO_VERSION: u32 = 2" crates/proto` finds the constant.
- `pgrep -fl "anthrex daemon"` and `pgrep -fl fake-agent` show nothing of yours after the test run.
- CI from milestone 2 passes on the pull request, on macOS and Ubuntu.

## Manual check

Use an isolated daemon for every step:

```bash
export ANTHREX_SOCKET=/tmp/anthrex-m3.sock ANTHREX_DATA_DIR=/tmp/anthrex-m3-data ANTHREX_LOG=debug
cargo build
```

1. Run `target/debug/anthrex` in one terminal. In a second terminal with the same variables, run `target/debug/anthrex new --runtime claude --name c1`. The card shows `starting`, then `idle` once Claude's `SessionStart` hook arrives. `anthrex ls --json` shows a `session_id`.
2. Focus `c1` and ask for something that runs a shell command. The card shows `working` and a third line `Bash`. When Claude asks for permission, the card shows `attention`. Answer it and the card returns to `working`.
3. Focus another window before the turn ends. When it ends, `c1` shows `done` with a green check and the toast `c1 finished`. Focusing it turns it `idle`.
4. Leave `c1` idle for more than a minute. It stays `idle` or `done` and never turns `attention` from the idle bell.
5. Ask Claude to use a sub-agent, for example "use the Explore agent to find where the socket path is computed". While it runs, `anthrex ls --json` shows a sub-agent with `kind` `Explore`, a label, `state` `running` and a changing `tool`. Afterwards it shows `done`.
6. Press Esc during a Claude turn to interrupt it. Record the status that follows. If it stays `working`, add a follow-up for milestone 4 to `docs/superpowers/plans/2026-09-17-anthrex-foundation-followups.md`.
7. `anthrex new --runtime codex --name x1`. Watch the card go `starting`, `working`, `done` over a turn. Trigger an approval, for example by running with `-a untrusted` through a prompt that needs a command. The card shows `attention`. Confirm that no hook review prompt appears at startup (branch A). Ask for a sub-agent and check `anthrex ls --json` (branch A).
8. `anthrex kill x1`. The window is `exited` within about a second. `pgrep -fl codex` shows no process left from that window.
9. In an input harness that can deliver one crossterm paste event containing `aESC[201~b`, pass that event through the client with bracketed-paste mode on and confirm the forwarded paste body is `ab`. This checks sanitization of text already delivered as a paste event only; it does not claim that a real terminal and crossterm preserve the clipboard boundary when the clipboard itself contains an end marker.
10. `target/debug/anthrex daemon stop`, then `pgrep -fl "anthrex daemon"` shows nothing of yours.

## Risks and gotchas

1. **Codex hook trust does not work through `-c`.** You will see this when the review prompt still appears in step 5 of M3.9, or when the hash never matches. Take branch B, as the product spec 11.3 decision allows. Codex status then comes from notify, the title and the bell, and Codex sub-agent rows stay empty.
2. **A `-c` hooks array may replace the user's own `config.toml` hooks for that event.** M3.9 step 6 checks this. If it does, record it in "Implementation notes" and in the follow-ups for milestone 6, where `runtimes.codex` config can offer a switch. Do not merge arrays by hand.
3. **Dotted `-c` keys with quotes.** The trust key contains `/`, `:` and possibly spaces. If Codex splits the quoted key wrongly, the review prompt appears. Try the inline-table form `hooks.state={...}` only if it does not drop the user's other trust entries; M3.9 must check this.
4. **Hook payloads change between versions** (product spec risk 1). Every field in `ParsedHook` except `kind` is optional. Unknown events give `None` and are logged at debug level. A window whose hooks never parse stays on the output fallback, which is the intended degradation.
5. **Ordering between hooks.** Without the Ack wait (decision 11), a fast `PostToolUse` can overtake its `PreToolUse` and leave a stale tool. If a test flakes on the tool line, check that `anthrex hook` waits for the Ack.
6. **The hook adds latency to every tool call.** It is one short process and one socket round trip. If the daemon is slow, the 1-second deadline caps it. Never add work that can block to `handle_hook`: it runs under the manager lock.
7. **`cargo test -p anthrex` alone does not build `fake-agent`.** The harness panics with the fix. CI and the documented command use `--workspace`.
8. **Process-group reuse.** After a group dies, its id could in theory be reused within the 3-second escalation. The escalation stops on the first `ESRCH` from `killpg(pid, 0)`, which makes this very unlikely. Do not remove that check.
9. **Interactive shells put background jobs in their own process group.** `killpg` does not reach them directly; the shell forwards SIGHUP to its jobs itself. The `signal_group_reaches_background_children` test therefore uses a non-interactive `sh -c`. Do not change it to an interactive shell.
10. **Claude turns interrupted with Esc** may fire no `Stop`. After the first hook the output fallback is off, so such a window can stay `Working`. This is not solved here. Manual check step 6 records what happens.
11. **vt100 and OSC terminators.** The fake agent ends titles with ST (`ESC \`). If a test sees a `Bell` event for a title, the fake agent is writing BEL.
12. **Hook commands inherit the agent's environment.** The launcher already sets `ANTHREX_SOCKET`, so `anthrex hook` reaches the right daemon. A test that forgets to isolate the socket would send hooks to the user's live daemon. Every test uses the harness in decision 63.
13. **`WindowManager::new` changes signature.** Every call site, in the daemon, the TUI tests and `lifecycle.rs`, must move to `ManagerConfig` in the same task (M3.4).

## Follow-ups handled

From `docs/superpowers/plans/2026-09-17-anthrex-foundation-followups.md`, "Assignment to milestones", row M3:

| Item | Task |
|------|------|
| Kill by process group: SIGHUP, then SIGTERM, then SIGKILL via `killpg` | M3.1 (decisions 46 to 48) |
| Cap the per-window input queue by bytes (1 MiB) as well as chunks | M3.1 (decision 49) |
| After a caught parser panic: signal the child, keep the first exit reason, and publish | M3.1 (decision 50) |
| Strip `ESC [ 201 ~` from pasted text | M3.12 (decision 51) |
| `status::next` doc comment | M3.5 |
| Task 2 test-coverage minors: every `ClientMsg` and `DaemonMsg` variant round-trips, and the serialized case of `HookSource` is asserted | M3.2 (`every_message_round_trips`, `hook_source_serializes_kebab_case`) |

## Implementation notes

Implementation starts from main commit `43dc604` after the user's explicit request
to merge PR #3, delete `m2-ci`, and continue. Both local and remote M2 branches
were deleted; the existing isolated worktree is reused on `m3-agent-status`.

- The M2 raw-mode readiness and isolated smoke-directory improvements supersede
  stale instructions to rely only on a fixed 300 ms delay or to write a shared
  `/tmp/anthrex-smoke-fake.jsonl`. The smoke script also no longer has
  `reset_state`, and stage 8 detaches its client; M3.13 will explicitly reattach
  for the new stage and put its script under the unique test data directory.
- M3.6 refers to `SubagentTracker` before M3.8 defines it (and calls M3.7 the
  tracker task). The field and its calls are integrated in M3.8 together, with
  M3.6 retaining the existing empty `subagents` list in the interim. Similarly,
  the M3.4 default Codex hook source remains `None` until M3.9 verifies it.
- Production behavior, test evidence, installed Codex observations, and final
  verification are recorded here as their tasks complete.

### M3.1 process safety

Commit `4723860` extracts the shared group signal/escalation code into private
`process.rs`. Byte reservations are RAII-owned by queued chunks, so a failed
enqueue or discarded queue also releases its reservation. The blocked writer
chunk continues to count until both its write and flush return.

Test corrections: the background-child shell traps TERM and reaps its child
(avoiding dependence on Linux PID 1); the input fixture waits for a raw-mode
READY marker and checks sustained rejection for 200 ms; the parser-panic test
waits for actual PID disappearance and explicitly delivers a subsequent exit
event instead of using a fixed delay to assume waiter delivery. The existing
backpressure regression now retries its 4 KiB chunk rather than an empty one:
an empty chunk does not detect the new byte limit. The server flow directly
asserts the 1.5-second acceptance bound.

All new behaviors were observed failing before implementation. Afterward,
`cargo test -p anthrex-daemon --tests` passed 43 tests (16 unit, 12 manager,
5 server, 10 window); daemon clippy and workspace formatting passed. The
non-reading-child test confirmed sustained backpressure while input/list
operations stayed below 100 ms. Full milestone verification follows integration.

Review caught a shutdown race: a leader could exit after HUP while descendants
still awaited TERM, and shutdown would no longer wait for the discarded cleanup
handle. Follow-up `0fd1a61` retains cloneable completion handles independently of
entry/leader liveness and awaits them outside the manager lock. A new real-process
test reproduces the old failure for both explicit kill and parser panic, then
passes with the fix. On Linux it isolates subreaper mode in a test subprocess
and reaps only the exact owned descendant. The macOS daemon suite now passes
44 tests; the conditional Linux fixture will be validated by CI. Escalation
remains bounded: an absent group stops it early; terminal SIGKILL at three
seconds completes signal dispatch without an unbounded zombie-reaping wait.

### M3.2 protocol acceptance clarification

The brief requires both a negative JSON assertion for `has_session` and a grep
finding no occurrences. The assertion keeps that literal; the meaningful
acceptance condition is no production field or constructor use.

Commit `4c1cf93` updates all clients and fixtures to protocol 2, covers all
13 client and 8 daemon message variants in named MessagePack round-trips,
and verifies the real socket rejects version 1 with the restart instruction.
At this point all five milestone commands pass: 93 workspace tests, build,
clippy with warnings denied, formatting, and every existing PTY smoke stage.
Session id and requested model are exposed; sub-agent lists remain empty until
M3.8. The hook acknowledgment behavior deliberately remains in M3.6.

### Codex version target

The user explicitly requested the latest Codex version during implementation.
On 2026-09-18, `codex --version` and `brew list --cask --versions codex` both
report 0.155.0. `gh api repos/openai/codex/releases/latest` reports stable
`rust-v0.155.0`, published 2026-09-17T23:14:43Z
([official release](https://github.com/openai/codex/releases/tag/rust-v0.155.0)).
No upgrade is necessary. Runtime verification targets this latest stable
version; the default launcher remains `codex` on PATH, not a versioned path.

### M3.3 scripted agent harness

Commit `21975cf` adds the non-publishable `anthrex-fake-agent` crate and all ten
milestone-3 script steps. Its real subprocess tests cover Claude stdin hooks,
Codex last-argument notify, lifecycle flags, payload defaults, terminal signals,
input barriers, git commits, version/argument capture, and the unsupported MCP
step. A 2 MiB payload to a non-reading hook additionally verifies that the
five-second timeout includes blocked stdin delivery and cleans its owned group.

One brief ambiguity is resolved using product spec section 12: requested argv
capture happens before `--version` exits, while script loading/execution is
skipped. The combined case has a regression test. M3.10 will finish its startup
version probe before accepting window launches, preventing argument-file races.

Red tests failed before implementation, including a corrected timeout assertion
that rejects an immediate process failure. Green verification passes 5 unit and
14 integration harness tests, strict crate clippy, all 112 workspace tests,
formatting, and the check that `target/debug/fake-agent` is built.

Independent review found that test-side subprocess waits could hang before the
timeout assertions ran. Commit `4ece8a0` adds deadline-based child and pipe waits,
owned-process-group cleanup, and a real timeout/reaping regression. Scoped review
confirms the fix; all 5 unit and 15 integration harness tests, strict clippy,
formatting and diff checks pass.

### M3.4 launcher configuration

Commit `a2e6ea3` splits launchers, registers the ten Claude hooks with quoted
executable paths, and migrates manager creation to explicit `ManagerConfig`.
Empty binary overrides use PATH defaults. The real daemon supplies its current
executable; Codex hook-source detection remains disabled until M3.9 verification.
Independent spec/quality review approves the change. All 118 workspace tests,
strict clippy, formatting and an all-target workspace build pass.

### M3.5 pure hook/status engine

Commit `ff09e06` adds the pure Claude payload parser, source acceptance matrix,
context-aware status rules and exact title vocabulary. Codex-specific title and
notify transitions remain deferred to M3.10; manager context is wired in M3.6.
Independent spec/quality review approves the task. All 138 workspace tests,
strict clippy and formatting pass.

### M3.6 daemon hooks and viewers

Commit `9801308` adds per-window agent facts, applies hooks using the pre-event
status context, acknowledges ignored hooks, and tracks subscribed viewers.
Subscription ownership releases counts on replacement, unsubscribe, disconnect,
malformed frames and cancellation. Real PTY/socket tests cover those paths.
Independent review found no issues; all 151 workspace tests, strict clippy and
formatting pass. Sub-agent tracking remains deferred to M3.8.

### M3.7 silent hook CLI and end-to-end status

Commit `9349b24` adds the hidden pre-clap hook path and `ls --json`. A main-thread
deadline bounds the complete hook operation, including capped stdin, JSON work,
socket exchange and runtime teardown. Real isolated tests cover acknowledgment
ordering, shallow response stripping, held-open stdin and Claude status flow.
All 167 workspace tests, strict clippy and formatting pass; task review approves.

The fallback test retains the planned 3500 ms wait but adds a read-line barrier:
it observes `Quiet` before releasing `SessionStart`, because the one-second
ticker can detect a three-second quiet period near four seconds. The idle/bell
test likewise synchronizes on emitted output before its sustained assertion.
One supplemental Codex source/Error coverage test was added after the initial
green run without separate historical red evidence; it passes with the suite.

### M3.8 sub-agent tracker

Commit `2cf6fda` adds the pure tracker and real Claude PTY/socket tests for
spawn metadata, nesting, tool and permission state, and child-exit failure.
Metadata-only changes publish updated window information. Independent review
found no issues; 185 workspace tests, strict clippy, and formatting pass.
The tracker is 608 lines including its 15 unit tests; its production logic
remains cohesive. Codex metadata extraction is intentionally left to M3.11.

### M3.9 runtime findings (verified ahead of helper implementation)

Commit `24ba860` implements the verified helpers and seven literal regression
tests. Independent review found no issues; 192 workspace tests, strict clippy,
and formatting pass. The evidence below determines the corrected trust identity.

The M3.4 binary was built with `cargo build --workspace --all-targets`. All
manual runs used fresh `/tmp/anthrex-smoke-*` sockets/data and a separate
`CODEX_HOME=/tmp/anthrex-m3-codex.LvA6TL`; the model's working directory was
`/tmp/anthrex-m3-workspace.tHwabh`, not its home. `CODEX_HOME` isolation is
documented in [OpenAI's environment-variable documentation](https://developers.openai.com/fr-FR/docs/config-file/environment-variables)
and worked in practice. The empty home requested login;
only `auth.json` was copied with mode 0600, then deleted after verification.
The user's real config was never modified. Each probe stopped its own daemon;
the final process check showed only the pre-existing user daemon. The temporary
Codex home was then deleted; these notes preserve the relevant evidence.

`codex --version` reports `codex-cli 0.155.0`. `codex --help` lists `-C`, `-c`,
`-m`, `-s`, and `-a`; `codex features list` reports `hooks` and `multi_agent`
enabled. Approval-policy choices are now `on-request` and `never`.

The initial flags were:

```text
-c tui.terminal_title=["status"]
-c hooks.PreToolUse=[{hooks=[{type="command",command="/tmp/anthrex-m3-codex-hook-log.py"}]}]
```

The temporary logging hook parsed JSON from stdin and appended it to a local
JSONL file. Codex displayed a startup review, listing the command, synchronous
execution, 600-second timeout, and source `/<session-flags>/config.toml`.
Trusting this known test hook in the UI persisted key
`/<session-flags>/config.toml:pre_tool_use:0:0` with hash
`sha256:d58938d41a83039d93b170819dc549e06a9b2f646de44905e017131bfdb30177`.

The installed-tag source explains the differences from decisions 21–22:
`hooks/src/engine/discovery.rs::config_toml_source_path` uses a synthetic
session-flags path; `normalize_handler` adds `async: false` and timeout 600;
`hook_hash` converts the identity through TOML, which omits absent matcher.
`config/src/fingerprint.rs::version_for_toml` recursively sorts JSON and hashes
compact bytes. `config/src/overrides.rs::apply_toml_override` splits dotted keys
on literal periods, requiring the inline-table form below. These files were
read from `openai/codex`, tag `rust-v0.155.0`, using
`gh api 'repos/openai/codex/contents/codex-rs/<path>?ref=rust-v0.155.0' --jq .content | base64 -D`.

The verified canonical identity for the logging command is:

```json
{"event_name":"pre_tool_use","hooks":[{"async":false,"command":"/tmp/anthrex-m3-codex-hook-log.py","timeout":600,"type":"command"}]}
```

Independent SHA-256 calculation matches the persisted hash exactly. After
removing the temporary persisted trust entry, restarting with this additional
flag displayed no review and a real `pwd` call produced a new hook record:

```text
-c hooks.state={"/<session-flags>/config.toml:pre_tool_use:0:0"={trusted_hash="sha256:d58938d41a83039d93b170819dc549e06a9b2f646de44905e017131bfdb30177"}}
```

The payload arrived on stdin with `hook_event_name`, `session_id`,
`tool_name: "Bash"`, and `tool_input: {"command":"pwd"}`. This establishes
**branch A**. For decision 22's anthrex command, the corrected hashes are
`sha256:cb3a96262c960578050675ee2bf22680d8aaf909f7941553bae039b36074ad7e`
(`pre_tool_use`) and
`sha256:69de55a0df8e7abb3454e8d67f19af0d5797d208837a23f45af6e2617fb3668e`
(`session_start`). The synthetic source does not depend on `CODEX_HOME`.

Two additional hooks for the same event, one in the temporary user TOML and
one in its `hooks.json`, were explicitly trusted. One tool call then produced
all three tagged/untagged records with the same tool-use id. Restarting with
the computed session trust produced no review and all three hooks ran again:
the session array preserves both user sources and their trust entries. Codex
warns when both user representations are present, recommending one per layer;
this warning was expected in this coexistence test.

Debug logs show whole titles `Starting`, `Working`, and `Ready`, without a
prefix. A high-effort turn and a background-wait request did not produce
`Thinking` or `Waiting`; those two remain runtime-unobserved. The installed
`tui/src/chatwidget/status_surfaces.rs::run_state_status_text` confirms all five
exact strings, and the action-required prefix applies only when `Spinner` is
selected, not for `["status"]`. No parser-prefix change is justified; the two
unobserved states remain an explicit manual verification limitation.

### M3.10 full-launch trust correction

The first complete launch (`a1c5b3a`) exposed a distinction the single-hook
probe could not: repeated `hooks.state={...}` overrides replace the table
within the CLI layer. With eight generated hooks and eight temporary logging
hooks, real Codex reported 15 pending reviews; only generated `Stop` was active.
The official `config/src/overrides.rs::build_cli_overrides_layer` applies each
override in sequence, and `apply_toml_override` uses `table.insert` for this key.
Each adjacent trust override therefore must carry all generated entries so far;
the final table contains all eight. This preserves the specified pair ordering
and cross-layer user-hook/trust merging, without editing user configuration or
bypassing trust. Regression and real-runtime re-verification follow this finding.

Fix `694a77a` adds cumulative trust tables and unit/process-captured argv
regressions. Re-review is clean. Real Codex now shows one active generated hook
for each of all eight events; only the eight temporary logging hooks require
review. The temporary persisted config contains only logging-hook trust, never
session-flags trust. The status integration's prior full verification passed
217 tests, build, strict clippy, formatting, and the existing PTY smoke stages.
The version probe is a focused `lifecycle/codex_version.rs` helper rather than
embedding process ownership/deadlines in the startup function; exact argv tests
exercise the public launch boundary in `launch/mod.rs`.

### M3.11 observed sub-agent mapping

Commit `ea23840` implements the exact observed alias and verified field mapping,
with unit and real PTY/socket regressions. Independent review found no issues;
workspace tests, strict clippy, and formatting pass. The tests also reject fuzzy
tool names, ignore unobserved type fields, and never expose the opaque message.

Verification used `python3 -u /tmp/anthrex-m3-codex-probe.py
/tmp/anthrex-m3-runtime.5UY3dG /tmp/anthrex-m3-workspace.tHwabh`, with a fresh
daemon/socket per run and `ANTHREX_LOG=debug`. A final restart showed no review
prompt and reached idle. The real runtime creates its session lazily: its
`SessionStart` and session ID arrived with the first submitted turn, not at the
empty composer. All probe daemons/clients were stopped; the temporary home,
including the mode-0600 auth copy, was deleted and absence verified. Only the
pre-existing user daemon remained. The known detached-client shutdown issue
is recorded for M6; the probe helper reaped only its own client PID.

Real Codex 0.155.0 with its selected `gpt-6-astra` model emitted the exact tool
name `collaborationspawn_agent`, not the public-source prediction `spawn_agent`.
Decision 38 is refined to accept both exact names. Its input had `task_name`,
`fork_turns`, `model`, and an opaque `message`, but no type field. Extraction
uses `task_name` and `model`; type and prompt-label fallback remain unset.
Opaque messages are neither decoded nor displayed as labels. Other presets
may therefore retain the safe lifecycle `agent_type` fallback until verified.
`SubagentStart` and `SubagentStop` provide child `agent_id`, `agent_type`, and
the root `session_id`; child tool events also carry `agent_id`. No explicit
parent identifier appears, so FIFO pairing remains best effort as designed.
The observed child ran Bash to list `README.md`, finished, appeared as `done`
in `ls --json`, and never replaced the root session ID.

Full observed payloads (only the opaque message value is redacted):

```json
{"session_id":"01a0b598-36df-77f0-a183-d9552de86572","turn_id":"01a0b598-e93f-7b80-b69c-ed658db897ae","transcript_path":"/private/tmp/anthrex-m3-runtime.5UY3dG/sessions/2026/09/18/rollout-2026-09-18T18-37-26-01a0b598-36df-77f0-a183-d9552de86572.jsonl","cwd":"/private/tmp/anthrex-m3-workspace.tHwabh","hook_event_name":"PreToolUse","model":"gpt-6-astra","permission_mode":"default","tool_name":"collaborationspawn_agent","tool_input":{"task_name":"list_filenames","fork_turns":"none","model":"gpt-6-astra","message":"[opaque runtime value redacted]"},"tool_use_id":"call_6KQGopUXx7OoxsrRwWdoep8w"}
{"session_id":"01a0b598-36df-77f0-a183-d9552de86572","turn_id":"01a0b599-14ed-79c1-971d-aa5117fbac98","transcript_path":"/private/tmp/anthrex-m3-runtime.5UY3dG/sessions/2026/09/18/rollout-2026-09-18T18-38-23-01a0b599-14ca-75a0-90ee-777f88489171.jsonl","cwd":"/private/tmp/anthrex-m3-workspace.tHwabh","hook_event_name":"SubagentStart","model":"gpt-6-astra","permission_mode":"default","agent_id":"01a0b599-14ca-75a0-90ee-777f88489171","agent_type":"default"}
{"session_id":"01a0b598-36df-77f0-a183-d9552de86572","turn_id":"01a0b599-14ed-79c1-971d-aa5117fbac98","transcript_path":"/private/tmp/anthrex-m3-runtime.5UY3dG/sessions/2026/09/18/rollout-2026-09-18T18-37-26-01a0b598-36df-77f0-a183-d9552de86572.jsonl","agent_transcript_path":"/private/tmp/anthrex-m3-runtime.5UY3dG/sessions/2026/09/18/rollout-2026-09-18T18-38-23-01a0b599-14ca-75a0-90ee-777f88489171.jsonl","cwd":"/private/tmp/anthrex-m3-workspace.tHwabh","hook_event_name":"SubagentStop","model":"gpt-6-astra","permission_mode":"default","stop_hook_active":false,"agent_id":"01a0b599-14ca-75a0-90ee-777f88489171","agent_type":"default","last_assistant_message":"README.md"}
```

### M3.12 sidebar and paste safety

Commits `23f2b63` and `63ceea9` add shared variable-height card geometry,
the working-tool line, and linear paste-marker removal (including nested
markers). Review caught scalar-width truncation splitting emoji sequences;
the fix uses ratatui's intact graphemes and adds a rendered modifier/ZWJ
regression. Re-review is clean. The initial full workspace run passed 222
tests; the added emoji regression and all 10 UI tests pass, with strict clippy
and formatting. The existing large `app.rs` is a documented M4 organization
follow-up, not an unrelated refactor in this milestone.

Final review narrowed decision 51 to the boundary the implementation can
actually enforce. An isolated crossterm 0.29 event-reader probe received the
bytes `ESC[200~aESC[201~b\rESC[201~` and emitted exactly:

```text
EVENT Paste("a")
EVENT Key(KeyEvent { code: Char('b'), modifiers: KeyModifiers(0x0), kind: Press, state: KeyEventState(0x0) })
EVENT Key(KeyEvent { code: Enter, modifiers: KeyModifiers(0x0), kind: Press, state: KeyEventState(0x0) })
```

Crossterm therefore consumes the first end marker before `App::on_paste` and
delivers the remaining bytes as ordinary key events. Once `EventStream` has
split those events, anthrex cannot recover the intended clipboard boundary.
The correct pure helper remains unchanged, but M3 makes no end-to-end
clipboard escape guarantee. No timing filter or input-semantics change was
added; terminal-input policy is an explicit M7 follow-up.

### M3.13 smoke and contributor documentation

Commit `e781762` adds stage 8b and the fake-agent documentation. Every smoke
attempt overrides both runtime binaries with the fake agent, including the
harness RED run. That run detected the missing scripted lifecycle; it is not
claimed as a production RED, because earlier tasks already implement status.
The unique-data-directory script then made the stage pass: working/Bash,
completion toast/done, JSON session metadata, removal, detach, and daemon stop.
All five required commands pass locally: build, 223 workspace tests, strict
clippy, formatting, and all PTY smoke stages. No task-owned daemon remained.

### Final verification and handoff

Whole-branch review and its scoped re-review are complete. Commit `af7437c`
addresses all four findings: the paste-boundary documentation, blank script
lines, unused test stdin, and fragmented output-marker matching. All five
required local commands pass after that wave, including 227 workspace tests.
After the verification correction below, all five commands passed independently
again at `6f35470` (227 workspace tests). Both macOS and Ubuntu CI passed in
[run 35382553847](https://github.com/danielpina1/anthrex/actions/runs/35382553847).
M3 is complete in [PR #4](https://github.com/danielpina1/anthrex/pull/4), pending
human review and merge; M4 is the next ready milestone. No automatic merge.
The full human interactive checklist above remains outstanding and will be
listed in the pull request. Automated PTY checks and the documented real-Codex
probe do not substitute for that human visual/interaction sign-off.

### Fresh verification follow-ups

macOS CI exposed a test assertion comparing complete sub-agent snapshots even
though `started_secs` and `ended_secs` are ages recomputed at each read. A
deadline-based real-CLI regression deliberately crosses a second boundary and
failed with ages 0 versus 1 before the assertion fix. The test now checks all
stable fields, the root session id, age ordering and elapsed-time bounds, and
requires both ages to advance. The running snapshot also no longer assumes
that setup must finish within its first second.

A separate local `unsupported_codex_version_warns_once_at_startup` stderr-drain
timeout remains unexplained. The original failure did not identify which of
the daemon, `ls`, or stop commands timed out. One baseline and twelve repeated
four-test version suites passed, as did a temporary eight-thread diagnostic
running 160 startup probes. That diagnostic was removed. The shared command
helper now reports the command, PID, pipe and observed exit status on timeout;
deadlines and subprocess behavior are unchanged. These passing runs are not
evidence that the original timeout is fixed. Further occurrences need the new
diagnostic context before attributing a cause or changing process ownership.
