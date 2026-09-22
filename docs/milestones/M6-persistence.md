# Milestone 6: Persistence, resume, rename, config, reconnect

## Header

| | |
|--|--|
| Status | `blocked` |
| Depends on | 3 |
| Spec sections | Core spec (`docs/superpowers/specs/2026-09-17-anthrex-design.md`) 3.6, 3.7, 6.2 and 6.3 (rename and restart), 6.5 (config). Product spec (`docs/superpowers/specs/2026-09-18-anthrex-product-design.md`) section 6 items 2 to 6, 9 and 10, section 10.4 (configuration keys), section 11.3 (`runtimes.codex.bypass_hook_trust`), section 11.6, and section 12 item 1 (the config part). |
| Branch | `m6-persistence` |
| Protocol version | Unchanged. No message changes shape. `Restart` and `Rename` already exist in `crates/proto/src/messages.rs`. (Product spec 10.1: only a milestone that changes a message shape bumps `PROTO_VERSION`, to one more than the value on `main`.) |

## Goal

When this milestone is done, the daemon remembers its windows. After `anthrex daemon stop`, a crash, or a reboot, the next daemon lists every window as exited, and one key restarts a Claude or Codex window inside its previous session. The user can rename and restart windows from the client and from the CLI. A `config.toml` sets the prefix key, colours, bells, the programs to run, and a few limits. When the daemon goes away, the client keeps its screen, reconnects on its own for 30 seconds, and resubscribes. Two daemons can never share a data directory or delete each other's socket, and the daemon log no longer grows without bound.

## Starting point

Milestone 3 is merged before this one starts. This brief uses the names of `docs/milestones/M3-agent-status.md`:

- `anthrex hook` exists, and Claude windows launch with `--settings '<json>'` carrying the hook commands.
- `WindowInfo.session_id: Option<String>` has replaced `has_session`. `WindowInfo.model` and `WindowInfo.subagents` exist. The daemon records a window's session id when a hook or Codex `notify` delivers it.
- `proto::PROTO_VERSION` is whatever is on `main` when this milestone starts (`docs/ROADMAP.md`'s convention: each milestone re-derives its number as one more than the value already on `main`, rather than trusting a number written into an older brief). This milestone does not change any message's shape, so it does not bump the number (see Header).
- `crates/daemon/src/launch/` is a directory module: `mod.rs` (`LaunchContext`, `plan`, `shell_quote`, `hook_command`), `claude.rs` and `codex.rs`. `LaunchContext` carries `exe`, `claude_bin`, `codex_bin` and `codex_hook_source`.
- `WindowManager::new(config: ManagerConfig)`. `ManagerConfig` holds `socket_path`, `shell`, `exe`, `claude_bin`, `codex_bin` and `codex_hook_source`; `ManagerConfig::from_vars` reads `ANTHREX_CLAUDE_BIN` and `ANTHREX_CODEX_BIN` through a closure, and `from_env` wraps it.
- `crates/fake-agent` builds the `fake-agent` binary. The end-to-end harness `crates/cli/tests/support/mod.rs` has `fake_agent_bin()` and `TestDaemon` (`start`, `client`, `anthrex`, `data_dir`).
- Kill uses the process group (SIGHUP, SIGTERM, SIGKILL via `killpg`). `Entry` has `child_alive`, `viewers` and an `AgentState`.

If the merged code differs from these names, use the real names and record the mapping under "Implementation notes".

Milestones 4 and 5 are merged before this one starts; milestone 7 is not. Where this brief touches milestone 7's future work it says so, framed as what changes once milestone 7 lands:

- `WindowInfo.project: PathBuf` exists (milestone 4). The state file saves the project root, and restore uses the saved value. Restore computes it the way milestone 4 does at window creation only when the saved value is null.
- `WindowSpec.worktree_branch` is handled by milestone 5's worktree support (`crates/daemon/src/worktree/ops.rs`, `crates/daemon/src/manager/create.rs`). A window the daemon created a worktree for carries `Entry.managed: Option<worktree::ManagedWorktree>` (`ManagedWorktree { repo_root, path, branch }`, `crates/daemon/src/worktree/ops.rs:38`); a window merely inside an existing worktree it did not create has `Entry.managed == None`. `Entry.worktree: Option<PathBuf>` is a different field (milestone 4.5): the checkout root the daemon watches for git status, set for any window under a worktree whether or not this daemon created it. The state file saves `Entry.managed` as `WorktreeRecord`, restore puts it back on the entry, and `Remove { remove_worktree, force }` (`crates/proto/src/messages.rs`) works on restored windows as it does on live ones, including a `DaemonMsg::RemoveDirty { window_id, message }` refusal when the checkout still holds work.
- Milestone 7 is not merged: a client has one subscription today. Once it lands, reconnect resubscribes every visible pane, and the restart re-attach rule in decision 21 applies to each subscription's forwarder.

Two facts about the current code, because they differ from what one might expect:

- `ClientMsg::Rename` is already implemented. `crates/daemon/src/server.rs` answers it with `ack_or_error("rename", manager.rename(window_id, name))`, and `WindowManager::rename` checks for empty and duplicate names. Only `ClientMsg::Restart` answers `Error { request: "restart", message: "restart is not supported by this daemon version" }`.
- No key is bound to rename or restart in `crates/tui/src/keymap.rs`. `Command` has no rename, restart or reconnect variant.

## Scope

In scope:

- The config module and `config.toml`, read by the daemon, the client and the CLI.
- The state file, format version 2: debounced atomic saves, load at start, version-1 compatibility, the corrupt-file policy.
- Restored windows: listed as exited, viewable, restartable.
- Restart with resume for Claude and Codex, restart for shells, from `C-b R` and `anthrex restart`.
- Rename from `C-b ,` and `anthrex rename`.
- The lifetime lock file, the umask around bind, the socket unlink rule.
- Client reconnect, `C-b r`, resubscribe, and the dropped-`Subscribe` repair.
- `C-b Q` quits only after the daemon confirmed the shutdown.
- Log rotation and the handshake read timeout.
- The end-to-end `lifecycle::run` tests.

Out of scope:

- Runs in the state file. The `runs` array and the per-window `run` field are written empty. Milestone 8 fills them.
- The `[orchestrator]` config section. It is milestone 8 and 9. This milestone ignores it.
- Saving split-pane layouts. The product spec lists it as a follow-up.
- Screen contents across a daemon restart. The core spec says they are not persisted.
- Reloading `config.toml` while running. Each process reads it once at start.

## Design decisions

### Config

1. **Location.** `proto::paths::config_path()` returns the config file path. It gains an override: when `ANTHREX_CONFIG` is set, that path is used. Without it the path stays `dirs::config_dir()/anthrex/config.toml`, which is `~/Library/Application Support/anthrex/config.toml` on macOS and `~/.config/anthrex/config.toml` on Linux, as core spec section 2.3 says. Tests, the smoke script and every manual run set `ANTHREX_CONFIG` so the user's own config never leaks in.
2. **A new crate.** Config parsing lives in a new crate, `crates/config` (package `anthrex-config`, library name `config`), because the daemon, the client and the CLI all read it and `proto` does no file I/O. It depends on `proto` (for `Runtime`), `serde` and `toml = "1"`. `toml` 1.1.6 was checked with `cargo info toml` on 2026-09-18: its `rust-version` is 1.85, below this workspace's 1.92.
3. **Parsing never fails.** The file is parsed into a `toml::Table`, then each key is read on its own. A value with the wrong type or out of range produces a `config::Problem` and that key keeps its default. A TOML syntax error produces one `Problem` and every key keeps its default. A missing file is not a problem.
4. **Keys, defaults and validation.**

   | Key | Default | Valid values | Consumed by |
   |-----|---------|--------------|-------------|
   | `prefix` | `"C-b"` | `"C-"` followed by one lowercase ASCII letter, except `h`, `i`, `j` and `m`: terminals send those as Backspace, Tab, Enter and Enter | client keymap, help and hints |
   | `accent` | `"#89b4fa"` | `#` and six hex digits | client theme |
   | `bell.attention` | `true` | boolean | client: ring the outer terminal's bell when a background window enters Attention |
   | `bell.done` | `false` | boolean | client: ring it when a background window enters Done |
   | `default_runtime` | `"shell"` | `"claude"`, `"codex"`, `"shell"` | `C-b c` (the new-agent dialog's preselected runtime), `anthrex new` without `--runtime` |
   | `scrollback_lines` | `5000` | 0 to 100000 | client parsers |
   | `ui.sidebar_width` | the client's built-in width, `tui::ui::DEFAULT_SIDEBAR_WIDTH` (34) | 24 to 60 | client layout: the starting width that `C-b <` and `C-b >` adjust |
   | `ui.tree_keep_finished_secs` | `300` | 0 to 300. The daemon drops finished sub-agents after 300 seconds (product spec 4.4 rule 3), so more would have no effect | milestone 4's tree hides finished sub-agent rows older than this (task M6.9) |
   | `panes.max` | `6` | 1 to 16 | milestone 7's pane limit. The value is carried in `UiSettings` for it |
   | `runtimes.claude.command` | `"claude"` | a non-empty string | daemon launcher |
   | `runtimes.codex.command` | `"codex"` | a non-empty string | daemon launcher and the Codex version probe |
   | `runtimes.codex.bypass_hook_trust` | `false` | boolean | daemon launcher: product spec 11.3. When true, every Codex window gets `--dangerously-bypass-hook-trust` right after the five `-c` flags of milestone 3's decision 19, followed by milestone 3's eight `-c hooks.<Event>=...` flags without the `hooks.state...trusted_hash` flags. This also turns lifecycle hooks on when `codex_hook_source` is `None` |

5. **Unknown keys.** An unknown key anywhere produces a `Problem` with the message `unknown key, ignored`, so typos are visible. The whole `[orchestrator]` table is skipped silently: milestone 8 parses it. A top-level `sidebar_width`, the core spec's name, is honoured as `ui.sidebar_width` and produces the problem `renamed to ui.sidebar_width`. When both are present, `ui.sidebar_width` wins.
6. **Runtime commands.** A command is a program name or path, not a command line. It is not split on spaces. A leading `~/` is expanded against `$HOME` when the config is read. A bare name is found on `PATH` when the window spawns, through the existing `/bin/sh -c 'exec "$0" "$@"'` wrapper in `Window::spawn`. Precedence, highest first: `ANTHREX_CLAUDE_BIN` or `ANTHREX_CODEX_BIN`, then `runtimes.*.command`, then the default. The daemon resolves them once at start into milestone 3's `ManagerConfig.claude_bin` and `ManagerConfig.codex_bin`: `ManagerConfig::from_vars` and `from_env` gain a `runtimes: &config::Runtimes` parameter and apply this precedence. `ManagerConfig` also gains `codex_bypass_hook_trust: bool`, copied into `LaunchContext`. A changed config therefore needs a daemon restart.
7. **Problems are shown once.** The daemon logs each problem at `warn` once, at start. The client shows all of them once, at start, in a `Modal::Notice` titled ` config `, dismissed by any key. `anthrex new`, the only one-shot command that reads the config, prints each to stderr as `anthrex: config: <key>: <message>`. `anthrex hook` never reads the config. The display form of a `Problem` is `<key>: <message> (using <default>)`, for example `prefix: expected C- followed by a letter (using C-b)`.

### State file

8. **Path and shape.** The state file is `<data_dir>/state.json`, where `<data_dir>` is `DaemonOptions::data_dir`. Format version 2:

   ```json
   {
     "version": 2,
     "next_id": 8,
     "windows": [
       {
         "id": 3,
         "name": "api-worker",
         "runtime": "claude",
         "cwd": "/Users/me/repos/shop",
         "project": "/Users/me/repos/shop",
         "worktree": null,
         "model": "opus",
         "initial_prompt": "fix the failing tests",
         "session_id": "5f0c2d1e-8a8b-4c1e-9d55-2b7e9f1a0c11",
         "created_at": 1789123456,
         "status": "working",
         "run": null
       }
     ],
     "runs": []
   }
   ```

   `runtime` and `status` use the existing lowercase serde names of `proto::Runtime` and `proto::Status`. `created_at` is Unix seconds. `model` and `initial_prompt` are the values from the window's `WindowSpec`, not what hooks reported. `worktree` is `{"repo_root": "...", "path": "...", "branch": "..."}` when the window's `Entry.managed` is `Some`, `null` otherwise. `project` and `worktree` follow "Starting point". `run` is always `null` and `runs` always `[]` in this milestone.
9. **Saving.** The daemon saves whenever the window list changes. A new persister task subscribes to `WindowManager::watch()`. Every name, status, session-id and project change already publishes on that channel. On a change it waits 100 ms, collecting any further changes. Then it takes `WindowManager::state_snapshot()`, a clone taken under the manager lock with no I/O under it, and writes the file on `tokio::task::spawn_blocking`. It skips the write when the serialized bytes equal the last ones it wrote. A failed write is logged at `warn` and tried again on the next change.
10. **Atomic write.** `state::save` serializes with `serde_json::to_vec_pretty`. It writes `<data_dir>/state.json.tmp`, created with mode `0o600` because prompts can be private, then calls `sync_all` on it. It renames the file over `state.json`, then opens `<data_dir>` and calls `sync_all` on the directory so the rename itself is durable. Only one writer exists at a time: the persister, or the final flush after the persister has stopped (decision 11).
11. **Shutdown order.** In `lifecycle::run`, after `server::serve` returns: `manager.shutdown().await`; await the persister task, which stops on the same `CancellationToken`; save once more from `state_snapshot()` on `spawn_blocking`, awaited; unlink the socket (decision 26); remove `daemon.pid`; release the lock last. A socket file that has disappeared therefore means the state is on disk.
12. **Loading.** `lifecycle::run` loads the state file once, on `spawn_blocking`, before it binds the socket, so the first client's `Welcome` already lists the restored windows.
    - Missing file: start empty.
    - Any other read error: log at `error` and start empty. Leave the file alone.
    - Not valid JSON, not an object, or `version` or `next_id` missing or not an integer: the file is corrupt. Rename it to `state.json.corrupt-<unix-seconds>`, adding `-1`, `-2` and so on if that name exists. Log the new path at `warn` and start empty.
    - `version` greater than 2: rename it to `state.json.unsupported-<unix-seconds>` in the same way. Log at `warn` and start empty. This protects a newer daemon's file from being overwritten by an older one.
    - Version 1 or 2: each entry of `windows` is deserialized on its own into `WindowRecord`. An entry that fails, such as an unknown runtime, or that repeats an earlier entry's id or name, is skipped and logged at `warn`. The rest load.
    - `next_id` becomes the larger of the saved value and the largest loaded id plus one.
13. **Version 1.** Core spec 3.6 defines version 1 as `version`, `next_id` and `windows`, each window holding `id`, `name`, `runtime`, `cwd`, `worktree`, `model`, `initial_prompt`, `session_id`, `created_at` and `status`. Milestone 1 never wrote such a file, but the loader accepts one. Every field `WindowRecord` has beyond `id`, `name`, `runtime` and `cwd` is `#[serde(default)]`, so a version-1 record loads with `project`, `session_id`, `model` and `run` empty when absent, as product spec 6 item 5 says. `status` defaults to `exited`. It is read leniently: an unknown string counts as `exited`. The next save writes version 2. Unknown fields are ignored, so a later milestone can add fields without breaking this loader.
14. **Restored windows.** Each loaded record becomes a window with no process. Its status is `Exited`. Its `exit` is `Some(ExitInfo { code: None, reason: "daemon restarted" })`, with the reason from the new constant `daemon::manager::DAEMON_RESTARTED`. Its `since` is the load time. Its session id, name, spec and `created_at` are the saved ones. Inside the manager, the new private enum `Process` replaces the plain `Window` field of `Entry`: `Process::Live(Window)` or `Process::Dormant { output: broadcast::Sender<Bytes>, cols: u16, rows: u16 }`. The dormant sender has capacity 1, and nothing is ever sent on it. It exists so that a subscriber has a receiver that closes when the window restarts (decision 21). A dormant window's behaviour:
    - `resize` records the size.
    - `write_input` fails with `window is not running; restart it with C-b R or anthrex restart <id>`.
    - `kill` and `signal` succeed and do nothing.
    - `remove` drops the record.
    - `attach` returns a placeholder snapshot at the recorded size: `ESC [ 2 J ESC [ H`, then `[anthrex] this window stopped when the daemon restarted.\r\n`, then `[anthrex] restart it with C-b R (default keys) or: anthrex restart <id>\r\n`, then, when the session id is known, `[anthrex] the restart resumes session <session_id>.\r\n`.

### Restart and resume

15. **Resume argv.** `launch::LaunchContext` gains `resume: Option<&'a str>`. When it is `Some(id)`:
    - Claude: the normal argv (`--name <name>`, milestone 3's `--settings`, `--model <model>` when set), then `--resume <id>`. The initial prompt and its `--` are left out.
    - Codex: the normal global options (`-C <cwd>`, milestone 3's `-c` overrides, `-m <model>` when set, see decision 16), then `resume <id>`. The initial prompt and its `--` are left out.
    - Shell: ignored.

    The initial prompt is never sent on resume. It already belongs to the session being resumed, and sending it again would start the task over. A restart without a known session id is a fresh launch, exactly like the original create, initial prompt included. Claude `--resume <session_id>` is verified for Claude Code 2.1.x in core spec section 10. `claude --help` on 2.1.276 (checked 2026-09-18) lists `-r, --resume` and `-n, --name`.
16. **Codex resume options.** Core spec 3.2 and risk 4 say to check which of `-m` and a prompt `codex resume` accepts, and to drop the rest on resume only. Checked on 2026-09-18 against codex-cli 0.155.0, which is newer than the 0.135.0 the spec cites. `codex resume --help` prints `Usage: codex resume [OPTIONS] [SESSION_ID] [PROMPT]` and lists `-C, --cd`, `-c, --config` and `-m, --model`. `codex -C /tmp -m gpt-x -c 'tui.notification_method="bel"' resume --help` exits 0, so the parser accepts root options before `resume`. Decision: keep `-m` on resume, and never pass a prompt on resume (decision 15). Task M6.6 repeats the check against the implementer's installed `codex` and records the output. If `-m` is not listed there, `launch::plan` drops `-m` on resume only. Whether Codex applies root options to the `resume` subcommand is confirmed by the manual check, step 6. If it does not, move the options after `resume` and record it; both forms appear in `codex resume --help` on 0.155.0.
17. **Restarting an exited window.** It relaunches at once. It gets the same id, name and spec, and a fresh `Window` from `Window::spawn` at the last known size. `resume` is the saved session id. The process is swapped in under the manager lock. The window's status becomes `Starting` as on create, `exit`, `tool` and the sub-agent list are cleared, and the session id is kept. A restart never creates a worktree: it runs in the saved `cwd`, which for a worktree window is the worktree path (milestone 5 decision 11), and leaves `Entry.managed` and `Entry.worktree` untouched. It never calls `worktree::create` from `spec.worktree_branch`.
18. **Restarting a live window.** It kills the window first through milestone 3's kill path. It waits on a spawned task, never under the lock, polling every 50 ms until the status is `Exited`, for at most the kill escalation's total grace plus 2 seconds, then relaunches as in decision 17. When the wait runs out, the reply is `Error { request: "restart", message: "window <id> did not exit; not restarted" }`. A second `Restart` for a window already being restarted gets `Error { request: "restart", message: "window <id> is already restarting" }`. `Entry` gains `restarting: bool` for this.
19. **Restart preconditions.** A window whose `cwd` no longer exists gets `Error { request: "restart", message: "directory does not exist: <cwd>" }`. `create` already checks the same (`crates/daemon/src/manager/create.rs`). A removed worktree path fails the same way, since a worktree window's `cwd` is the worktree path itself (decision 17).
20. **The Restart request.** `handle_client` runs `manager.restart(window_id)` on a `tokio::spawn`ed task. That task sends `Ack { request: "restart" }` or `Error { request: "restart", .. }` through a clone of `out_tx`. The client's request loop keeps serving other messages while a live window is being stopped.
21. **Subscribers follow a restart.** Swapping the process drops the old `Window`, or the dormant sender, and so closes the broadcast channel a forwarder is reading. In `forward_output_from`, `RecvError::Closed` no longer ends the loop at once. It calls `reattach()`. On success it replaces the receiver and sends a fresh `Snapshot`, exactly like the `Lagged` branch. On failure, because the window was removed, it ends. The swap happens under the same lock `attach` takes, so a re-attach always sees the new process. No protocol change is needed: every viewer of the window gets the new screen.

### Rename

22. **Rename rules.** A window name is trimmed, and must be 1 to 64 characters with no control characters (Unicode category `Cc`, `char::is_control()`) and no bidi text-direction override characters — U+202A through U+202E and U+2066 through U+2069 — refused with the same message as a control character. **Amended, fix wave 6 (task 8 review, Minor):** the original wording ("no control characters", implemented with `char::is_control()`) let U+202E RIGHT-TO-LEFT OVERRIDE through, because it is Unicode category `Cf` ("format"), not `Cc`. Confirmed live against the daemon: `anthrex rename 1 "bad\u{202e}name"` succeeded, and `anthrex ls` rendered the name with its glyphs reordered — the "Trojan Source" spoofing class, and a real security surface here specifically, because anthrex renders window names as labels distinguishing several agents running side by side with different worktrees and permissions. The fix rejects exactly this list of bidi formatting characters, not the whole `Cf` category: `Cf` also contains U+200D ZERO WIDTH JOINER, which a legitimate multi-codepoint emoji sequence needs to fuse into the single grapheme cluster this decision's own 64-character limit is built to count (a family emoji, `man+ZWJ+woman+ZWJ+girl+ZWJ+boy`, at exactly 64 repetitions, must keep being accepted) — rejecting the category would break that case to fix this one. The manager enforces this on both `create` and `rename`, with the errors `name must not be empty`, `name must be at most 64 characters` and `name must not contain control characters`. The existing duplicate check stays. The client checks the same rules before it sends, so an error shows inside the prompt, and it still shows a daemon `Error` as a toast. The next resume of a Claude window passes the new name to `--name`.

### Keys

23. **Key bindings.** Product spec 6 item 4 gives `C-b r` to reconnect. The core spec's keymap gave `C-b r` to rename and `C-b R` to restart. Resolution:
    - `C-b ,` renames the focused window, as in tmux.
    - `C-b R` restarts the focused window. An exited window restarts at once, with the toast `restarting <name>`. A live window asks first: `Restart '<name>'? It is running and will be stopped first.`
    - `C-b r` reconnects while disconnected. While connected it shows the toast `connected`.

    None of these keys is used by milestones 4, 7 or 9.

### Lifecycle and the socket

24. **Lock file.** The lock file is `<data_dir>/daemon.lock`. `proto::paths::lock_path()` returns it for the CLI. `daemon::lockfile::DaemonLock::acquire` opens it with `std::fs::OpenOptions` (read, write, create, mode `0o600`). Rust opens files with `O_CLOEXEC`, so agent processes never inherit the lock. It then calls `libc::flock(fd, LOCK_EX | LOCK_NB)`. On `EWOULDBLOCK` it retries every 100 ms for up to `DaemonOptions::lock_wait`, which the CLI sets to `daemon::LOCK_WAIT` (5 seconds). The wait covers a stop immediately followed by a start. If the lock is still held, `run` returns the error `another anthrex daemon is running with data directory <dir> (pid <pid>)`. The pid comes from `daemon.pid`, or reads `unknown` if that file is unreadable. Acquiring the lock is the first thing `run` does after creating the data directory, before logging starts, before the socket is looked at, and before `daemon.pid` is written. The `DaemonLock` value lives until `run` returns. Dropping it closes the file, which releases the lock.
25. **Umask around bind.** `lifecycle::bind_socket` sets `libc::umask(0o077)` immediately before `UnixListener::bind` and restores the previous mask immediately after, on success and on error. The socket is born `0o600`. The existing `set_permissions(0o600)` stays as a second guard.
26. **Unlinking the socket.** After binding, `run` records the socket's `(dev, ino)` from `std::fs::metadata`. At shutdown it unlinks the path only if the path's current `(dev, ino)` is still the recorded pair, and it does this while still holding the lock. A daemon with the same data directory cannot exist while the lock is held. The inode check also covers a replacement daemon that uses a different data directory but the same `ANTHREX_SOCKET`. `prepare_socket` keeps its live-socket probe for that case. With the lock held, the probe-then-remove race can only happen between daemons with different data directories, which is a misconfiguration.
27. **`anthrex daemon stop` waits for the exit.** After the connection closes, `daemon stop` waits until `daemon::lockfile::wait_released(&proto::paths::lock_path(), Duration::from_secs(10))` returns true, on `spawn_blocking`. That call tries `LOCK_EX | LOCK_NB` on a fresh descriptor every 100 ms and releases at once on success. Only then does it print `daemon stopped`. If the timeout passes, it fails with `the daemon did not exit within 10 s; see <log path>`. `anthrex daemon stop` followed by `anthrex` therefore never races a daemon that is still stopping.
28. **Log rotation.** The daemon log is size-based. `daemon::logfile::RotatingFile` implements `std::io::Write` over `<data_dir>/daemon.log`, and `tracing_appender::non_blocking` wraps it, so rotation runs on the appender's own thread. Before writing a buffer that would take the file past `LOG_MAX_BYTES` (10 MiB), it rotates: deletes `daemon.log.4`, renames `.3` to `.4`, `.2` to `.3`, `.1` to `.2` and `daemon.log` to `.1`, then opens a new `daemon.log`. At most `LOG_KEEP` = 5 files exist, 50 MiB in total. A single buffer larger than the limit is written whole into a fresh file. `init_logging` uses `try_init` so that two `run` calls in one test process do not panic.
29. **Handshake timeout.** The new constant `proto::HANDSHAKE_TIMEOUT` is 5 seconds.
    - The server wraps its first `read_frame` (the `Hello`) in it. On timeout it logs at `debug` and drops the connection.
    - `tui::connection::Connection::connect` and `CliClient::connect` wrap their `Welcome` read in it. On timeout they fail with `timed out waiting for the daemon's handshake at <socket>`.

### Client: reconnect and quitting

30. **Link state.** `App.connected: bool` is replaced by `App.link: Link`, with `Connected`, `Reconnecting { attempts: u32, reason: String }` and `Lost { reason: String }`. `App::connected()` returns whether it is `Connected`. All of this is pure state. The event loop in `crates/tui/src/lib.rs` owns the socket, the timers and the attempt task.
31. **Retry schedule.** When the connection closes, the event loop makes a first attempt 2 seconds later (`reconnect::RETRY_INTERVAL`). Each failed attempt schedules the next one 2 seconds after it finished. The loop gives up after the first failure that ends 30 seconds or more after the drop (`reconnect::RETRY_WINDOW`), and the link becomes `Lost`. Each attempt runs on a spawned task so the UI stays responsive: connect, then a handshake bounded by `HANDSHAKE_TIMEOUT`. A protocol-version `Error` counts as a failed attempt, and its message becomes the `reason`. Automatic attempts never start a daemon: a daemon stopped on purpose stays stopped.
32. **`C-b r`.** While not connected, `C-b r` starts an attempt now, unless one is already in flight. It opens a new 30-second window. This attempt may start the daemon: if connecting fails and `TuiOptions::daemon_exe` is `Some`, it calls `tui::spawn::ensure_daemon`, then connects. `ensure_daemon` moves from `crates/cli/src/spawn.rs` to the new `crates/tui/src/spawn.rs` and takes the executable path as a parameter. The CLI passes `std::env::current_exe()`, and tests pass `None`. This is the core spec 6.3 "re-spawn the daemon" behaviour.
33. **Resubscribe.** On success, `App::on_reconnected(windows)` sets `Connected`, applies the `Welcome` list as a `WindowsChanged`, and resubscribes the focused window with a fresh parser, even though it is already `focused`. With milestone 7, it resubscribes every visible pane. If the focused window is gone, the existing fallback in `replace_windows` picks a neighbour. Exactly one `Subscribe` is emitted per visible window. The toast is `reconnected`.
34. **What the user sees while disconnected.** The main area keeps the last screen. The status bar shows the red ` DISCONNECTED ` badge (already in `ui/statusbar.rs`), followed by `reconnecting (attempt N)` while `Reconnecting`, or by `<prefix> r to reconnect` once `Lost`. Keystrokes for the PTY are dropped silently. Commands that need the daemon toast `not connected; <prefix> r to reconnect`.

    ```
     DISCONNECTED  reconnecting (attempt 3)             daemon: daemon shutting down
     DISCONNECTED  C-b r to reconnect                   connection to the daemon lost
    ```

35. **Dropped `Subscribe`.** `App` gains `subscribed: Option<u32>`, the window whose `Subscribe` was handed to the connection. The early return in `App::focus` for the already-focused window now also requires `subscribed == Some(id)`. `lib.rs` reports every refused send to `App::on_send_failed(&ClientMsg)`. For a `Subscribe`, that clears `subscribed`, and `on_tick` then sends `Subscribe` again for the focused window while `Connected` and `subscribed != focused`. The tick runs every 100 ms, so a full outgoing queue is retried until it drains. Other refused sends keep today's behaviour: `Input` silently, everything else with the toast `daemon is not responding`.
36. **`C-b Q`.** After the confirm, `C-b Q` sends only `Shutdown` and sets `App.stopping: Option<Instant>`. The client quits when the link closes while `stopping` is set; `Bye` arrives first and the close follows it. If the `Shutdown` send is refused, `stopping` is cleared and the toast reads `could not reach the daemon; run anthrex daemon stop`. If no close arrives within 5 seconds, `on_tick` clears `stopping` and toasts `the daemon did not confirm the shutdown; run anthrex daemon stop`. While `stopping` is set, a lost link never starts a reconnect.
37. **Bells.** When a window that is not focused changes to `Attention` and `bell.attention` is true, or to `Done` and `bell.done` is true, `replace_windows` also returns `Effect::Bell`. `lib.rs` writes one `0x07` byte to stdout and flushes. It sends at most one bell per `WindowsChanged`.

### Client: settings and file layout

38. **`UiSettings`.** The client's view of the config is the new struct `tui::settings::UiSettings` (decision 4 lists its fields' sources). `App::new(windows, default_dir, settings)` replaces the current `App::new(windows, default_dir, prefix)`. `UiSettings::default()` matches `config::Config::default()`. `app::SCROLLBACK_LINES` goes away in favour of `settings.scrollback_lines`. `theme::ACCENT` becomes `theme::DEFAULT_ACCENT`. Every renderer takes the accent from `app.settings.accent`, and every piece of help or hint text takes the prefix label from `app.settings.prefix_label` instead of a hard-coded `C-b`.
39. **Splitting `app.rs`.** Before adding to it, turn `crates/tui/src/app.rs` into `crates/tui/src/app/mod.rs` with `git mv`. Milestone 4 moved its tests into `crates/tui/src/app_tests.rs` (included with `#[path]`); move that file to `crates/tui/src/app/tests.rs` with `git mv` and include it as a plain `#[cfg(test)] mod tests;`. Milestone 4's `crates/tui/src/tree_input.rs` stays where it is. Milestone 5 already created `crates/tui/src/app/modal_keys.rs` (declared as `mod modal_keys;` inside `app.rs`); it stays a submodule of `app`. New pure code goes into `crates/tui/src/app/link.rs` (link state, reconnect transitions, stopping) and `crates/tui/src/app/prompt.rs` (the rename prompt). The purity rule in `AGENTS.md` applies to everything under `crates/tui/src/app/`.

## Interfaces

### `crates/config` (new crate, package `anthrex-config`, library `config`)

```rust
pub struct Config {
    pub prefix: Prefix,
    pub accent: Rgb,
    pub bell: Bell,
    pub default_runtime: proto::Runtime,
    pub scrollback_lines: usize,
    pub ui: Ui,
    pub panes: Panes,
    pub runtimes: Runtimes,
}
pub struct Prefix(pub char);                 // Ctrl + this lowercase letter
impl Prefix { pub fn label(&self) -> String } // "C-b"
pub struct Rgb(pub u8, pub u8, pub u8);
pub struct Bell { pub attention: bool, pub done: bool }
pub struct Ui { pub sidebar_width: Option<u16>, pub tree_keep_finished_secs: u64 }
pub struct Panes { pub max: u8 }
pub struct Runtimes { pub claude: RuntimeCommand, pub codex: RuntimeCommand, pub codex_bypass_hook_trust: bool }
pub struct RuntimeCommand { pub command: String }
pub struct Problem { pub key: String, pub message: String, pub default: String }
impl std::fmt::Display for Problem          // "<key>: <message> (using <default>)"
impl Default for Config

pub fn parse(text: &str) -> (Config, Vec<Problem>);
pub fn load(path: &std::path::Path) -> (Config, Vec<Problem>); // missing file: defaults, no problems
```

All types derive `Debug, Clone, PartialEq`.

A complete example file, which is also the test fixture for `every_key_is_read`:

```toml
prefix = "C-a"
accent = "#f5c2e7"
default_runtime = "claude"
scrollback_lines = 10000

[bell]
attention = true
done = true

[ui]
sidebar_width = 40
tree_keep_finished_secs = 120

[panes]
max = 4

[runtimes.claude]
command = "~/bin/claude"

[runtimes.codex]
command = "/opt/homebrew/bin/codex"
bypass_hook_trust = true

[orchestrator]
max_parallel = 3
```

### `proto`

```rust
pub const HANDSHAKE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5); // lib.rs, new
pub fn config_path() -> PathBuf;  // paths.rs: now honours ANTHREX_CONFIG
pub fn lock_path() -> PathBuf;    // paths.rs, new: data_dir().join("daemon.lock")
```

### `daemon`

```rust
// lifecycle.rs
pub struct DaemonOptions {
    pub socket_path: PathBuf,
    pub data_dir: PathBuf,
    pub config_path: PathBuf,         // new
    pub lock_wait: Duration,          // new; the CLI passes LOCK_WAIT, tests pass Duration::ZERO
}
pub const LOCK_WAIT: Duration = Duration::from_secs(5); // re-exported from lib.rs

// lockfile.rs (new)
pub struct DaemonLock { /* holds the open File */ }
impl DaemonLock { pub fn acquire(data_dir: &Path, wait: Duration) -> anyhow::Result<DaemonLock> }
pub fn wait_released(lock_path: &Path, timeout: Duration) -> bool; // blocking

// logfile.rs (new)
pub const LOG_MAX_BYTES: u64 = 10 * 1024 * 1024;
pub const LOG_KEEP: usize = 5;
pub struct RotatingFile { /* ... */ }
impl RotatingFile { pub fn open(dir: &Path, name: &str, max_bytes: u64, keep: usize) -> std::io::Result<Self> }
impl std::io::Write for RotatingFile

// state.rs (new)
pub const STATE_VERSION: u32 = 2;
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StateFile {
    pub version: u32,
    pub next_id: u32,
    #[serde(default)] pub windows: Vec<WindowRecord>,
    #[serde(default)] pub runs: Vec<serde_json::Value>, // milestone 8 replaces the element type
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WindowRecord {
    pub id: u32,
    pub name: String,
    pub runtime: Runtime,
    pub cwd: PathBuf,
    #[serde(default)] pub project: Option<PathBuf>,
    #[serde(default)] pub worktree: Option<WorktreeRecord>,
    #[serde(default)] pub model: Option<String>,
    #[serde(default)] pub initial_prompt: Option<String>,
    #[serde(default)] pub session_id: Option<String>,
    #[serde(default)] pub created_at: u64,
    #[serde(default = "exited", deserialize_with = "lenient_status")] pub status: Status,
    #[serde(default)] pub run: Option<serde_json::Value>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorktreeRecord { pub repo_root: PathBuf, pub path: PathBuf, pub branch: String }

// Fix wave 1 (task-2/3/4 review): decision 12 distinguishes "log at error" for an
// unreadable file (real data loss for this boot) from "log at warn" for a corrupt/
// unsupported file or a skipped record (already recovered) — a bare Vec<String> cannot
// carry that. Problem mirrors config::Problem (crates/config/src/lib.rs) with a Severity
// field added; load's warnings become Vec<Problem>.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity { Error, Warn }
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem { pub severity: Severity, pub key: String, pub message: String } // Display: "{key}: {message}"

pub fn load(path: &Path) -> (StateFile, Vec<Problem>);   // blocking; levelled problems to log
pub fn save(path: &Path, state: &StateFile) -> std::io::Result<()>; // blocking, atomic
pub fn spawn_persister(manager: Arc<WindowManager>, path: PathBuf, shutdown: CancellationToken)
    -> tokio::task::JoinHandle<()>;
pub const SAVE_DEBOUNCE: Duration = Duration::from_millis(100);

// launch/mod.rs
pub struct LaunchContext<'a> {
    // milestone 3's fields (window_id, name, socket_path, shell, exe, claude_bin, codex_bin, codex_hook_source), plus:
    pub codex_bypass_hook_trust: bool, // new; decision 4
    pub resume: Option<&'a str>,       // new; decision 15
}

// manager.rs
pub const DAEMON_RESTARTED: &str = "daemon restarted";
pub struct ManagerConfig {
    // milestone 3's fields (and milestone 5's worktrees_root), plus:
    pub codex_bypass_hook_trust: bool, // new
}
impl ManagerConfig {
    /// Milestone 3's function with a new `runtimes` parameter: env, then config, then default (decision 6).
    pub fn from_vars(socket_path: PathBuf, shell: String, exe: PathBuf, runtimes: &config::Runtimes,
                     var: impl Fn(&str) -> Option<String>) -> Self;
    pub fn from_env(socket_path: PathBuf, shell: String, runtimes: &config::Runtimes) -> anyhow::Result<Self>;
}
impl WindowManager {
    // `new(config: ManagerConfig)` is unchanged.
    pub fn restore(&self, state: StateFile);
    pub fn state_snapshot(&self) -> StateFile;
    pub async fn restart(self: &Arc<Self>, id: u32) -> anyhow::Result<()>;
}
```

### `tui`

```rust
// lib.rs
pub struct TuiOptions {
    pub socket_path: PathBuf,
    pub default_dir: PathBuf,
    pub focus: Option<String>,
    pub settings: settings::UiSettings,     // new
    pub config_problems: Vec<String>,       // new, already formatted
    pub daemon_exe: Option<PathBuf>,        // new; Some(current_exe) from the CLI
}

// settings.rs (new)
#[derive(Debug, Clone, PartialEq)]
pub struct UiSettings {
    pub prefix: (KeyCode, KeyModifiers),
    pub prefix_label: String,
    pub accent: ratatui::style::Color,
    pub bell_attention: bool,
    pub bell_done: bool,
    pub default_runtime: proto::Runtime,
    pub scrollback_lines: usize,
    pub sidebar_width: u16,
    pub tree_keep_finished_secs: u64,
    pub panes_max: u8,
}
impl Default for UiSettings
impl UiSettings { pub fn from_config(c: &config::Config) -> Self }

// app/
pub enum Effect { Send(ClientMsg), Quit, Reconnect, Bell }
pub enum Link { Connected, Reconnecting { attempts: u32, reason: String }, Lost { reason: String } }
// Milestone 5 left PendingAction { Kill(u32), StopDaemon } (Remove moved to Modal::Remove); this adds Restart.
pub enum PendingAction { Kill(u32), StopDaemon, Restart(u32) }
pub enum Modal {
    // milestone 1's Confirm and Help, milestone 5's NewAgent, Remove and ForceRemove, plus:
    Rename { window_id: u32, input: String, error: Option<String> },
    Notice { title: String, lines: Vec<String> },
}
impl App {
    pub fn new(windows: Vec<WindowInfo>, default_dir: PathBuf, settings: UiSettings) -> Self;
    pub fn connected(&self) -> bool;
    pub fn on_link_lost(&mut self, reason: String) -> Vec<Effect>;
    pub fn on_reconnect_failed(&mut self, error: String, gave_up: bool);
    pub fn on_reconnected(&mut self, windows: Vec<WindowInfo>) -> Vec<Effect>;
    pub fn on_send_failed(&mut self, msg: &ClientMsg);
}

// keymap.rs: Command gains RenameWindow (','), RestartWindow ('R'), Reconnect ('r').

// reconnect.rs (new)
pub const RETRY_INTERVAL: Duration = Duration::from_secs(2);
pub const RETRY_WINDOW: Duration = Duration::from_secs(30);
pub struct RetrySchedule { /* started: Instant, next: Instant */ }
impl RetrySchedule {
    pub fn after_drop(now: Instant) -> Self;        // next = now + RETRY_INTERVAL
    pub fn manual(now: Instant) -> Self;            // next = now, new 30 s window
    pub fn due(&self, now: Instant) -> bool;
    pub fn after_failure(&mut self, now: Instant) -> bool; // false = give up
}
pub async fn attempt(socket: PathBuf, daemon_exe: Option<PathBuf>) -> anyhow::Result<Connection>;

// spawn.rs (moved from crates/cli/src/spawn.rs)
pub async fn ensure_daemon(exe: &Path, socket: &Path) -> anyhow::Result<()>;
```

### CLI

```
anthrex rename <id|name> <new-name>   Rename a window. Prints nothing; exit 1 with the daemon's error.
anthrex restart <id|name>             Restart a window, resuming its session when known. Waits up to 15 s for Ack.
anthrex new [--runtime claude|codex|shell] ...   --runtime defaults to config default_runtime.
anthrex daemon stop                   Now returns only after the daemon released its lock (decision 27).
```

`CliClient` gains `request_with_timeout(msg, Duration)`. `restart` uses 15 seconds. `request` keeps its 5 seconds.

## Tasks

### M6.1 The config crate and `ANTHREX_CONFIG`

**Files.** Create `crates/config/Cargo.toml` and `crates/config/src/lib.rs`. Modify the root `Cargo.toml` (add the member, add `config = { path = "crates/config", package = "anthrex-config" }` and `toml = "1"` to `[workspace.dependencies]`) and `crates/proto/src/paths.rs`.

**Tests first.** In `crates/config/src/lib.rs`:
- `empty_text_gives_defaults`: `parse("")` returns `Config::default()` and no problems. Assert the defaults field by field against decision 4, with `ui.sidebar_width == None`.
- `every_key_is_read`: parse the example file from "Interfaces". Every field has the example's value. `runtimes.claude.command` is `$HOME/bin/claude`, with `$HOME` read in the test. There are no problems: `[orchestrator]` is skipped silently.
- `bad_values_keep_their_defaults`: `prefix = "C-m"`, `accent = "blue"`, `scrollback_lines = -1`, `ui.sidebar_width = 10`, `panes.max = 17`, `runtimes.codex.bypass_hook_trust = "yes"`, `bell.done = "yes"`, `default_runtime = "perl"`, `runtimes.codex.command = ""`. Each key keeps its default, and there is exactly one problem per key, naming that key.
- `syntax_error_is_one_problem`: `parse("prefix = ")` gives defaults and exactly one problem.
- `unknown_keys_are_reported_and_ignored`: `colour = "red"` and `[bell] loud = true` give two problems with message `unknown key, ignored`.
- `old_sidebar_width_is_an_alias`: `sidebar_width = 40` gives `ui.sidebar_width == Some(40)` and one problem `renamed to ui.sidebar_width`. With both keys set, `ui.sidebar_width` wins.
- `problem_display`: formats as `prefix: expected C- followed by a letter (using C-b)`.
- `missing_file_is_defaults`: `load` on a path in a temp dir that does not exist gives defaults and no problems.

In `crates/proto/src/paths.rs`, under the existing `ENV_LOCK`: `config_env_override` sets `ANTHREX_CONFIG=/tmp/x/c.toml` and asserts `config_path()` returns it. The existing `default_data_dir_ends_with_anthrex` must remove `ANTHREX_CONFIG` first. Add `lock_path` to `env_overrides_socket_and_data_dir`.

**Change.** Implement decisions 1 to 6 and the `config` interface. Read the file into a `toml::Table` and read each known key with its own validation. Walk the table to report unknown keys, skipping `orchestrator`. Add `lock_path()`.

**Acceptance.** `cargo test -p anthrex-config -p anthrex-proto` passes. Nothing else uses the crate yet.

### M6.2 Lock file, umask, socket unlink, `daemon stop` waits, and the lifecycle tests

**Files.** Create `crates/daemon/src/lockfile.rs` and `crates/daemon/tests/lifecycle.rs`. Modify `crates/daemon/src/lifecycle.rs`, `crates/daemon/src/lib.rs`, `crates/daemon/Cargo.toml` (add `config`) and `crates/cli/src/main.rs`.

**Tests first.**
- In `lockfile.rs`: `second_acquire_fails_while_held`. Acquire in a temp dir. A second `acquire(dir, Duration::ZERO)` fails, and the error contains `another anthrex daemon`. Drop the first; a third acquire succeeds. `flock` locks belong to the open file description, so this works inside one process on macOS and Linux.
- `acquire_waits_for_release`: hold the lock, drop it from a thread after 300 ms, and `acquire(dir, 2 s)` succeeds within 1.5 s.
- `wait_released_reports_both_outcomes`: `wait_released` is false after 300 ms while the lock is held, and true as soon as it is dropped.
- In `lifecycle.rs` unit tests: `bind_restores_the_umask`. Set the umask to `0o022`, call `bind_socket`, assert the socket mode is `0o600` and the umask reads back as `0o022`. Read it with `umask(x)`, then restore.
- In `crates/daemon/tests/lifecycle.rs`, new. Each test uses a temp dir for the socket and data, a config path that does not exist, and `lock_wait: Duration::ZERO`. It runs `daemon::run` on a `tokio::spawn`ed task and talks to it with raw frames, like `crates/daemon/tests/server.rs`.
  - `run_starts_serves_and_stops_cleanly`: wait up to 5 s for the socket. `Hello` gives a `Welcome` with no windows. Create a shell window. Send `Shutdown`. `run` returns `Ok` within 10 s. The socket and `daemon.pid` are gone. `DaemonLock::acquire(data_dir, ZERO)` succeeds.
  - `second_daemon_with_the_same_data_dir_is_refused`: with the first daemon running, a second `run` with the same data dir and a different socket path returns `Err` containing `another anthrex daemon`. The second socket path was never created. The first socket still answers `Hello`.
  - `a_stopping_daemon_leaves_a_replacement_socket_alone`: after the daemon binds, unlink the socket path and bind a plain `std::os::unix::net::UnixListener` at the same path. Send `Shutdown` through a connection opened before the swap. After `run` returns, the path still exists and the plain listener still accepts.

**Change.** Implement decisions 24 to 27. The new order at the top of `run`:
1. Create the data dir.
2. `DaemonLock::acquire`.
3. `init_logging`, now with `try_init`.
4. `prepare_socket`, `bind_socket`, record `(dev, ino)`.
5. Write `daemon.pid`.

The end of `run` unlinks by inode. Add `config_path` and `lock_wait` to `DaemonOptions`. The CLI fills them from `proto::paths::config_path()` and `daemon::LOCK_WAIT`. `anthrex daemon stop` waits with `wait_released`, as in decision 27.

**Acceptance.** All tests pass. `python3 scripts/pty-smoke.py` still passes.

### M6.3 Log rotation and the handshake timeout

**Files.** Create `crates/daemon/src/logfile.rs`. Modify `crates/daemon/src/lifecycle.rs` (`init_logging`), `crates/daemon/src/server.rs`, `crates/proto/src/lib.rs`, `crates/tui/src/connection.rs`, `crates/cli/src/client.rs`, `crates/daemon/tests/server.rs` and `crates/tui/tests/connection.rs`.

**Tests first.**
- In `logfile.rs`: `rotates_by_size_and_keeps_n_files`. `RotatingFile::open(dir, "daemon.log", 1024, 3)`. Write 40 lines of 100 bytes, each `line-NN` padded. Afterwards `daemon.log`, `daemon.log.1` and `daemon.log.2` exist and `daemon.log.3` does not. Every file is at most 1024 bytes. The last line written is in `daemon.log`. No line is split across files.
- `appends_to_an_existing_log`: create `daemon.log` holding 1000 bytes, open with a 1024 limit, and write 100 bytes. The file rotates first, and `daemon.log.1` holds the original 1000 bytes.
- `an_oversized_write_goes_whole_into_a_fresh_file`: one 5000-byte write with a 1024 limit lands intact in a new `daemon.log`.
- In `crates/daemon/tests/server.rs`: `a_client_that_never_says_hello_is_dropped`. Connect and send nothing. A read on the socket returns EOF after at least 4.5 s and at most 7 s.
- In `crates/tui/tests/connection.rs`: `connect_times_out_on_a_silent_server`. Bind a plain `tokio::net::UnixListener` that accepts and never writes. `Connection::connect` fails within 7 s with an error containing `timed out`.

**Change.** Implement decisions 28 and 29. `init_logging` builds `RotatingFile::open(data_dir, "daemon.log", LOG_MAX_BYTES, LOG_KEEP)` and passes it to `tracing_appender::non_blocking`, replacing `tracing_appender::rolling::never`.

**Acceptance.** All tests pass. A daemon started by hand still writes `daemon.log`.

### M6.4 The state file module

**Files.** Create `crates/daemon/src/state.rs`, but not the persister yet. Modify `crates/daemon/src/lib.rs` and `crates/daemon/Cargo.toml` (add `serde` and `serde_json`).

**Tests first.** All in `state.rs`, each using a temp data dir:
- `save_then_load_round_trips`: a `StateFile` with two records, one of them fully populated. `save`, then `load`, gives an equal value and no warnings.
- `save_is_atomic_and_private`: after `save`, `state.json.tmp` does not exist and `state.json` has mode `0o600`. A leftover garbage `state.json.tmp` from before is replaced without error.
- `missing_file_loads_empty`: an empty `windows` list, `next_id == 1` and no warnings.
- `corrupt_file_is_moved_aside`: write `{not json`. `load` gives an empty state and one warning naming the new path. `state.json` is gone, and exactly one `state.json.corrupt-*` file holds the original bytes. A second corrupt load in the same second produces a name ending in `-1`.
- `newer_version_is_moved_aside`: `{"version": 3, "next_id": 1, "windows": []}` ends up as `state.json.unsupported-*`.
- `version_1_file_loads`: a hand-written version-1 file with the core spec 3.6 fields, one record without `session_id` and one with `"status": "sleeping"`. It loads with `project == None`, `run == None`, the missing `session_id == None`, and the unknown status as `Exited`.
- `bad_records_are_skipped`: four records: a valid one, one with `"runtime": "perl"`, one that repeats the first record's name, and a second valid record positioned after both bad ones (order-independence: a rejected record must not poison the records that follow it, decision 12's reason for rejecting per-record rather than per-file). The loaded state holds both valid records, with two warnings.
- `next_id_is_never_below_a_loaded_id`: `next_id: 2` with a record of id 7 loads as `next_id == 8`.
- `record_json_shape`: serializing the example record from decision 8 matches the example JSON, field names and order included.

**Change.** Implement `StateFile`, `WindowRecord`, `WorktreeRecord`, `load` and `save` as decisions 8, 10, 12 and 13 describe. `load` parses to `serde_json::Value` first, checks `version` and `next_id`, then deserializes each window record on its own. For the timestamp use `SystemTime::now().duration_since(UNIX_EPOCH)`.

**Acceptance.** All tests pass. Nothing calls the module yet.

### M6.5 Restored windows and the persister

**Files.** Modify `crates/daemon/src/manager.rs`, `crates/daemon/src/state.rs` (add `spawn_persister`), `crates/daemon/src/lifecycle.rs`, `crates/daemon/tests/manager.rs` and `crates/daemon/tests/lifecycle.rs`. If `manager.rs` would pass about 600 lines, turn it into `crates/daemon/src/manager/mod.rs`, and put the restore and restart code in `crates/daemon/src/manager/restore.rs`.

  *Correction (fix wave 4, item 8, M6.5 review): leave `Entry` private, not `pub(super)`.* `Entry` is declared in `manager/mod.rs`; every module that names it (`manager/create.rs`, `manager/remove.rs`, `manager/restore.rs`) is a descendant of `manager`, and Rust already gives a descendant access to a private item of an ancestor module — `pub(super)` is not needed for that. Worse, `pub(super)` on an item declared at the crate's `manager/mod.rs` resolves to the *crate root*, so it would expose `Entry` to every module in `anthrex-daemon`, wider than intended. The brief's instruction rested on a wrong model of Rust visibility (that a sibling file needs `pub(super)` to see a parent's private item); the implementation left `Entry` private, correctly.

**Tests first.**
- In `crates/daemon/tests/manager.rs`: `restored_windows_are_exited_and_viewable`. `restore` a `StateFile` with one shell record (cwd a temp dir, session id `s-1`, name `kept`). `list()` shows it with status `Exited`, exit reason `daemon restarted`, the same id and name, and session id `s-1`. `attach` returns a snapshot whose text, fed into a `vt100::Parser`, contains `this window stopped when the daemon restarted` and `resumes session s-1`. `write_input` fails with a message containing `not running`. `kill` returns `Ok`. `create` with the name `kept` fails as a duplicate. The next created window gets an id greater than the restored one.
- `state_snapshot_reflects_the_window_table`: create two shell windows and rename one. `state_snapshot()` has `version == 2`, both records with the current names and runtime `shell`, `next_id == 3`, `runs` empty, and a nonzero `created_at`.
- `persister_writes_changes_within_a_second`: start `spawn_persister` on a temp path, create a window, and within 1 s `load(path)` lists it. Rename it 20 times in a loop without sleeping; within 1 s the file shows the last name. Cancel the token and await the handle.
- In `crates/daemon/tests/lifecycle.rs`: `windows_survive_a_daemon_restart_as_exited`. First `run`: create a shell window named `keep`, rename it to `kept`, send `Shutdown`, wait for `run` to return. `state.json` holds `kept`. Second `run` with the same data dir: `Welcome` lists `kept`, `Exited`, reason `daemon restarted`. `Subscribe` gives the placeholder `Snapshot`.

**Change.**
- Introduce `Process` (decision 14) and route `write_input`, `resize`, `attach`, `snapshot`, `signal` and `remove` through it.
- Add `created_at: SystemTime` and `restarting: bool` to `Entry`.
- Implement `restore`: it builds dormant entries, sets `next_id` and publishes once. Implement `state_snapshot`.
- Implement `spawn_persister` as in decision 9.
- In `run`, load the state (decision 12) and call `restore` before `prepare_socket`, then start the persister after `bind_socket`. Follow the shutdown order in decision 11. Log every warning returned by `load`.

**Acceptance.** All tests pass. By hand: start a daemon with isolated variables, create a window, stop it, start again, and `anthrex ls` shows the window as exited.

### M6.6 Runtime commands from config and the resume argv

**Files.** Modify `crates/daemon/src/launch/mod.rs`, `crates/daemon/src/launch/claude.rs`, `crates/daemon/src/launch/codex.rs`, `crates/daemon/src/manager.rs` (`ManagerConfig`), `crates/daemon/src/lifecycle.rs`, and every construction of `ManagerConfig` or `LaunchContext` in `crates/daemon/tests/` and `crates/tui/tests/`.

**Tests first.** In `manager.rs`, extending milestone 3's `bin_overrides_come_from_the_environment_variables`:
- `commands_resolve_env_over_config_over_default`: `ManagerConfig::from_vars` with a closure environment and a `config::Runtimes`. Nothing set gives `claude` and `codex`. Config values are used when there is no environment value. `ANTHREX_CLAUDE_BIN` wins over config for Claude only. `codex_bypass_hook_trust` is copied from the config.

In `launch/`:
- `bypass_hook_trust_adds_the_flag_and_drops_trust_hashes` (in `codex.rs`): with `codex_bypass_hook_trust: true` and `codex_hook_source: None`, the args contain `--dangerously-bypass-hook-trust` right after the five `-c` flags, then the eight `hooks.<Event>` flags, and no `hooks.state` flag. With it false and `codex_hook_source: None`, neither appears, as in milestone 3.
- `claude_resume_adds_resume_and_drops_the_prompt`: a spec with model `opus` and prompt `fix it`, and `resume: Some("sess-1")`. The args end with `["--model", "opus", "--resume", "sess-1"]`, and neither `--` nor `fix it` appears. Milestone 3's `--name` and `--settings` stay first.
- `codex_resume_puts_resume_last_and_drops_the_prompt`: a spec with model `m1` and prompt `hello`, and `resume: Some("thr-1")`. The last two args are `["resume", "thr-1"]`. `-C <cwd>` and `-m m1` come before them. Neither `--` nor `hello` appears.
- `shell_ignores_resume`.
- Update the existing `launch/` tests for the new `LaunchContext` fields. Milestone 3's `programs_come_from_the_context` keeps covering which program runs.

**Change.** Implement decisions 4 (`runtimes.codex.bypass_hook_trust`), 6, 15 and 16. `lifecycle::run` loads the config with `config::load(&opts.config_path)` on `spawn_blocking`, logs each problem at `warn`, and builds the manager with `ManagerConfig::from_env(socket, shell, &config.runtimes)`. Run and record, in "Implementation notes":

```bash
codex --version
codex resume --help
codex -C /tmp -m gpt-x -c 'tui.notification_method="bel"' resume --help; echo "rc=$?"
claude --version
claude --help | grep -n -E -- '--resume|--name'
```

If `codex resume --help` does not list `-m, --model`, drop `-m` on resume only, and change `codex_resume_puts_resume_last_and_drops_the_prompt` to assert its absence.

**Acceptance.** All tests pass. No test uses a real `claude` or `codex`.

### M6.7 Restart in the daemon

**Files.** Modify `crates/daemon/src/manager.rs` (or `manager/restore.rs`), `crates/daemon/src/server.rs`, `crates/daemon/tests/manager.rs` and `crates/daemon/tests/lifecycle.rs`. Create `crates/cli/tests/persistence.rs`.

**Tests first.**
- In `crates/daemon/src/server.rs` tests: `forwarder_reattaches_when_the_channel_closes`. Drive `forward_output_from` with a broadcast receiver whose sender the test drops. The `reattach` closure returns a second window's `Attachment` the first time and `Err` the second time. The forwarder sends one `Snapshot` built from the second attachment, forwards that window's output, and ends when that channel also closes and `reattach` fails.
- In `crates/daemon/tests/manager.rs`:
  - `restart_of_a_restored_shell_runs_it_again`: restore a shell record, `restart(id).await` is `Ok`, and within 5 s the status is not `Exited`. `write_input(b"echo re-$((2+3))\n")` shows `re-5` on the screen within 5 s.
  - `restart_of_a_live_window_replaces_the_process`: create a shell window and type `echo pid-$$` with Enter. Read the pid from the screen text. `restart` returns `Ok`. Type the same command again: within 5 s the screen shows a different `pid-` number, and the status is not `Exited`.
  - `concurrent_restart_is_refused`: two `restart` calls on a live window started together. One returns `Ok`. The other fails with `already restarting`.
  - `restart_needs_the_directory`: restore a record whose cwd does not exist. `restart` fails with `directory does not exist`.
- In `crates/daemon/tests/lifecycle.rs`, with a config file that sets `runtimes.claude.command` and `runtimes.codex.command` to a temp script `argv.sh` (mode `0o755`). The script prints `ARGV:` and then each argument as ` [arg]` on one line, then runs `exec sleep 60`:
  - `restart_resumes_claude_and_codex_sessions`: pre-write a version-2 `state.json` with a Claude record (session `sess-abc`, model `opus`, prompt `do it`) and a Codex record (session `thr-1`, model `m1`, prompt `go`). Start `run`. For each window, `Subscribe` (placeholder `Snapshot`), then `Restart`. The reply is `Ack { request: "restart" }`. A new `Snapshot` or `Output` follows on the same connection. The Claude window's screen shows `[--resume] [sess-abc]` and no `[do it]`. The Codex window's screen ends its argv line with `[resume] [thr-1]`, shows `[-m] [m1]` before `[resume]`, and has no `[go]`.
- In a new `crates/cli/tests/persistence.rs`, because hooks need the real `anthrex` binary: `session_id_learned_from_a_hook_is_saved`. Start milestone 3's `support::TestDaemon` with a script that fires `SessionStart` and then `read_line`. Create a Claude window. Within 2 s, `<TestDaemon::data_dir()>/state.json` holds a non-null `session_id` for that window, equal to `WindowInfo.session_id` from `ListWindows`.
  - `restart_request_does_not_block_the_connection`: restart a live shell window, which takes the kill grace. While it runs, send `ListWindows` on the same connection; the reply arrives within 500 ms, before the restart's `Ack`.

**Change.** Implement decisions 17 to 21. `WindowManager::restart` does the following:
1. Validate under the lock and set `restarting`.
2. Kill if live, then wait outside the lock.
3. Under the lock, check the cwd, build the plan with `resume = session_id`, `Window::spawn`, swap the process, reset the status fields, publish, and clear `restarting` on every path.

In `handle_client`, replace the `Restart` error arm with the spawned task from decision 20. In `forward_output_from`, handle `Closed` as in decision 21.

**Acceptance.** All tests pass. `ClientMsg::Restart` no longer answers "not supported".

### M6.8 Rename rules and the CLI commands

**Files.** Modify `crates/daemon/src/manager.rs`, `crates/daemon/tests/manager.rs`, `crates/cli/src/main.rs`, `crates/cli/src/client.rs` and `crates/cli/Cargo.toml` (add `config`).

**Tests first.**
- In `crates/daemon/tests/manager.rs`: `names_are_validated_on_create_and_rename`. A 65-character name, a name with `\x1b`, and `"  "` are each refused by both `create` and `rename`, with the messages from decision 22. A 64-character name is accepted. `rename` to the window's own current name succeeds.
- In `crates/cli/src/main.rs` tests (new module): `rename_and_restart_parse`. `Cli::try_parse_from(["anthrex", "rename", "3", "new name"])` and `["anthrex", "restart", "api"]` parse into the new variants. `["anthrex", "new"]` parses with `runtime == None`.

**Change.** Implement decision 22 in the manager. Add `Command::Rename { target, name }` and `Command::Restart { target }`. They resolve the target with `resolve_target` and call `expect_ack`. Restart uses `request_with_timeout(.., 15 s)`. Make `New { runtime }` an `Option<RuntimeArg>` that falls back to `config::load(&proto::paths::config_path())`'s `default_runtime`, printing that config's problems to stderr (decision 7).

**Acceptance.** All tests pass. By hand, with isolated variables: `anthrex rename 1 foo` then `anthrex ls` shows `foo`. `anthrex restart foo` exits 0.

### M6.9 Client settings from config

**Files.** Run `git mv crates/tui/src/app.rs crates/tui/src/app/mod.rs` and `git mv crates/tui/src/app_tests.rs crates/tui/src/app/tests.rs` (decision 39). Create `crates/tui/src/settings.rs`. Modify `crates/tui/src/tree.rs`, `crates/tui/src/lib.rs`, `crates/tui/src/theme.rs`, `crates/tui/src/ui/mod.rs`, `crates/tui/src/ui/modal.rs`, `crates/tui/src/ui/sidebar.rs`, `crates/tui/src/ui/statusbar.rs`, `crates/tui/src/ui/terminal.rs`, `crates/tui/Cargo.toml` (add `config`) and `crates/cli/src/main.rs`.

**Tests first.**
- In `settings.rs`: `from_config_maps_every_field`. The example config maps to prefix `(Char('a'), CONTROL)`, label `C-a`, accent `Color::Rgb(0xf5, 0xc2, 0xe7)`, sidebar width 40, and so on. `UiSettings::from_config(&Config::default()) == UiSettings::default()`. A `None` sidebar width maps to the built-in width, `ui::DEFAULT_SIDEBAR_WIDTH`.
- In `app/tests.rs`:
  - `custom_prefix_drives_the_keymap`: with prefix `C-a`, `C-a j` switches windows and `C-b` goes to the PTY as byte `0x02`.
  - `scrollback_follows_settings`: with `scrollback_lines = 10`, after 50 lines of output, scrolling up by 100 stops at offset 10.
  - `bells_follow_settings`: `bell_attention = true` and `bell_done = false`. A background window going to `Attention` returns `Effect::Bell`, and one going to `Done` does not. The focused window never rings.
  - `new_window_uses_the_default_runtime`: with `default_runtime = Claude`, `C-b c` opens `Modal::NewAgent` (milestone 5's new-agent dialog) with its form preselecting Claude, where `App::new_agent_defaults` (`crates/tui/src/app/modal_keys.rs`) hard-codes `Runtime::Claude` today.
  - `config_problems_open_a_notice`: `App` created from `TuiOptions`-like input with one problem has `Modal::Notice` open. Any key closes it and sends nothing to the PTY.
- In `ui/mod.rs` tests:
  - `help_and_hints_use_the_configured_prefix`: with prefix `C-a`, the help overlay and the status bar show `C-a ?` and no `C-b`.
  - `layout_uses_the_configured_sidebar_width`: a client built with `sidebar_width = 40` draws a 40-column sidebar. Assert `App::sidebar_width` starts at 40 and `C-b <` still steps it by 4.
  - `accent_colours_the_focused_border`: the focused main border cell's foreground is the configured accent, read from the `TestBackend` buffer.
- In `tree.rs` tests: `finished_subagents_older_than_the_setting_are_hidden`. With `keep_finished_secs = 120`, a `Done` sub-agent with `ended_secs` 150 has no row and one with `ended_secs` 60 has one; a `Running` sub-agent always has one. With the default 300, both finished rows show.

**Change.** Implement decisions 37 to 39 and the `bell.*`, `default_runtime`, `scrollback_lines` and `ui.tree_keep_finished_secs` rows of decision 4. `tree::TreeState` gains `pub keep_finished_secs: u64` (default 300), set from `settings.tree_keep_finished_secs`; `tree::build` skips a finished sub-agent whose `ended_secs` exceeds it, and its descendants are attached to the nearest shown ancestor, as orphans are. The new-agent form's first open uses `settings.default_runtime` in place of Claude. `layout(area, sidebar_width)` already takes a `sidebar_width: u16` parameter, so initialise `App::sidebar_width` from `settings.sidebar_width` instead of the constant. `modal::render` takes `&App` so it can reach the accent and the prefix label. `HELP` becomes a function of the prefix label. `lib.rs` writes the bell byte for `Effect::Bell`. The CLI's `attach` loads the config once, builds `UiSettings::from_config`, formats the problems, and passes them in `TuiOptions`. Update every existing `App::new` call in tests to pass `UiSettings::default()`.

**Acceptance.** All tests pass. No file under `crates/tui/src/app/` or `crates/tui/src/ui/` does I/O. `crates/tui/src/app/mod.rs` is under 600 lines.

### M6.10 Rename, restart and quit keys

**Files.** Create `crates/tui/src/app/prompt.rs`. Modify `crates/tui/src/keymap.rs`, `crates/tui/src/app/mod.rs`, `crates/tui/src/app/tests.rs`, `crates/tui/src/ui/modal.rs` and `crates/tui/src/lib.rs`.

**Tests first.**
- In `keymap.rs`: `rename_restart_and_reconnect_keys`. Prefix then `,` gives `Run(RenameWindow)`, `R` with Shift gives `Run(RestartWindow)`, and `r` gives `Run(Reconnect)`.
- In `app/tests.rs`:
  - `rename_prompt_edits_and_sends`: `C-b ,` opens `Modal::Rename` prefilled with the focused window's name. Backspace removes a character, typed characters append, and Enter sends `Rename { window_id, name }` and closes the modal. Esc closes it and sends nothing.
  - `rename_prompt_checks_before_sending`: renaming to another window's name, to spaces, or to 65 characters keeps the modal open with `error` set, and sends nothing.
  - `restart_of_an_exited_window_is_immediate`: the focused window is `Exited`. `C-b R` sends `Restart` and toasts `restarting a`.
  - `restart_of_a_live_window_asks_first`: the focused window is `Working`. `C-b R` opens `Confirm` with `PendingAction::Restart(id)`. `y` sends `Restart`.
  - `stop_daemon_waits_for_confirmation`: `C-b Q`, then `y`, gives only `Send(Shutdown)`. `on_daemon(Bye)` gives no `Quit`. `on_link_lost(..)` gives `[Quit]`.
  - `stop_daemon_send_failure_is_reported`: after `C-b Q y`, `on_send_failed(&Shutdown)` clears `stopping` and toasts `could not reach the daemon; run anthrex daemon stop`. A later `on_link_lost` does not quit.
  - `stop_daemon_times_out`: with `stopping` set 6 s in the past (set the field directly), `on_tick` clears it and toasts `the daemon did not confirm the shutdown; run anthrex daemon stop`.
- In `ui/mod.rs` tests: `rename_modal_renders`: the modal shows ` rename `, the input text with a cursor block, and the error line when set.

```
╭ rename ─────────────────────────╮
│ api-worker█                     │
│ a window named 'api' exists     │
│ Enter = rename    Esc = cancel  │
╰─────────────────────────────────╯
```

**Change.** Implement decisions 22 (client side), 23 and 36. `PendingAction::StopDaemon` now yields `[Send(Shutdown)]` and sets `stopping`. Add the three commands to `HELP`: `<prefix> ,` rename, `<prefix> R` restart, `<prefix> r` reconnect.

**Acceptance.** All tests pass.

### M6.11 Reconnect

**Files.** Create `crates/tui/src/app/link.rs`, `crates/tui/src/reconnect.rs` and `crates/tui/src/spawn.rs` (moved from `crates/cli/src/spawn.rs` with `git mv`). Modify `crates/tui/src/lib.rs`, `crates/tui/src/app/mod.rs`, `crates/tui/src/app/tests.rs`, `crates/tui/src/ui/statusbar.rs`, `crates/tui/Cargo.toml` (add `libc`, needed by `spawn.rs`), `crates/cli/src/main.rs` and `crates/tui/tests/connection.rs`.

**Tests first.**
- In `reconnect.rs`: `retry_schedule_timing`, with synthetic `Instant`s (`t0 + Duration`).
  - `after_drop(t0)` is not due at `t0 + 1.9 s` and is due at `t0 + 2 s`.
  - `after_failure(t0 + 2.1 s)` returns true and makes the next attempt due at `t0 + 4.1 s`.
  - `after_failure(t0 + 30 s)` returns false.
  - `manual(t1)` is due at `t1` and gives up only at `t1 + 30 s`.
- In `app/tests.rs`:
  - `link_lost_then_reconnected_resubscribes_the_focused_window`: focus window 2 and apply a snapshot. `on_link_lost("x")` sets `Reconnecting { attempts: 0 }` and toasts. `on_reconnect_failed("refused", false)` gives `attempts == 1`. `on_reconnected(vec![win 1, win 2])` returns exactly one `Send(Subscribe { window_id: 2, .. })`, sets `Connected`, and toasts `reconnected`.
  - `reconnect_when_the_focused_window_is_gone_focuses_a_neighbour`: exactly one `Subscribe`, for the neighbour.
  - `giving_up_sets_lost_and_c_b_r_retries`: `on_reconnect_failed(.., true)` gives `Lost`. `C-b r` returns `[Effect::Reconnect]` and sets `Reconnecting`. While `Connected`, `C-b r` returns nothing and toasts `connected`.
  - `a_refused_subscribe_is_retried_on_tick`: `focus(2)` emits `Subscribe`. `on_send_failed(&that Subscribe)` makes the next `on_tick` emit `Subscribe { window_id: 2 }` again, and the tick after that emits none. `focus(2)` right after the failure also emits it: the early return no longer blocks the retry.
  - `commands_while_disconnected_toast`: in `Reconnecting`, `on_send_failed(&Kill { .. })` toasts `not connected; C-b r to reconnect`. `on_send_failed(&Input { .. })` toasts nothing.
- In `ui/mod.rs` tests: `statusbar_shows_reconnect_state`: `Reconnecting { attempts: 3 }` renders `DISCONNECTED` and `reconnecting (attempt 3)`. `Lost` renders `C-b r to reconnect`.
- In `crates/tui/tests/connection.rs`: `attempt_fails_without_a_daemon_and_succeeds_with_one`. `reconnect::attempt(socket, None)` on a path with no listener returns `Err`. After `start_daemon()` on that path, `attempt` returns a `Connection` whose `windows` match the daemon's.

**Change.** Implement decisions 30 to 35.

In `lib.rs`:
- `conn` becomes `Option<Connection>`. Receiving uses a helper that stays pending forever while `conn` is `None`.
- A closed receive drops the connection, calls `app.on_link_lost(reason)`, where the reason is the last `Bye` reason or `connection closed`, and sets `schedule = Some(RetrySchedule::after_drop(now))` unless the effects contain `Quit`.
- A `select!` branch fires when the schedule is due and no attempt is in flight, and spawns `reconnect::attempt(socket, None)`.
- A branch awaits the in-flight `JoinHandle`. On `Ok` it installs the connection and applies `app.on_reconnected(conn.windows.clone())`. On `Err` it calls `schedule.after_failure(now)` and `app.on_reconnect_failed(err, gave_up)`.
- `Effect::Reconnect` sets `schedule = Some(RetrySchedule::manual(now))` and spawns `reconnect::attempt(socket, opts.daemon_exe.clone())` at once, unless one is in flight.
- `apply` reports refused sends to `app.on_send_failed`.

`DaemonMsg::Bye` now only records the reason and toasts. The link changes when the channel closes. The CLI uses `tui::spawn::ensure_daemon(&std::env::current_exe()?, &socket)` and passes `daemon_exe: Some(current_exe)`.

**Acceptance.** All tests pass. By hand, with isolated variables: attach, run `anthrex daemon stop` in another terminal, see `DISCONNECTED`, run `anthrex daemon start` within 30 s, and the client reconnects with the focused window live again.

### M6.12 Smoke stages

**Files.** Modify `scripts/pty-smoke.py`.

**Tests first.** The new stages are the tests. Run the script before the stages exist to confirm the earlier stages still pass with the new `ENV`.

**Change.**
- Add `ANTHREX_CONFIG` to the script's `ENV`, pointing to `/tmp/anthrex-smoke-data/config.toml`. The file is never created, so the user's own config never applies.
- `stop_daemon` keeps its socket wait as a guard, but `anthrex daemon stop` now returns only after the daemon exits.
- Add two stages after stage 9:
  - **Stage 10, persistence and restart.** `anthrex daemon start`. `anthrex ls` lists `shell-1` to `shell-4` with status `exited`. `anthrex rename shell-1 kept` exits 0, and `ls` shows `kept`. `anthrex restart kept` exits 0. Attach, press `C-b 1`, type `echo back-$((1+1))` and Enter, wait for `back-2`, detach.
  - **Stage 11, reconnect.** Attach again and wait for `kept`. Run `anthrex daemon stop` from the script. Wait for `DISCONNECTED` on the screen. Run `anthrex daemon start`. Within 10 s `DISCONNECTED` is gone from the reconstructed screen and `kept` is still listed. Detach with `C-b d`, which must exit 0.
- Stage 9 stays the last stop. Move the final `daemon stop` and status check after stage 11.

**Acceptance.** `python3 scripts/pty-smoke.py` prints `ALL SMOKE STAGES PASSED`. Afterwards `pgrep -fl "anthrex daemon"` shows nothing started by the script.

## Verification

Run the five commands from `AGENTS.md`:

```bash
cargo build --workspace --all-targets
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
python3 scripts/pty-smoke.py
```

Milestone-specific checks:

1. `grep -rn "rolling::never" crates/` finds nothing.
2. `grep -rn '"restart is not supported' crates/` finds nothing.
3. `grep -rn "C-b" crates/tui/src/ui/` finds only defaults and test strings, no hard-coded hint or help text.
4. `wc -l crates/tui/src/app/*.rs crates/daemon/src/*.rs` shows no file over about 600 lines.
5. The new stages 10 and 11 of the smoke script pass.

## Manual check

Use isolated paths in every terminal, and never run `anthrex` without them:

```bash
export ANTHREX_SOCKET=/tmp/m6.sock ANTHREX_DATA_DIR=/tmp/m6-data ANTHREX_CONFIG=/tmp/m6-config.toml
```

1. Write `/tmp/m6-config.toml` with `prefix = "C-a"`, `accent = "#f5c2e7"`, `bell.done = true` and `colour = "red"`. Run `anthrex`. A ` config ` notice lists `colour: unknown key, ignored`. Any key closes it. The help (`C-a ?`) and the status bar say `C-a`. Borders use the pink accent.
2. `anthrex new --runtime claude --name cl --prompt "reply with the word ready"`. Focus it and let it answer. `anthrex new --runtime codex --name cx --prompt "reply with the word ready"`. Focus it and let it complete one turn, which is when Codex reports its thread id.
3. Open `/tmp/m6-data/state.json`. It has `"version": 2`, and both records have a non-null `session_id`. File mode is `0600`.
4. With the client attached, run `anthrex daemon stop` in a second terminal. The client shows `DISCONNECTED` and `reconnecting (attempt N)`, and the last screen stays visible.
5. Within 30 seconds, run `anthrex daemon start`. The client reconnects. `cl` and `cx` show `✕` exited. Focusing one shows the "stopped when the daemon restarted" placeholder with its session id.
6. Focus `cl` and press `C-a R`. Claude starts, and the previous conversation, including "ready", is visible in the resumed session. Focus `cx` and press `C-a R`. Codex resumes the same thread in the same directory, and the sidebar status follows its turns: the terminal-title status works, which shows the `-c` overrides were applied on resume. Record the Claude and Codex versions used.
7. `C-a ,` on `cl`: rename it to `claude-main`. Stop and start the daemon. The name survives. `anthrex restart claude-main` from the second terminal works too.
8. Press `C-a R` on a running window. It asks first. Confirm, and the window restarts in place while the client stays subscribed.
9. Stop the daemon and wait more than 30 seconds. The status bar shows `C-a r to reconnect`. Press `C-a r`. The daemon starts, the client reconnects, and the windows show as exited.
10. While a daemon runs, run `anthrex daemon start --foreground` in the second terminal. It fails within about 5 seconds with `another anthrex daemon is running`, and the first daemon's socket still works (`anthrex ls`).
11. Stop the daemon, run `echo garbage > /tmp/m6-data/state.json`, and start it. `daemon.log` has a warning naming `state.json.corrupt-<n>`, that file holds `garbage`, and the daemon starts empty.
12. `C-a Q`, then `y`. The client quits only after the daemon confirms. `pgrep -fl "anthrex daemon"` shows nothing of yours. Finish with `anthrex daemon stop` using the same variables, then remove `/tmp/m6-data`, `/tmp/m6.sock` and `/tmp/m6-config.toml`.

## Risks and gotchas

1. **Earlier milestones renamed things.** This brief uses the names of the milestone 3, 4 and 5 briefs ("Starting point"). If the merged code differs, search for the brief's name first, then the milestone's diff. Record the mapping under "Implementation notes".
2. **Children inheriting the lock.** If an agent process inherits the `daemon.lock` descriptor, a crashed daemon's orphaned agents keep the lock, and no new daemon can start. You would see `another anthrex daemon is running` with a pid that no longer exists. `std::fs::OpenOptions` sets `O_CLOEXEC`. Do not open the lock with `libc::open` without it.
3. **The umask is process-wide.** For the microseconds between `umask(0o077)` and the restore, any file another thread creates is private. That is harmless, but keep nothing else between the two calls. Tests that read the umask must restore it.
4. **A restart shows a blank or frozen screen.** The forwarder saw `Closed` before the new process was in place, or the dormant sender was not dropped at the swap. The swap must happen in one critical section under the manager lock. The `Process::Dormant` sender must be dropped by the swap and cloned nowhere else. Check `forwarder_reattaches_when_the_channel_closes`.
5. **Watch coalescing.** `tokio::sync::watch` keeps only the latest value, so the persister may skip intermediate states. That is intended: it always saves the latest `state_snapshot()`, not the value it was woken with.
6. **Codex has no thread id until its first completed turn.** `notify` fires only on turn completion (core spec 3.2). A Codex window restarted before its first turn starts fresh, with its initial prompt. That is correct behaviour.
7. **Claude sessions are tied to a directory.** `--resume` works when the restart runs in the same `cwd`, which it does. If the directory moved, the restart fails with `directory does not exist`. Do not try to guess a new directory.
8. **Two `run` calls in one test process.** `tracing_subscriber::fmt().init()` panics the second time, so use `try_init`. The SIGTERM and Ctrl-C handlers can be installed more than once.
9. **Timing-sensitive tests.** The handshake tests take about 5 seconds and the live-restart tests take the kill grace. Use deadline loops with generous upper bounds, as `AGENTS.md` requires, and never rely on a fixed sleep alone.
10. **Protocol mismatch after an upgrade.** A client reconnecting to a daemon built from another version gets a handshake `Error`. That counts as a failed attempt, and once the link is `Lost` its message is shown. The user then runs `anthrex daemon stop` and attaches again.
11. **The persister and the final flush writing at once.** Both use `state.json.tmp`. Await the persister's `JoinHandle` before the final save. Two concurrent writers would produce a torn temp file.
12. **`app.rs` split conflicts.** Milestone 7 also edits the client state, and is not merged yet. If it lands first, rebase the `git mv` onto its layout rather than re-splitting.

## Follow-ups handled

From `docs/superpowers/plans/2026-09-17-anthrex-foundation-followups.md`, "Assignment to milestones", row M6:

| Item | Where |
|------|-------|
| The lifetime lock file | M6.2, decision 24 |
| The unconditional socket unlink at shutdown | M6.2, decision 26 |
| The stale-socket TOCTOU | M6.2, decisions 24 and 26 |
| Umask around bind | M6.2, decision 25 |
| Reconnect, including re-subscribe after a dropped `Subscribe` | M6.11, decisions 30 to 35 |
| `C-b Q` confirms that the shutdown was delivered before quitting | M6.10, decision 36 |
| End-to-end `lifecycle::run` start and stop test | M6.2 (`run_starts_serves_and_stops_cleanly`), extended in M6.5 and M6.7 |
| Log rotation | M6.3, decision 28 |
| Handshake read timeout | M6.3, decision 29 |

## Implementation notes

Twelve tasks, fourteen fix waves, a whole-branch review and a final gate. Every deviation below was raised by an implementer or a reviewer
rather than discovered afterwards.

### Amendments to the decisions in this brief

- **Decision 12 (state file problems).** `state::load` returns `Vec<Problem>` carrying a
  `Severity`, not `Vec<String>`. The decision specifies two severities — `error` for an unreadable
  file, `warn` for corrupt, unsupported or skipped records — and a bare string cannot carry the
  distinction. `config::Problem` deliberately has **no** severity: an unreadable config falls back
  to defaults and loses nothing, while an unreadable state file loses the user's window list. The
  asymmetry is intentional; do not "fix" it into symmetry.
- **Decision 18 (kill wait).** `restart` polls `child_alive`, not status `Exited`. Restart reuses a
  live window id, so a stale `WindowEvent::Exited` from the old process can be applied to the new
  one; `ParserPanicked` reaches status `Exited` without touching `child_alive`. Because
  `child_alive = false` is written in exactly one place, observing it false *is* observing the old
  exit was consumed. The decision's literal wording does not close that race.
- **Decision 22 (name rules).** Extended beyond `char::is_control()` to reject the bidi formatting
  characters U+202A–U+202E and U+2066–U+2069. They are category `Cf`, not `Cc`, so they passed the
  original rule while reordering rendered text — a spoofing surface in a tool that runs several
  agents side by side. The rule is that explicit list, **not** the `Cf` category, because `Cf` also
  contains the zero-width joiner that legitimate emoji names need.
- **Decision 39 (file layout).** `Entry` stays private rather than `pub(super)`: every referencing
  file is a descendant of `manager` and already sees a private ancestor item, and `pub(super)`
  there resolves to the crate root — wider than intended. `stopping`'s logic lives in
  `app/link.rs` as the decision says; it was briefly in `app/lifecycle.rs` and was moved back.

### Deviations from the task list

- `init_logging` returns `anyhow::Result<WorkerGuard>`. `RotatingFile::open` can fail where the old
  rolling-appender constructor could not.
- A restored record's `project` is **never fabricated**. The brief's shape invited falling back to
  `cwd`, but `state_snapshot` writes unconditionally, so the fallback would bake itself into the
  file permanently and no later boot could re-derive it. A re-probe was rejected because the only
  non-blocking slot for it is before the socket bind, which is where a slow `codex` probe broke
  daemon auto-start (see below).
- `bad_records_are_skipped` uses **four** records, not the three the task text said: two survivors
  and two rejections require four, and the fourth must sit *after* both bad ones, since
  order-independence is the property decision 12 exists to provide.
- Several files outside a task's stated list had to change: removing `theme::ACCENT` and adding a
  parameter to `subagent_forest` are breaking changes the brief did not anticipate.
- Module splits beyond those named: `manager/{entry,restore,restart,create}.rs`, `app/windows.rs`,
  `app/lifecycle.rs`, `app/link.rs`, `tree/{forest,names}.rs`, `server_tests.rs`, and several test
  modules. Each was verified as a pure move by direct content comparison, never by relying on
  `git`'s rename detection.
- `scripts/pty-smoke.py` gained two stages beyond the brief: **stage 13** quits through the TUI
  with `C-b Q`, and **stage 12b** covers restart-and-resume against a fake runtime. Both were added
  because the defects below were invisible without them.

### Surprises worth recording

- **`anthrex daemon start` is timing-sensitive to anything before the socket bind.** Moving
  `codex_version::check` (5 s timeout) ahead of the bind broke auto-start entirely, because
  `ensure_daemon` waits only 3 s for the socket. Only the state-file load belongs before the bind.
- **Restart is the first code in this project that reuses a live window id.** Every structure keyed
  by id that assumed one process per id became suspect; the cleanup map produced the same defect
  **five** separate times before this milestone closed it, not three: fix wave 8's structural
  `Drop`-based eviction closed the first three (the timeout refusal, `finish_restart`'s own call,
  and phase C's failure path); fix-wave-12-re-review found a fourth (an unconditional `Drop` could
  evict a foreign record on a bail that landed before phase B ever ran, closed with a
  `phase_b_entered` flag set ahead of the kill call); the final gate found a fifth (that flag was
  set from *reaching the kill call*, not from *the kill call actually inserting a record* —
  `start_cleanup` short-circuits, inserting nothing, whenever an unrelated record already occupies
  `cleanups[id]`, so an attempt could mark itself as owning phase B while owning no record at all,
  and `Drop` would then evict someone else's). Do not claim this closes the class: the guarantee now
  rests on `Restarting::owns_cleanup_record` being *derived* from
  `WindowManager::kill_reporting_insert`'s own report of whether its call actually inserted the
  record (`crates/daemon/src/manager/entry.rs`'s `start_cleanup` returning `Result<bool>`), never
  asserted at the call site ahead of it — a property that holds only as long as every future kill
  path threading into `cleanups[id]` continues to report insertion rather than presence. A new kill
  path that sets ownership before calling, the way this one used to, reopens the same shape.
- **Two Criticals were invisible to a green suite** and were found only by driving the real product
  over a PTY: `restart` worked exactly once per window, and `C-b Q` never quit. Both lived in the
  wiring rather than in any unit — the second because its test called the handler directly,
  bypassing the event-loop guard that contained the bug.
- **A guard at admission does not constrain work already in flight.** `restart` and
  `remove_with_worktree` both needed their `shutting_down` check repeated inside the operation.
- **A bound must exceed the code's own legal worst case, in any language.** Six instances found so
  far, not four: two purely in Rust, then four more in `scripts/pty-smoke.py` at the Rust/Python
  language boundary — `run_cmd`'s calls wrapping `restart`/`daemon stop`, `run_worktree_cli_stage`'s
  worktree `new`/`rm`, `run_worktree_form_stage`'s TUI-driven equivalents of that same worktree
  create/remove (missed by the sweep that had just fixed the CLI stage one function above it), and
  `run_cmd`'s own default `timeout`, numerically equal to `anthrex new`'s worst case (missed by the
  same sweep that rewrote that default's comment and reaffirmed `new` as safe from observed cost
  rather than the budget). Every sweep for this class enumerated a rule narrower than "every wait
  whose bound must exceed a daemon budget" and missed whatever did not match its narrower
  construct. See `docs/timing-budgets.md`.

### Runtime verification (run 2026-09-21 against the binaries installed on this machine)

```
$ codex --version
codex-cli 0.155.0

$ claude --version
2.1.278 (Claude Code)
```

`codex resume --help` **does** list `-m, --model <MODEL>`, so the conditional in task M6.6 — drop
`-m` on resume if the flag is absent — does not apply and `-m` is kept. It also lists
`--dangerously-bypass-hook-trust`, used by decision 4. `claude --help` lists `-r, --resume [value]`
and `-n, --name <name>`. Root options precede the `resume` subcommand for Codex; the full transcript
was captured in the task's report and reproduced independently by its reviewer.

### Accepted residuals

Both are recorded in `docs/superpowers/plans/2026-09-17-anthrex-foundation-followups.md`.

- **The `ensure_daemon` phantom-daemon race** is closed in two layers. The first, `acquire_or_yield`,
  has no timing bet in it and is what prevents a loser becoming a daemon. The second, a 250 ms
  spawn-claim grace, is a bounded heuristic that only reduces how often a redundant child is
  spawned at all; its worst case is a redundant process that exits 0. Measured 0/50 after versus
  100 % before. Whoever revisits this should first reproduce the mechanism at `c1b4555~1` — a clean
  number with no demonstrated failing baseline is the shape of measurement this milestone withdrew
  three times.
- **`handle_hook` is keyed by window id with no per-spawn tag**, so a hook from a replaced process
  can in principle land on its successor. Same shape as the accepted `WindowEvent` staleness; a real
  fix needs a protocol-level generation counter, which is a protocol bump and every client updated.
