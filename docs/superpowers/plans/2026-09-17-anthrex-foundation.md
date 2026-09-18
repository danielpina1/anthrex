# anthrex Foundation (Plan 1 of 5) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A working daemon-backed terminal multiplexer: `anthrex` attaches to a daemon that owns PTY windows (shell, or a plain `claude` / `codex` launch), shows a sidebar plus the focused window, switches between windows, creates and kills them, and detaches leaving everything running.

**Architecture:** Four crates in one workspace. `proto` holds the wire types and framing. `daemon` owns PTYs through `portable-pty`, mirrors each screen with `vt100`, tracks a simple status state machine, and serves a length-prefixed MessagePack protocol over a Unix socket. `tui` is a ratatui client with its own `vt100` parser fed by a snapshot on attach and live bytes after. `cli` is the `anthrex` binary. This plan implements section 11 step 1 of the spec; hook-driven status, dialogs, worktrees, persistence and CI follow in plans 2 to 5.

**Tech Stack:** Rust 2024 edition (rustc 1.92), tokio 1, portable-pty 0.9, vt100 0.16, ratatui 0.30, crossterm 0.29, tui-term 0.3, rmp-serde 1, clap 4.

**Spec:** `docs/superpowers/specs/2026-09-17-anthrex-design.md`

## Global Constraints

- Rust edition `2024`, `rust-version = "1.92"`, one Cargo workspace, crates under `crates/`.
- Crate versions: ratatui 0.30, crossterm 0.29, tui-term 0.3, vt100 0.16, portable-pty 0.9, tokio 1, rmp-serde 1, clap 4, dirs 6.
- Protocol: 4-byte big-endian length prefix, MessagePack body, `MAX_FRAME` = 16 MiB, `PROTO_VERSION` checked in the handshake.
- Socket: macOS `$TMPDIR/anthrex-<uid>/daemon.sock`; Linux `$XDG_RUNTIME_DIR/anthrex/daemon.sock`, fallback `/tmp/anthrex-<uid>/daemon.sock`. Socket directory mode `0700`.
- Data dir: `dirs::data_dir()/anthrex` (`daemon.log`, `daemon.pid`).
- Child environment: inherit plus `TERM=xterm-256color`, `COLORTERM=truecolor`, `ANTHREX_WINDOW_ID`, `ANTHREX_SOCKET`.
- Status transitions for this plan (spec section 3.4 "all" and "Shell" rows, applied to every runtime as the fallback rule): `Spawned` → Starting; `ChildExited` → Exited; `Bell` → Attention; `Focused`: Done → Idle; `InputSent`: Attention → Working; `Output`: Starting/Idle/Done → Working; `Quiet` (3 s without output): Working → Idle.
- Kill: SIGTERM, then SIGKILL after 3 s. Shutdown: SIGTERM all, wait up to 3 s, SIGKILL stragglers.
- UI: sidebar 30 columns, rounded borders, unicode-only glyphs (`◌ ○ ◆ ✓ ✕` and a braille spinner), accent `#89b4fa`, prefix key Ctrl-b.
- Platforms: macOS and Linux only.
- Commits: conventional-commit style subject, body optional, always end with `Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>`.

### Spec addenda decided in this plan

- `DaemonMsg::Ack { request }` is added as the reply to `Kill`, `Remove`, `Rename` and `Unsubscribe` so the CLI can report success. `Input` and `Resize` stay unacknowledged.
- Every child is launched as `/bin/sh -c 'exec "$0" "$@"' <program> <args...>` so a missing binary shows `command not found` inside the window and exits 127 instead of failing the create request.
- `WindowManager::new` takes the shell path explicitly instead of reading `$SHELL` so tests can pin `/bin/sh`.
- `ANTHREX_SOCKET` and `ANTHREX_DATA_DIR` environment variables override the computed paths (used by tests and by the daemon the client spawns).
- `Restart` and `HookEvent` requests are answered with `Error` until plans 2 and 4 implement them.

## File structure

| Path | Responsibility |
|------|----------------|
| `Cargo.toml` | Workspace members, shared package fields, shared dependency versions, release profile |
| `crates/proto/src/lib.rs` | Re-exports and `PROTO_VERSION` |
| `crates/proto/src/types.rs` | `Runtime`, `Status`, `ExitInfo`, `WindowSpec`, `WindowInfo`, `ClientKind` |
| `crates/proto/src/messages.rs` | `ClientMsg`, `DaemonMsg`, `HookSource` |
| `crates/proto/src/codec.rs` | `encode`, `decode`, `read_frame`, `write_frame`, `CodecError`, `MAX_FRAME` |
| `crates/proto/src/paths.rs` | `socket_path`, `data_dir`, `log_path`, `pid_path`, `state_path`, `config_path` |
| `crates/daemon/src/lib.rs` | Module list, re-export of `run` and `DaemonOptions` |
| `crates/daemon/src/launch.rs` | `LaunchPlan`, `LaunchContext`, `plan()` per runtime |
| `crates/daemon/src/status.rs` | `StatusEvent`, pure `next()` transition function |
| `crates/daemon/src/window.rs` | `Window` (PTY, child, VT mirror, output broadcast), `WindowEvent`, `Attachment` |
| `crates/daemon/src/manager.rs` | `WindowManager`: create/list/input/resize/attach/focus/kill/remove/rename/tick/shutdown, change broadcasting |
| `crates/daemon/src/server.rs` | `serve()` accept loop and per-client protocol handling |
| `crates/daemon/src/lifecycle.rs` | `prepare_socket`, logging, pid file, signal handling, `run()` |
| `crates/daemon/tests/window.rs` | PTY integration tests |
| `crates/daemon/tests/manager.rs` | Manager integration tests |
| `crates/daemon/tests/server.rs` | Protocol integration tests over a temp socket |
| `crates/tui/src/lib.rs` | `TuiOptions`, `run()`, event loop |
| `crates/tui/src/connection.rs` | Socket client with reader/writer tasks |
| `crates/tui/src/keymap.rs` | `encode_key`, `Keymap` prefix state machine, `Command` |
| `crates/tui/src/app.rs` | `App` state and pure update functions returning `Effect`s |
| `crates/tui/src/theme.rs` | Colours, glyphs, spinner, styles |
| `crates/tui/src/ui/mod.rs` | `Layout`, `layout()`, `draw()` |
| `crates/tui/src/ui/sidebar.rs` | Agent cards, footer, `hit_test` |
| `crates/tui/src/ui/terminal.rs` | Focused window block and `PseudoTerminal` |
| `crates/tui/src/ui/statusbar.rs` | Key hints, prefix indicator, toast |
| `crates/tui/src/ui/modal.rs` | Confirm and help overlays |
| `crates/tui/tests/connection.rs` | Client against an in-process daemon |
| `crates/cli/src/main.rs` | clap definitions and command dispatch |
| `crates/cli/src/client.rs` | `CliClient`, `resolve_target`, `format_table` |
| `crates/cli/src/spawn.rs` | `ensure_daemon`, detached daemon spawn |
| `README.md` | Build, run, keys, smoke checklist |

---

### Task 1: Workspace scaffold

**Files:**
- Create: `Cargo.toml`, `crates/proto/Cargo.toml`, `crates/proto/src/lib.rs`, `crates/daemon/Cargo.toml`, `crates/daemon/src/lib.rs`, `crates/tui/Cargo.toml`, `crates/tui/src/lib.rs`, `crates/cli/Cargo.toml`, `crates/cli/src/main.rs`
- Modify: `.gitignore` (already contains `target/`)

**Interfaces:**
- Produces: workspace dependency aliases `proto`, `daemon`, `tui` usable as `proto.workspace = true` and imported as `use proto::...`; `proto::PROTO_VERSION: u32`.

- [ ] **Step 1: Write the workspace manifest**

`Cargo.toml`:

```toml
[workspace]
resolver = "3"
members = ["crates/proto", "crates/daemon", "crates/tui", "crates/cli"]

[workspace.package]
version = "0.1.0"
edition = "2024"
rust-version = "1.92"
license = "MIT"

[workspace.dependencies]
proto = { path = "crates/proto", package = "anthrex-proto" }
daemon = { path = "crates/daemon", package = "anthrex-daemon" }
tui = { path = "crates/tui", package = "anthrex-tui" }
anyhow = "1"
thiserror = "2"
serde = { version = "1", features = ["derive"] }
serde_bytes = "0.11"
serde_json = "1"
rmp-serde = "1"
tokio = { version = "1", features = ["rt-multi-thread", "macros", "net", "io-util", "sync", "time", "signal"] }
tokio-util = "0.7"
bytes = "1"
dirs = "6"
libc = "0.2"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
tracing-appender = "0.2"
portable-pty = "0.9"
vt100 = "0.16"
ratatui = "0.30"
crossterm = { version = "0.29", features = ["event-stream"] }
tui-term = "0.3"
clap = { version = "4", features = ["derive"] }
futures = "0.3"
tempfile = "3"

[profile.release]
lto = "thin"
codegen-units = 1
strip = true
```

- [ ] **Step 2: Write the four crate manifests and empty sources**

`crates/proto/Cargo.toml`:

```toml
[package]
name = "anthrex-proto"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true

[lib]
name = "proto"
path = "src/lib.rs"

[dependencies]
serde.workspace = true
serde_bytes.workspace = true
serde_json.workspace = true
rmp-serde.workspace = true
thiserror.workspace = true
tokio.workspace = true
dirs.workspace = true
libc.workspace = true
```

`crates/proto/src/lib.rs`:

```rust
//! Wire types shared by the daemon, the TUI client, and the CLI.

/// Bumped whenever a message shape changes incompatibly.
pub const PROTO_VERSION: u32 = 1;
```

`crates/daemon/Cargo.toml`:

```toml
[package]
name = "anthrex-daemon"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true

[lib]
name = "daemon"
path = "src/lib.rs"

[dependencies]
proto.workspace = true
anyhow.workspace = true
tokio.workspace = true
tokio-util.workspace = true
bytes.workspace = true
portable-pty.workspace = true
vt100.workspace = true
libc.workspace = true
tracing.workspace = true
tracing-subscriber.workspace = true
tracing-appender.workspace = true

[dev-dependencies]
tempfile.workspace = true
```

`crates/daemon/src/lib.rs`:

```rust
//! The anthrex daemon: owns PTY windows and serves them over a Unix socket.
```

`crates/tui/Cargo.toml`:

```toml
[package]
name = "anthrex-tui"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true

[lib]
name = "tui"
path = "src/lib.rs"

[dependencies]
proto.workspace = true
anyhow.workspace = true
tokio.workspace = true
ratatui.workspace = true
crossterm.workspace = true
tui-term.workspace = true
vt100.workspace = true
futures.workspace = true
dirs.workspace = true

[dev-dependencies]
daemon.workspace = true
tokio-util.workspace = true
tempfile.workspace = true
```

`crates/tui/src/lib.rs`:

```rust
//! The anthrex terminal client.
```

`crates/cli/Cargo.toml`:

```toml
[package]
name = "anthrex"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
description = "A terminal multiplexer for coding agents"

[[bin]]
name = "anthrex"
path = "src/main.rs"

[dependencies]
proto.workspace = true
daemon.workspace = true
tui.workspace = true
anyhow.workspace = true
clap.workspace = true
tokio.workspace = true
libc.workspace = true
crossterm.workspace = true
```

`crates/cli/src/main.rs`:

```rust
use clap::Parser;

#[derive(Parser)]
#[command(name = "anthrex", version, about = "A terminal multiplexer for coding agents")]
struct Cli {}

fn main() {
    let _cli = Cli::parse();
    println!("anthrex {} (proto {})", env!("CARGO_PKG_VERSION"), proto::PROTO_VERSION);
}
```

- [ ] **Step 3: Build and run**

Run: `cargo build && cargo run -p anthrex -q`
Expected: build succeeds; output `anthrex 0.1.0 (proto 1)`.

Run: `cargo test`
Expected: all crates report `0 passed` with no failures.

- [ ] **Step 4: Commit**

```bash
git add Cargo.toml Cargo.lock crates
git commit -m "chore: scaffold anthrex workspace with proto, daemon, tui and cli crates

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 2: Protocol types and messages

**Files:**
- Create: `crates/proto/src/types.rs`, `crates/proto/src/messages.rs`
- Modify: `crates/proto/src/lib.rs`

**Interfaces:**
- Produces: `proto::{Runtime, Status, ExitInfo, WindowSpec, WindowInfo, ClientKind, ClientMsg, DaemonMsg, HookSource}` with the exact fields below. `Runtime::label() -> &'static str`, `Runtime: FromStr`, `Status::label() -> &'static str`.

- [ ] **Step 1: Write the failing tests**

Append to `crates/proto/src/types.rs` (create the file with just the test module first):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_parses_and_labels() {
        assert_eq!("claude".parse::<Runtime>().unwrap(), Runtime::Claude);
        assert_eq!("CODEX".parse::<Runtime>().unwrap(), Runtime::Codex);
        assert_eq!(Runtime::Shell.label(), "shell");
        assert!("perl".parse::<Runtime>().is_err());
    }

    #[test]
    fn status_labels_are_lowercase_words() {
        assert_eq!(Status::Attention.label(), "attention");
        assert_eq!(Status::Exited.label(), "exited");
    }

    #[test]
    fn window_info_round_trips_through_json() {
        let info = WindowInfo {
            id: 7,
            name: "api".into(),
            runtime: Runtime::Claude,
            cwd: "/tmp/repo".into(),
            branch: Some("feat/api".into()),
            status: Status::Working,
            tool: Some("Bash".into()),
            since_secs: 12,
            last_output_secs: 1,
            has_session: true,
            exit: None,
        };
        let json = serde_json::to_string(&info).unwrap();
        assert!(json.contains("\"runtime\":\"claude\""));
        assert_eq!(serde_json::from_str::<WindowInfo>(&json).unwrap(), info);
    }
}
```

Create `crates/proto/src/messages.rs` with just its test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ClientKind, Runtime, WindowSpec};

    #[test]
    fn input_bytes_survive_messagepack() {
        let msg = ClientMsg::Input { window_id: 3, bytes: vec![0x1b, b'[', b'A', 0xff] };
        let packed = rmp_serde::to_vec_named(&msg).unwrap();
        let back: ClientMsg = rmp_serde::from_slice(&packed).unwrap();
        assert_eq!(back, msg);
    }

    #[test]
    fn hook_payload_survives_messagepack() {
        let msg = ClientMsg::HookEvent {
            window_id: 1,
            source: HookSource::Claude,
            payload: serde_json::json!({"hook_event_name": "Stop", "session_id": "abc"}),
        };
        let packed = rmp_serde::to_vec_named(&msg).unwrap();
        let back: ClientMsg = rmp_serde::from_slice(&packed).unwrap();
        assert_eq!(back, msg);
    }

    #[test]
    fn every_daemon_message_round_trips() {
        let spec = WindowSpec {
            name: None,
            runtime: Runtime::Shell,
            cwd: "/tmp".into(),
            worktree_branch: None,
            model: None,
            initial_prompt: None,
        };
        let msgs = vec![
            ClientMsg::Hello { proto_version: 1, client: ClientKind::Tui },
            ClientMsg::CreateWindow { spec, cols: 80, rows: 24 },
            ClientMsg::Subscribe { window_id: 1, cols: 80, rows: 24 },
        ];
        for m in msgs {
            let back: ClientMsg = rmp_serde::from_slice(&rmp_serde::to_vec_named(&m).unwrap()).unwrap();
            assert_eq!(back, m);
        }
        let d = DaemonMsg::Snapshot { window_id: 1, cols: 80, rows: 24, bytes: b"\x1b[H\x1b[Jhi".to_vec() };
        let back: DaemonMsg = rmp_serde::from_slice(&rmp_serde::to_vec_named(&d).unwrap()).unwrap();
        assert_eq!(back, d);
    }
}
```

Add to `crates/proto/src/lib.rs`:

```rust
pub mod messages;
pub mod types;

pub use messages::{ClientMsg, DaemonMsg, HookSource};
pub use types::{ClientKind, ExitInfo, Runtime, Status, WindowInfo, WindowSpec};
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p anthrex-proto`
Expected: compile error, `cannot find type Runtime` (and friends).

- [ ] **Step 3: Implement the types**

Put this above the test module in `crates/proto/src/types.rs`:

```rust
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::str::FromStr;

/// Which program a window runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Runtime {
    Claude,
    Codex,
    Shell,
}

impl Runtime {
    pub fn label(self) -> &'static str {
        match self {
            Runtime::Claude => "claude",
            Runtime::Codex => "codex",
            Runtime::Shell => "shell",
        }
    }
}

impl FromStr for Runtime {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "claude" => Ok(Runtime::Claude),
            "codex" => Ok(Runtime::Codex),
            "shell" => Ok(Runtime::Shell),
            other => Err(format!("unknown runtime '{other}' (expected claude, codex or shell)")),
        }
    }
}

impl std::fmt::Display for Runtime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

/// Sidebar status of a window. See spec section 3.4.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Starting,
    Working,
    Idle,
    Attention,
    Done,
    Exited,
}

impl Status {
    pub fn label(self) -> &'static str {
        match self {
            Status::Starting => "starting",
            Status::Working => "working",
            Status::Idle => "idle",
            Status::Attention => "attention",
            Status::Done => "done",
            Status::Exited => "exited",
        }
    }
}

/// How a window's child ended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExitInfo {
    pub code: Option<i32>,
    pub reason: String,
}

/// What the client asks for when creating a window.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowSpec {
    pub name: Option<String>,
    pub runtime: Runtime,
    pub cwd: PathBuf,
    pub worktree_branch: Option<String>,
    pub model: Option<String>,
    pub initial_prompt: Option<String>,
}

/// The display projection of a window, sent to clients.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    pub has_session: bool,
    pub exit: Option<ExitInfo>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ClientKind {
    Tui,
    Cli,
    Hook,
}
```

Put this above the test module in `crates/proto/src/messages.rs`:

```rust
use crate::types::{ClientKind, WindowInfo, WindowSpec};
use serde::{Deserialize, Serialize};

/// Who produced a hook event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum HookSource {
    Claude,
    CodexNotify,
}

/// Client → daemon.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ClientMsg {
    Hello { proto_version: u32, client: ClientKind },
    ListWindows,
    CreateWindow { spec: WindowSpec, cols: u16, rows: u16 },
    Subscribe { window_id: u32, cols: u16, rows: u16 },
    Unsubscribe,
    Input {
        window_id: u32,
        #[serde(with = "serde_bytes")]
        bytes: Vec<u8>,
    },
    Resize { window_id: u32, cols: u16, rows: u16 },
    Kill { window_id: u32 },
    Remove { window_id: u32, remove_worktree: bool, force: bool },
    Restart { window_id: u32 },
    Rename { window_id: u32, name: String },
    HookEvent { window_id: u32, source: HookSource, payload: serde_json::Value },
    Shutdown,
}

/// Daemon → client.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum DaemonMsg {
    Welcome { daemon_version: String, windows: Vec<WindowInfo> },
    WindowsChanged { windows: Vec<WindowInfo> },
    Created { window_id: u32 },
    Snapshot {
        window_id: u32,
        cols: u16,
        rows: u16,
        #[serde(with = "serde_bytes")]
        bytes: Vec<u8>,
    },
    Output {
        window_id: u32,
        #[serde(with = "serde_bytes")]
        bytes: Vec<u8>,
    },
    Ack { request: String },
    Error { request: String, message: String },
    Bye { reason: String },
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p anthrex-proto`
Expected: `6 passed`.

- [ ] **Step 5: Commit**

```bash
git add crates/proto
git commit -m "feat(proto): add window types and client/daemon messages

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 3: Framing codec

**Files:**
- Create: `crates/proto/src/codec.rs`
- Modify: `crates/proto/src/lib.rs`

**Interfaces:**
- Produces: `proto::codec::{encode, decode, read_frame, write_frame, CodecError, MAX_FRAME}`; `read_frame` returns `Ok(None)` on clean EOF.

- [ ] **Step 1: Write the failing tests**

Create `crates/proto/src/codec.rs` with only:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ClientKind, ClientMsg, DaemonMsg};
    use tokio::io::AsyncWriteExt;

    #[test]
    fn encode_prefixes_big_endian_length() {
        let frame = encode(&ClientMsg::ListWindows).unwrap();
        let len = u32::from_be_bytes([frame[0], frame[1], frame[2], frame[3]]) as usize;
        assert_eq!(len, frame.len() - 4);
        let back: ClientMsg = decode(&frame[4..]).unwrap();
        assert_eq!(back, ClientMsg::ListWindows);
    }

    #[test]
    fn encode_rejects_oversized_body() {
        let msg = DaemonMsg::Output { window_id: 1, bytes: vec![0u8; MAX_FRAME + 1] };
        assert!(matches!(encode(&msg), Err(CodecError::TooLarge(_))));
    }

    #[tokio::test]
    async fn frames_round_trip_over_a_duplex_stream() {
        let (mut a, mut b) = tokio::io::duplex(4096);
        let hello = ClientMsg::Hello { proto_version: 1, client: ClientKind::Cli };
        write_frame(&mut a, &hello).await.unwrap();
        write_frame(&mut a, &ClientMsg::Unsubscribe).await.unwrap();
        let first: Option<ClientMsg> = read_frame(&mut b).await.unwrap();
        let second: Option<ClientMsg> = read_frame(&mut b).await.unwrap();
        assert_eq!(first, Some(hello));
        assert_eq!(second, Some(ClientMsg::Unsubscribe));
    }

    #[tokio::test]
    async fn read_frame_returns_none_on_clean_eof() {
        let (a, mut b) = tokio::io::duplex(64);
        drop(a);
        let got: Option<ClientMsg> = read_frame(&mut b).await.unwrap();
        assert_eq!(got, None);
    }

    #[tokio::test]
    async fn read_frame_rejects_oversized_header() {
        let (mut a, mut b) = tokio::io::duplex(64);
        a.write_all(&((MAX_FRAME as u32) + 1).to_be_bytes()).await.unwrap();
        let got: Result<Option<ClientMsg>, CodecError> = read_frame(&mut b).await;
        assert!(matches!(got, Err(CodecError::TooLarge(_))));
    }
}
```

Add to `crates/proto/src/lib.rs`:

```rust
pub mod codec;
pub use codec::{CodecError, MAX_FRAME, decode, encode, read_frame, write_frame};
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p anthrex-proto codec`
Expected: compile error, `cannot find function encode`.

- [ ] **Step 3: Implement the codec**

Put above the test module in `crates/proto/src/codec.rs`:

```rust
//! Length-prefixed MessagePack framing: 4-byte big-endian length, then the body.

use serde::{Serialize, de::DeserializeOwned};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// Largest body accepted on either side.
pub const MAX_FRAME: usize = 16 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum CodecError {
    #[error("frame too large: {0} bytes (max {MAX_FRAME})")]
    TooLarge(usize),
    #[error("encode: {0}")]
    Encode(#[from] rmp_serde::encode::Error),
    #[error("decode: {0}")]
    Decode(#[from] rmp_serde::decode::Error),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

/// Serializes `msg` and prepends the length header.
pub fn encode<T: Serialize>(msg: &T) -> Result<Vec<u8>, CodecError> {
    let body = rmp_serde::to_vec_named(msg)?;
    if body.len() > MAX_FRAME {
        return Err(CodecError::TooLarge(body.len()));
    }
    let mut frame = Vec::with_capacity(4 + body.len());
    frame.extend_from_slice(&(body.len() as u32).to_be_bytes());
    frame.extend_from_slice(&body);
    Ok(frame)
}

/// Deserializes a body (without the length header).
pub fn decode<T: DeserializeOwned>(body: &[u8]) -> Result<T, CodecError> {
    Ok(rmp_serde::from_slice(body)?)
}

pub async fn write_frame<W, T>(writer: &mut W, msg: &T) -> Result<(), CodecError>
where
    W: AsyncWrite + Unpin,
    T: Serialize,
{
    let frame = encode(msg)?;
    writer.write_all(&frame).await?;
    writer.flush().await?;
    Ok(())
}

/// Reads one frame. Returns `Ok(None)` when the peer closed the stream between frames.
pub async fn read_frame<R, T>(reader: &mut R) -> Result<Option<T>, CodecError>
where
    R: AsyncRead + Unpin,
    T: DeserializeOwned,
{
    let mut header = [0u8; 4];
    match reader.read_exact(&mut header).await {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e.into()),
    }
    let len = u32::from_be_bytes(header) as usize;
    if len > MAX_FRAME {
        return Err(CodecError::TooLarge(len));
    }
    let mut body = vec![0u8; len];
    reader.read_exact(&mut body).await?;
    Ok(Some(decode(&body)?))
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p anthrex-proto`
Expected: `11 passed`.

- [ ] **Step 5: Commit**

```bash
git add crates/proto
git commit -m "feat(proto): add length-prefixed MessagePack framing

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 4: Platform paths

**Files:**
- Create: `crates/proto/src/paths.rs`
- Modify: `crates/proto/src/lib.rs`

**Interfaces:**
- Produces: `proto::paths::{socket_path, socket_dir, data_dir, config_path, log_path, pid_path, state_path}` all returning `PathBuf`. `ANTHREX_SOCKET` and `ANTHREX_DATA_DIR` override the computed values.

- [ ] **Step 1: Write the failing tests**

Create `crates/proto/src/paths.rs` with only:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // Environment variables are process-global; serialize the tests that touch them.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn default_socket_path_is_inside_a_private_dir() {
        let _guard = ENV_LOCK.lock().unwrap();
        // SAFETY: tests in this module are serialized by ENV_LOCK and no other thread reads these variables.
        unsafe { std::env::remove_var("ANTHREX_SOCKET") };
        let p = socket_path();
        assert_eq!(p.file_name().unwrap(), "daemon.sock");
        let dir = p.parent().unwrap().to_string_lossy().into_owned();
        assert!(dir.contains("anthrex"), "{dir}");
        assert!(p.to_string_lossy().len() < 100, "socket path too long for sun_path: {}", p.display());
    }

    #[test]
    fn env_overrides_socket_and_data_dir() {
        let _guard = ENV_LOCK.lock().unwrap();
        // SAFETY: see above.
        unsafe {
            std::env::set_var("ANTHREX_SOCKET", "/tmp/x/custom.sock");
            std::env::set_var("ANTHREX_DATA_DIR", "/tmp/x/data");
        }
        assert_eq!(socket_path(), PathBuf::from("/tmp/x/custom.sock"));
        assert_eq!(log_path(), PathBuf::from("/tmp/x/data/daemon.log"));
        assert_eq!(pid_path(), PathBuf::from("/tmp/x/data/daemon.pid"));
        assert_eq!(state_path(), PathBuf::from("/tmp/x/data/state.json"));
        unsafe {
            std::env::remove_var("ANTHREX_SOCKET");
            std::env::remove_var("ANTHREX_DATA_DIR");
        }
    }

    #[test]
    fn default_data_dir_ends_with_anthrex() {
        let _guard = ENV_LOCK.lock().unwrap();
        unsafe { std::env::remove_var("ANTHREX_DATA_DIR") };
        assert_eq!(data_dir().file_name().unwrap(), "anthrex");
        assert_eq!(config_path().file_name().unwrap(), "config.toml");
    }
}
```

Add to `crates/proto/src/lib.rs`:

```rust
pub mod paths;
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p anthrex-proto paths`
Expected: compile error, `cannot find function socket_path`.

- [ ] **Step 3: Implement the paths**

Put above the test module in `crates/proto/src/paths.rs`:

```rust
//! Where the socket, data and config live. See spec section 2.3.

use std::path::PathBuf;

fn uid() -> u32 {
    // SAFETY: getuid has no preconditions and cannot fail.
    unsafe { libc::getuid() }
}

/// Directory that holds the daemon socket (created with mode 0700 by the daemon).
pub fn socket_dir() -> PathBuf {
    if cfg!(target_os = "macos") {
        let tmp = std::env::var_os("TMPDIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/tmp"));
        return tmp.join(format!("anthrex-{}", uid()));
    }
    match dirs::runtime_dir() {
        Some(rt) => rt.join("anthrex"),
        None => PathBuf::from(format!("/tmp/anthrex-{}", uid())),
    }
}

/// The daemon's Unix socket. `ANTHREX_SOCKET` overrides it.
pub fn socket_path() -> PathBuf {
    if let Some(p) = std::env::var_os("ANTHREX_SOCKET") {
        return PathBuf::from(p);
    }
    socket_dir().join("daemon.sock")
}

/// State, log and pid files. `ANTHREX_DATA_DIR` overrides it.
pub fn data_dir() -> PathBuf {
    if let Some(p) = std::env::var_os("ANTHREX_DATA_DIR") {
        return PathBuf::from(p);
    }
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("anthrex")
}

pub fn config_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("anthrex")
        .join("config.toml")
}

pub fn log_path() -> PathBuf {
    data_dir().join("daemon.log")
}

pub fn pid_path() -> PathBuf {
    data_dir().join("daemon.pid")
}

pub fn state_path() -> PathBuf {
    data_dir().join("state.json")
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p anthrex-proto`
Expected: `14 passed`.

- [ ] **Step 5: Commit**

```bash
git add crates/proto
git commit -m "feat(proto): add socket, data and config path helpers

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 5: Launch plans per runtime

**Files:**
- Create: `crates/daemon/src/launch.rs`
- Modify: `crates/daemon/src/lib.rs`

**Interfaces:**
- Produces: `daemon::launch::{LaunchPlan { program, args, cwd, env }, LaunchContext { window_id, name, socket_path, shell }, plan(&WindowSpec, &LaunchContext) -> LaunchPlan}`. Plan 2 extends `plan` with hook settings; the signature stays.

- [ ] **Step 1: Write the failing tests**

Create `crates/daemon/src/launch.rs` with only:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use proto::Runtime;
    use std::path::Path;

    fn spec(runtime: Runtime) -> WindowSpec {
        WindowSpec {
            name: Some("api".into()),
            runtime,
            cwd: "/tmp/repo".into(),
            worktree_branch: None,
            model: None,
            initial_prompt: None,
        }
    }

    fn ctx() -> LaunchContext<'static> {
        LaunchContext { window_id: 4, name: "api", socket_path: Path::new("/tmp/a.sock"), shell: "/bin/zsh" }
    }

    #[test]
    fn shell_runs_login_shell_in_cwd_with_anthrex_env() {
        let p = plan(&spec(Runtime::Shell), &ctx());
        assert_eq!(p.program, "/bin/zsh");
        assert_eq!(p.args, vec!["-l".to_string()]);
        assert_eq!(p.cwd, Path::new("/tmp/repo"));
        let env: std::collections::HashMap<_, _> = p.env.iter().cloned().collect();
        assert_eq!(env["TERM"], "xterm-256color");
        assert_eq!(env["COLORTERM"], "truecolor");
        assert_eq!(env["ANTHREX_WINDOW_ID"], "4");
        assert_eq!(env["ANTHREX_SOCKET"], "/tmp/a.sock");
    }

    #[test]
    fn claude_gets_name_model_and_prompt() {
        let mut s = spec(Runtime::Claude);
        s.model = Some("opus".into());
        s.initial_prompt = Some("fix the tests".into());
        let p = plan(&s, &ctx());
        assert_eq!(p.program, "claude");
        assert_eq!(p.args, vec!["--name", "api", "--model", "opus", "fix the tests"]);
    }

    #[test]
    fn codex_gets_cwd_flag_model_and_prompt() {
        let mut s = spec(Runtime::Codex);
        s.model = Some("gpt-5-codex".into());
        s.initial_prompt = Some("hello".into());
        let p = plan(&s, &ctx());
        assert_eq!(p.program, "codex");
        assert_eq!(p.args, vec!["-C", "/tmp/repo", "-m", "gpt-5-codex", "hello"]);
    }
}
```

Add to `crates/daemon/src/lib.rs`:

```rust
pub mod launch;
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p anthrex-daemon launch`
Expected: compile error, `cannot find function plan`.

- [ ] **Step 3: Implement the launcher**

Put above the test module in `crates/daemon/src/launch.rs`:

```rust
//! Builds the command line and environment for each runtime. See spec section 3.2.

use proto::{Runtime, WindowSpec};
use std::path::{Path, PathBuf};

/// Everything needed to spawn a window's child.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchPlan {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub env: Vec<(String, String)>,
}

/// Per-window facts the launcher needs that are not in the spec.
pub struct LaunchContext<'a> {
    pub window_id: u32,
    pub name: &'a str,
    pub socket_path: &'a Path,
    /// The user's login shell, e.g. `/bin/zsh`.
    pub shell: &'a str,
}

pub fn plan(spec: &WindowSpec, ctx: &LaunchContext<'_>) -> LaunchPlan {
    let env = vec![
        ("TERM".to_string(), "xterm-256color".to_string()),
        ("COLORTERM".to_string(), "truecolor".to_string()),
        ("ANTHREX_WINDOW_ID".to_string(), ctx.window_id.to_string()),
        ("ANTHREX_SOCKET".to_string(), ctx.socket_path.display().to_string()),
    ];
    let (program, args) = match spec.runtime {
        Runtime::Shell => (ctx.shell.to_string(), vec!["-l".to_string()]),
        Runtime::Claude => {
            let mut args = vec!["--name".to_string(), ctx.name.to_string()];
            if let Some(model) = &spec.model {
                args.push("--model".to_string());
                args.push(model.clone());
            }
            if let Some(prompt) = &spec.initial_prompt {
                args.push(prompt.clone());
            }
            ("claude".to_string(), args)
        }
        Runtime::Codex => {
            let mut args = vec!["-C".to_string(), spec.cwd.display().to_string()];
            if let Some(model) = &spec.model {
                args.push("-m".to_string());
                args.push(model.clone());
            }
            if let Some(prompt) = &spec.initial_prompt {
                args.push(prompt.clone());
            }
            ("codex".to_string(), args)
        }
    };
    LaunchPlan { program, args, cwd: spec.cwd.clone(), env }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p anthrex-daemon launch`
Expected: `3 passed`.

- [ ] **Step 5: Commit**

```bash
git add crates/daemon
git commit -m "feat(daemon): build launch plans for shell, claude and codex windows

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 6: Status state machine

**Files:**
- Create: `crates/daemon/src/status.rs`
- Modify: `crates/daemon/src/lib.rs`

**Interfaces:**
- Produces: `daemon::status::{StatusEvent, next(current: Status, event: StatusEvent, runtime: Runtime) -> Status}`. Plan 2 adds hook and title events to `StatusEvent` and per-runtime rows; the `next` signature stays.

- [ ] **Step 1: Write the failing tests**

Create `crates/daemon/src/status.rs` with only:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use proto::Runtime::{Claude, Shell};
    use proto::Status::*;

    #[test]
    fn output_makes_a_window_working_and_quiet_makes_it_idle() {
        assert_eq!(next(Starting, StatusEvent::Output, Shell), Working);
        assert_eq!(next(Idle, StatusEvent::Output, Shell), Working);
        assert_eq!(next(Done, StatusEvent::Output, Shell), Working);
        assert_eq!(next(Working, StatusEvent::Quiet, Shell), Idle);
        assert_eq!(next(Idle, StatusEvent::Quiet, Shell), Idle);
    }

    #[test]
    fn bell_demands_attention_until_input_is_sent() {
        assert_eq!(next(Working, StatusEvent::Bell, Shell), Attention);
        assert_eq!(next(Attention, StatusEvent::Output, Shell), Attention);
        assert_eq!(next(Attention, StatusEvent::InputSent, Shell), Working);
        assert_eq!(next(Idle, StatusEvent::InputSent, Shell), Idle);
    }

    #[test]
    fn focusing_clears_done() {
        assert_eq!(next(Done, StatusEvent::Focused, Shell), Idle);
        assert_eq!(next(Working, StatusEvent::Focused, Shell), Working);
    }

    #[test]
    fn exited_is_terminal() {
        assert_eq!(next(Working, StatusEvent::Exited, Shell), Exited);
        assert_eq!(next(Exited, StatusEvent::Output, Shell), Exited);
        assert_eq!(next(Exited, StatusEvent::Bell, Shell), Exited);
    }

    #[test]
    fn fallback_rules_apply_to_every_runtime_in_this_milestone() {
        assert_eq!(next(Starting, StatusEvent::Output, Claude), Working);
        assert_eq!(next(Working, StatusEvent::Quiet, Claude), Idle);
    }
}
```

Add to `crates/daemon/src/lib.rs`:

```rust
pub mod status;
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p anthrex-daemon status`
Expected: compile error, `cannot find function next`.

- [ ] **Step 3: Implement the transition function**

Put above the test module in `crates/daemon/src/status.rs`:

```rust
//! Pure status transitions. See spec section 3.4. This milestone implements the
//! "all" rows and the Shell rows; every runtime uses the Shell rows as its fallback.

use proto::{Runtime, Status};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusEvent {
    /// Bytes arrived from the PTY.
    Output,
    /// No output for `manager::QUIET_AFTER`.
    Quiet,
    /// The program rang the terminal bell.
    Bell,
    /// A client started viewing this window.
    Focused,
    /// A client typed into this window.
    InputSent,
    /// The child process ended.
    Exited,
}

pub fn next(current: Status, event: StatusEvent, _runtime: Runtime) -> Status {
    use Status::*;
    use StatusEvent as E;
    match (current, event) {
        (Exited, _) | (_, E::Exited) => Exited,
        (_, E::Bell) => Attention,
        (Done, E::Focused) => Idle,
        (Attention, E::InputSent) => Working,
        (Starting | Idle | Done, E::Output) => Working,
        (Working, E::Quiet) => Idle,
        (s, _) => s,
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p anthrex-daemon status`
Expected: `5 passed`.

- [ ] **Step 5: Commit**

```bash
git add crates/daemon
git commit -m "feat(daemon): add status transition function

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 7: PTY window

**Files:**
- Create: `crates/daemon/src/window.rs`, `crates/daemon/tests/window.rs`
- Modify: `crates/daemon/src/lib.rs`

**Interfaces:**
- Consumes: `daemon::launch::LaunchPlan`.
- Produces: `daemon::window::{Window, WindowEvent, Attachment, OUTPUT_CHANNEL_CAPACITY}`.
  - `Window::spawn(id: u32, plan: &LaunchPlan, cols: u16, rows: u16, events: mpsc::UnboundedSender<(u32, WindowEvent)>) -> anyhow::Result<Window>`
  - `write_input(&self, &[u8]) -> anyhow::Result<()>`, `resize(&self, cols, rows) -> anyhow::Result<()>`, `size(&self) -> (u16, u16)` as (cols, rows), `attach(&self) -> Attachment`, `snapshot(&self) -> Vec<u8>`, `screen_text(&self) -> String`, `signal(&self, sig: i32) -> anyhow::Result<()>`, `pid(&self) -> Option<u32>`.
  - `WindowEvent::{Output, Bell, Title(String), Exited { code: Option<i32>, signal: Option<String> }}`.
  - `Attachment { output: broadcast::Receiver<Bytes>, snapshot: Vec<u8>, cols: u16, rows: u16 }`.

- [ ] **Step 1: Write the failing integration tests**

Create `crates/daemon/tests/window.rs`:

```rust
use daemon::launch::LaunchPlan;
use daemon::window::{Window, WindowEvent};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

fn plan(program: &str, args: &[&str]) -> LaunchPlan {
    LaunchPlan {
        program: program.to_string(),
        args: args.iter().map(|s| s.to_string()).collect(),
        cwd: std::env::temp_dir(),
        env: vec![("TERM".to_string(), "xterm-256color".to_string())],
    }
}

async fn wait_until(what: &str, mut cond: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !cond() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn wait_event(
    rx: &mut mpsc::UnboundedReceiver<(u32, WindowEvent)>,
    pred: impl Fn(&WindowEvent) -> bool,
) -> WindowEvent {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let (_, ev) = rx.recv().await.expect("event channel closed");
            if pred(&ev) {
                return ev;
            }
        }
    })
    .await
    .expect("timed out waiting for event")
}

#[tokio::test]
async fn shows_output_accepts_input_and_reports_exit_code() {
    let (tx, mut rx) = mpsc::unbounded_channel();
    let w = Window::spawn(1, &plan("sh", &["-c", "echo hello-anthrex; cat"]), 80, 24, tx).unwrap();
    wait_until("greeting", || w.screen_text().contains("hello-anthrex")).await;
    w.write_input(b"ping-pong\n").unwrap();
    wait_until("echoed input", || w.screen_text().contains("ping-pong")).await;
    w.write_input(&[0x04]).unwrap(); // Ctrl-D ends `cat`
    let ev = wait_event(&mut rx, |e| matches!(e, WindowEvent::Exited { .. })).await;
    assert_eq!(ev, WindowEvent::Exited { code: Some(0), signal: None });
}

#[tokio::test]
async fn nonzero_exit_code_is_reported() {
    let (tx, mut rx) = mpsc::unbounded_channel();
    let _w = Window::spawn(2, &plan("sh", &["-c", "exit 3"]), 80, 24, tx).unwrap();
    let ev = wait_event(&mut rx, |e| matches!(e, WindowEvent::Exited { .. })).await;
    assert_eq!(ev, WindowEvent::Exited { code: Some(3), signal: None });
}

#[tokio::test]
async fn missing_binary_shows_error_in_window_and_exits_127() {
    let (tx, mut rx) = mpsc::unbounded_channel();
    let w = Window::spawn(3, &plan("definitely-not-a-binary-anthrex", &[]), 80, 24, tx).unwrap();
    let ev = wait_event(&mut rx, |e| matches!(e, WindowEvent::Exited { .. })).await;
    assert_eq!(ev, WindowEvent::Exited { code: Some(127), signal: None });
    assert!(w.screen_text().contains("not found"), "screen: {:?}", w.screen_text());
}

#[tokio::test]
async fn resize_changes_the_size_the_child_sees() {
    let (tx, _rx) = mpsc::unbounded_channel();
    let w = Window::spawn(4, &plan("sh", &["-c", "sleep 0.5; stty size"]), 80, 24, tx).unwrap();
    w.resize(100, 30).unwrap();
    assert_eq!(w.size(), (100, 30));
    wait_until("stty output", || w.screen_text().contains("30 100")).await;
}

#[tokio::test]
async fn bell_and_title_are_reported_as_events() {
    let (tx, mut rx) = mpsc::unbounded_channel();
    let _w = Window::spawn(
        5,
        &plan("sh", &["-c", "printf '\\033]0;hello-title\\007'; printf '\\007'; sleep 0.2"]),
        80,
        24,
        tx,
    )
    .unwrap();
    let title = wait_event(&mut rx, |e| matches!(e, WindowEvent::Title(_))).await;
    assert_eq!(title, WindowEvent::Title("hello-title".into()));
    wait_event(&mut rx, |e| *e == WindowEvent::Bell).await;
}

#[tokio::test]
async fn signal_terminates_the_child() {
    let (tx, mut rx) = mpsc::unbounded_channel();
    let w = Window::spawn(6, &plan("sleep", &["30"]), 80, 24, tx).unwrap();
    w.signal(libc::SIGTERM).unwrap();
    let ev = wait_event(&mut rx, |e| matches!(e, WindowEvent::Exited { .. })).await;
    match ev {
        WindowEvent::Exited { code: None, signal: Some(s) } => assert!(s.contains("TERM"), "{s}"),
        other => panic!("expected signal exit, got {other:?}"),
    }
}

#[tokio::test]
async fn attach_gives_a_snapshot_and_live_output_without_duplicates() {
    let (tx, _rx) = mpsc::unbounded_channel();
    let w = Window::spawn(7, &plan("sh", &["-c", "echo first; sleep 0.3; echo second; sleep 0.3"]), 80, 24, tx).unwrap();
    wait_until("first line", || w.screen_text().contains("first")).await;
    let mut att = w.attach();
    assert_eq!((att.cols, att.rows), (80, 24));
    let mut mirror = vt100::Parser::new(att.rows, att.cols, 0);
    mirror.process(&att.snapshot);
    assert!(mirror.screen().contents().contains("first"));
    let deadline = Instant::now() + Duration::from_secs(5);
    while !mirror.screen().contents().contains("second") {
        assert!(Instant::now() < deadline, "never saw second line");
        if let Ok(Ok(chunk)) = tokio::time::timeout(Duration::from_millis(200), att.output.recv()).await {
            mirror.process(&chunk);
        }
    }
    assert_eq!(mirror.screen().contents().matches("first").count(), 1);
}
```

Add `libc.workspace = true` under `[dev-dependencies]` in `crates/daemon/Cargo.toml` (it is already a normal dependency; integration tests need it declared too only if not already visible — since it is a regular dependency it is available to `tests/`; skip if the build already resolves `libc`).

Add to `crates/daemon/src/lib.rs`:

```rust
pub mod window;
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p anthrex-daemon --test window`
Expected: compile error, `unresolved import daemon::window`.

- [ ] **Step 3: Implement the window**

Create `crates/daemon/src/window.rs`:

```rust
//! One pseudo-terminal, its child process, and a VT screen mirror. See spec section 3.3.

use crate::launch::LaunchPlan;
use bytes::Bytes;
use portable_pty::{CommandBuilder, MasterPty, PtySize, native_pty_system};
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use tokio::sync::{broadcast, mpsc};

/// What a window reports to the manager. The manager receives `(window_id, event)` tuples.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WindowEvent {
    Output,
    Bell,
    Title(String),
    Exited { code: Option<i32>, signal: Option<String> },
}

/// Chunks a slow subscriber may fall behind by before it is sent a fresh snapshot.
pub const OUTPUT_CHANNEL_CAPACITY: usize = 1024;

/// Bell and title arrive through vt100's callback trait, so we count them here.
#[derive(Default)]
struct ScreenCallbacks {
    bells: usize,
    title: Option<String>,
}

impl vt100::Callbacks for ScreenCallbacks {
    fn audible_bell(&mut self, _: &mut vt100::Screen) {
        self.bells += 1;
    }

    fn set_window_title(&mut self, _: &mut vt100::Screen, title: &[u8]) {
        self.title = Some(String::from_utf8_lossy(title).into_owned());
    }
}

type Parser = vt100::Parser<ScreenCallbacks>;

/// A live subscription: the exact screen at subscribe time, then every later chunk.
pub struct Attachment {
    pub output: broadcast::Receiver<Bytes>,
    pub snapshot: Vec<u8>,
    pub cols: u16,
    pub rows: u16,
}

pub struct Window {
    master: Box<dyn MasterPty + Send>,
    writer: Mutex<Box<dyn Write + Send>>,
    pid: Option<u32>,
    parser: Arc<Mutex<Parser>>,
    output_tx: broadcast::Sender<Bytes>,
}

impl Window {
    /// Spawns `plan` inside a new PTY of `cols` x `rows`.
    ///
    /// The child is run through `/bin/sh -c 'exec "$0" "$@"'` so a missing program
    /// prints `command not found` inside the window and exits 127.
    pub fn spawn(
        id: u32,
        plan: &LaunchPlan,
        cols: u16,
        rows: u16,
        events: mpsc::UnboundedSender<(u32, WindowEvent)>,
    ) -> anyhow::Result<Self> {
        let pty = native_pty_system();
        let pair = pty.openpty(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })?;

        let mut cmd = CommandBuilder::new("/bin/sh");
        cmd.arg("-c");
        cmd.arg("exec \"$0\" \"$@\"");
        cmd.arg(&plan.program);
        cmd.args(&plan.args);
        cmd.cwd(&plan.cwd);
        for (key, value) in &plan.env {
            cmd.env(key, value);
        }

        let mut child = pair.slave.spawn_command(cmd)?;
        drop(pair.slave);
        let pid = child.process_id();
        let mut reader = pair.master.try_clone_reader()?;
        let writer = pair.master.take_writer()?;
        let parser = Arc::new(Mutex::new(vt100::Parser::new_with_callbacks(
            rows,
            cols,
            0,
            ScreenCallbacks::default(),
        )));
        let (output_tx, _) = broadcast::channel(OUTPUT_CHANNEL_CAPACITY);

        let reader_parser = Arc::clone(&parser);
        let reader_tx = output_tx.clone();
        let reader_events = events.clone();
        std::thread::Builder::new()
            .name(format!("pty-read-{id}"))
            .spawn(move || {
                let mut buf = [0u8; 8192];
                loop {
                    let n = match reader.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => n,
                    };
                    let chunk = Bytes::copy_from_slice(&buf[..n]);
                    let (bell, title) = {
                        // Hold the lock across process + send so `attach` can never
                        // observe a chunk in the screen without also receiving it, or vice versa.
                        let mut p = reader_parser.lock().unwrap();
                        let bells_before = p.callbacks().bells;
                        let title_before = p.callbacks().title.clone();
                        p.process(&chunk);
                        let _ = reader_tx.send(chunk);
                        let cb = p.callbacks();
                        let title = if cb.title != title_before { cb.title.clone() } else { None };
                        (cb.bells > bells_before, title)
                    };
                    let _ = reader_events.send((id, WindowEvent::Output));
                    if bell {
                        let _ = reader_events.send((id, WindowEvent::Bell));
                    }
                    if let Some(t) = title {
                        let _ = reader_events.send((id, WindowEvent::Title(t)));
                    }
                }
            })?;

        std::thread::Builder::new()
            .name(format!("pty-wait-{id}"))
            .spawn(move || {
                let event = match child.wait() {
                    Ok(status) => WindowEvent::Exited {
                        code: status.signal().is_none().then_some(status.exit_code() as i32),
                        signal: status.signal().map(str::to_owned),
                    },
                    Err(e) => WindowEvent::Exited { code: None, signal: Some(format!("wait failed: {e}")) },
                };
                let _ = events.send((id, event));
            })?;

        Ok(Self { master: pair.master, writer: Mutex::new(writer), pid, parser, output_tx })
    }

    pub fn pid(&self) -> Option<u32> {
        self.pid
    }

    pub fn write_input(&self, bytes: &[u8]) -> anyhow::Result<()> {
        let mut w = self.writer.lock().unwrap();
        w.write_all(bytes)?;
        w.flush()?;
        Ok(())
    }

    pub fn resize(&self, cols: u16, rows: u16) -> anyhow::Result<()> {
        self.master.resize(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })?;
        self.parser.lock().unwrap().screen_mut().set_size(rows, cols);
        Ok(())
    }

    /// Current size as `(cols, rows)`.
    pub fn size(&self) -> (u16, u16) {
        let (rows, cols) = self.parser.lock().unwrap().screen().size();
        (cols, rows)
    }

    /// Subscribes to live output and takes the screen snapshot atomically.
    pub fn attach(&self) -> Attachment {
        let p = self.parser.lock().unwrap();
        let output = self.output_tx.subscribe();
        let (rows, cols) = p.screen().size();
        Attachment { output, snapshot: Self::snapshot_of(p.screen()), cols, rows }
    }

    pub fn snapshot(&self) -> Vec<u8> {
        Self::snapshot_of(self.parser.lock().unwrap().screen())
    }

    fn snapshot_of(screen: &vt100::Screen) -> Vec<u8> {
        let mut out = screen.contents_formatted();
        out.extend_from_slice(&screen.state_formatted());
        out.extend_from_slice(&screen.input_mode_formatted());
        out
    }

    /// Plain text of the visible screen; used by tests and status heuristics.
    pub fn screen_text(&self) -> String {
        self.parser.lock().unwrap().screen().contents()
    }

    /// Sends a POSIX signal to the child.
    pub fn signal(&self, sig: i32) -> anyhow::Result<()> {
        let pid = self.pid.ok_or_else(|| anyhow::anyhow!("child has no pid"))?;
        // SAFETY: kill(2) has no memory-safety preconditions; an invalid pid returns an error.
        let rc = unsafe { libc::kill(pid as libc::pid_t, sig) };
        if rc != 0 {
            anyhow::bail!("kill({pid}, {sig}): {}", std::io::Error::last_os_error());
        }
        Ok(())
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p anthrex-daemon --test window`
Expected: `7 passed`. If `signal_terminates_the_child` reports the signal text differently than `SIGTERM`, print it once and loosen the assertion to `contains("TERM")` (already the case) or to the actual text portable-pty produces.

- [ ] **Step 5: Commit**

```bash
git add crates/daemon
git commit -m "feat(daemon): spawn PTY windows with a VT mirror and output broadcast

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 8: Window manager

**Files:**
- Create: `crates/daemon/src/manager.rs`, `crates/daemon/tests/manager.rs`
- Modify: `crates/daemon/src/lib.rs`

**Interfaces:**
- Consumes: `launch::{plan, LaunchContext}`, `status::{next, StatusEvent}`, `window::{Window, WindowEvent, Attachment}`.
- Produces: `daemon::manager::{WindowManager, QUIET_AFTER, KILL_GRACE}`:
  - `WindowManager::new(socket_path: PathBuf, shell: String) -> (Arc<WindowManager>, mpsc::UnboundedReceiver<(u32, WindowEvent)>)`
  - `watch(&self) -> watch::Receiver<Vec<WindowInfo>>`, `list(&self) -> Vec<WindowInfo>`
  - `create(&self, spec: WindowSpec, cols, rows) -> anyhow::Result<WindowInfo>`
  - `handle_event(&self, id, WindowEvent)`, `tick(&self)`
  - `write_input(&self, id, &[u8]) -> anyhow::Result<()>`, `resize(&self, id, cols, rows) -> anyhow::Result<()>`, `attach(&self, id) -> anyhow::Result<Attachment>`, `snapshot(&self, id) -> anyhow::Result<(Vec<u8>, u16, u16)>`, `focus(&self, id)`
  - `kill(self: &Arc<Self>, id) -> anyhow::Result<()>`, `remove(&self, id) -> anyhow::Result<()>`, `rename(&self, id, name: String) -> anyhow::Result<()>`, `async fn shutdown(&self)`

- [ ] **Step 1: Write the failing integration tests**

Create `crates/daemon/tests/manager.rs`:

```rust
use daemon::manager::WindowManager;
use proto::{Runtime, Status, WindowInfo, WindowSpec};
use std::sync::Arc;
use std::time::{Duration, Instant};

fn spec(name: &str) -> WindowSpec {
    WindowSpec {
        name: Some(name.to_string()),
        runtime: Runtime::Shell,
        cwd: std::env::temp_dir(),
        worktree_branch: None,
        model: None,
        initial_prompt: None,
    }
}

/// A manager whose events are pumped by a background task, like the daemon does.
fn manager() -> Arc<WindowManager> {
    let (m, mut events) = WindowManager::new("/tmp/unused.sock".into(), "/bin/sh".into());
    let pump = m.clone();
    tokio::spawn(async move {
        while let Some((id, ev)) = events.recv().await {
            pump.handle_event(id, ev);
        }
    });
    m
}

async fn wait_until(what: &str, mut cond: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(8);
    while !cond() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

fn find(m: &WindowManager, id: u32) -> WindowInfo {
    m.list().into_iter().find(|w| w.id == id).expect("window listed")
}

#[tokio::test]
async fn create_lists_the_window_and_notifies_watchers() {
    let m = manager();
    let mut rx = m.watch();
    let info = m.create(spec("one"), 80, 24).unwrap();
    assert_eq!(info.id, 1);
    assert_eq!(info.name, "one");
    assert_eq!(info.status, Status::Starting);
    rx.changed().await.unwrap();
    assert_eq!(rx.borrow().len(), 1);
    let second = m.create(spec("two"), 80, 24).unwrap();
    assert_eq!(second.id, 2);
    assert_eq!(m.list().iter().map(|w| w.name.as_str()).collect::<Vec<_>>(), vec!["one", "two"]);
}

#[tokio::test]
async fn names_default_to_runtime_and_id_and_must_be_unique() {
    let m = manager();
    let mut unnamed = spec("x");
    unnamed.name = None;
    let info = m.create(unnamed, 80, 24).unwrap();
    assert_eq!(info.name, "shell-1");
    let err = m.create(spec("shell-1"), 80, 24).unwrap_err();
    assert!(err.to_string().contains("already exists"));
    let mut bad_dir = spec("y");
    bad_dir.cwd = "/definitely/missing/dir".into();
    assert!(m.create(bad_dir, 80, 24).unwrap_err().to_string().contains("does not exist"));
}

#[tokio::test]
async fn input_reaches_the_shell_and_output_drives_status() {
    let m = manager();
    let id = m.create(spec("io"), 80, 24).unwrap().id;
    wait_until("prompt output", || find(&m, id).status == Status::Working).await;
    m.write_input(id, b"echo mgr-$((2+2))\n").unwrap();
    wait_until("command output", || {
        let (snap, _, _) = m.snapshot(id).unwrap();
        String::from_utf8_lossy(&snap).contains("mgr-4")
    })
    .await;
    // Nothing else is running, so after QUIET_AFTER the tick turns it idle.
    tokio::time::sleep(daemon::manager::QUIET_AFTER + Duration::from_millis(200)).await;
    m.tick();
    assert_eq!(find(&m, id).status, Status::Idle);
}

#[tokio::test]
async fn attach_then_resize_reports_new_size() {
    let m = manager();
    let id = m.create(spec("size"), 80, 24).unwrap().id;
    m.resize(id, 120, 40).unwrap();
    let att = m.attach(id).unwrap();
    assert_eq!((att.cols, att.rows), (120, 40));
    assert!(m.attach(99).is_err());
}

#[tokio::test]
async fn kill_terminates_and_remove_forgets() {
    let m = manager();
    let id = m.create(spec("victim"), 80, 24).unwrap().id;
    wait_until("shell started", || find(&m, id).status != Status::Starting).await;
    m.kill(id).unwrap();
    wait_until("exited", || find(&m, id).status == Status::Exited).await;
    let info = find(&m, id);
    assert!(info.exit.is_some());
    m.remove(id).unwrap();
    assert!(m.list().is_empty());
    assert!(m.remove(id).is_err());
}

#[tokio::test]
async fn rename_rejects_duplicates() {
    let m = manager();
    let a = m.create(spec("a"), 80, 24).unwrap().id;
    let _b = m.create(spec("b"), 80, 24).unwrap();
    assert!(m.rename(a, "b".into()).is_err());
    m.rename(a, "c".into()).unwrap();
    assert_eq!(find(&m, a).name, "c");
}

#[tokio::test]
async fn shutdown_ends_every_window() {
    let m = manager();
    for n in ["s1", "s2"] {
        m.create(spec(n), 80, 24).unwrap();
    }
    wait_until("both started", || m.list().iter().all(|w| w.status != Status::Starting)).await;
    m.shutdown().await;
    wait_until("both exited", || m.list().iter().all(|w| w.status == Status::Exited)).await;
}
```

Add to `crates/daemon/src/lib.rs`:

```rust
pub mod manager;
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p anthrex-daemon --test manager`
Expected: compile error, `unresolved import daemon::manager`.

- [ ] **Step 3: Implement the manager**

Create `crates/daemon/src/manager.rs`:

```rust
//! Owns every window, applies status events, and broadcasts the window list.

use crate::launch::{self, LaunchContext};
use crate::status::{self, StatusEvent};
use crate::window::{Attachment, Window, WindowEvent};
use proto::{ExitInfo, Status, WindowInfo, WindowSpec};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, watch};

/// A Working window with no output for this long becomes Idle.
pub const QUIET_AFTER: Duration = Duration::from_secs(3);
/// Time between SIGTERM and SIGKILL.
pub const KILL_GRACE: Duration = Duration::from_secs(3);

struct Entry {
    id: u32,
    name: String,
    spec: WindowSpec,
    status: Status,
    tool: Option<String>,
    since: Instant,
    last_output: Instant,
    session_id: Option<String>,
    exit: Option<ExitInfo>,
    window: Window,
}

impl Entry {
    fn info(&self) -> WindowInfo {
        WindowInfo {
            id: self.id,
            name: self.name.clone(),
            runtime: self.spec.runtime,
            cwd: self.spec.cwd.clone(),
            branch: self.spec.worktree_branch.clone(),
            status: self.status,
            tool: self.tool.clone(),
            since_secs: self.since.elapsed().as_secs(),
            last_output_secs: self.last_output.elapsed().as_secs(),
            has_session: self.session_id.is_some(),
            exit: self.exit.clone(),
        }
    }

    /// Applies a status event; returns whether the status changed.
    fn apply(&mut self, event: StatusEvent) -> bool {
        let next = status::next(self.status, event, self.spec.runtime);
        if next == self.status {
            return false;
        }
        self.status = next;
        self.since = Instant::now();
        true
    }
}

struct Inner {
    next_id: u32,
    entries: BTreeMap<u32, Entry>,
}

pub struct WindowManager {
    inner: Mutex<Inner>,
    changed: watch::Sender<Vec<WindowInfo>>,
    events: mpsc::UnboundedSender<(u32, WindowEvent)>,
    socket_path: PathBuf,
    shell: String,
}

impl WindowManager {
    /// Returns the manager and the event receiver the caller must pump into `handle_event`.
    pub fn new(socket_path: PathBuf, shell: String) -> (Arc<Self>, mpsc::UnboundedReceiver<(u32, WindowEvent)>) {
        let (events, events_rx) = mpsc::unbounded_channel();
        let (changed, _) = watch::channel(Vec::new());
        let manager = Arc::new(Self {
            inner: Mutex::new(Inner { next_id: 1, entries: BTreeMap::new() }),
            changed,
            events,
            socket_path,
            shell,
        });
        (manager, events_rx)
    }

    pub fn watch(&self) -> watch::Receiver<Vec<WindowInfo>> {
        self.changed.subscribe()
    }

    pub fn list(&self) -> Vec<WindowInfo> {
        self.inner.lock().unwrap().entries.values().map(Entry::info).collect()
    }

    fn publish(&self, inner: &Inner) {
        self.changed.send_replace(inner.entries.values().map(Entry::info).collect());
    }

    pub fn create(&self, spec: WindowSpec, cols: u16, rows: u16) -> anyhow::Result<WindowInfo> {
        let mut inner = self.inner.lock().unwrap();
        let id = inner.next_id;
        let name = match spec.name.as_deref().map(str::trim).filter(|n| !n.is_empty()) {
            Some(n) => n.to_string(),
            None => format!("{}-{id}", spec.runtime.label()),
        };
        if inner.entries.values().any(|e| e.name == name) {
            anyhow::bail!("a window named '{name}' already exists");
        }
        if !spec.cwd.is_dir() {
            anyhow::bail!("directory does not exist: {}", spec.cwd.display());
        }
        let plan = launch::plan(
            &spec,
            &LaunchContext { window_id: id, name: &name, socket_path: &self.socket_path, shell: &self.shell },
        );
        let window = Window::spawn(id, &plan, cols.max(1), rows.max(1), self.events.clone())?;
        inner.next_id += 1;
        let now = Instant::now();
        let entry = Entry {
            id,
            name,
            spec,
            status: Status::Starting,
            tool: None,
            since: now,
            last_output: now,
            session_id: None,
            exit: None,
            window,
        };
        let info = entry.info();
        inner.entries.insert(id, entry);
        tracing::info!(id, name = %info.name, runtime = %info.runtime, "window created");
        self.publish(&inner);
        Ok(info)
    }

    pub fn handle_event(&self, id: u32, event: WindowEvent) {
        let mut inner = self.inner.lock().unwrap();
        let Some(entry) = inner.entries.get_mut(&id) else { return };
        let changed = match event {
            WindowEvent::Output => {
                entry.last_output = Instant::now();
                entry.apply(StatusEvent::Output)
            }
            WindowEvent::Bell => entry.apply(StatusEvent::Bell),
            WindowEvent::Title(title) => {
                tracing::debug!(id, %title, "window title");
                false
            }
            WindowEvent::Exited { code, signal } => {
                let reason = match (&signal, code) {
                    (Some(sig), _) => format!("killed by {sig}"),
                    (None, Some(c)) => format!("exited with code {c}"),
                    (None, None) => "exited".to_string(),
                };
                tracing::info!(id, %reason, "window exited");
                entry.exit = Some(ExitInfo { code, reason });
                entry.apply(StatusEvent::Exited)
            }
        };
        if changed {
            self.publish(&inner);
        }
    }

    /// Called once a second by the daemon: Working windows that went quiet become Idle.
    pub fn tick(&self) {
        let mut inner = self.inner.lock().unwrap();
        let mut changed = false;
        for entry in inner.entries.values_mut() {
            if entry.status == Status::Working && entry.last_output.elapsed() >= QUIET_AFTER {
                changed |= entry.apply(StatusEvent::Quiet);
            }
        }
        if changed {
            self.publish(&inner);
        }
    }

    fn with_entry<R>(&self, id: u32, f: impl FnOnce(&mut Entry) -> R) -> anyhow::Result<R> {
        let mut inner = self.inner.lock().unwrap();
        let entry = inner.entries.get_mut(&id).ok_or_else(|| anyhow::anyhow!("no window with id {id}"))?;
        Ok(f(entry))
    }

    pub fn write_input(&self, id: u32, bytes: &[u8]) -> anyhow::Result<()> {
        let mut inner = self.inner.lock().unwrap();
        let entry = inner.entries.get_mut(&id).ok_or_else(|| anyhow::anyhow!("no window with id {id}"))?;
        entry.window.write_input(bytes)?;
        if entry.apply(StatusEvent::InputSent) {
            self.publish(&inner);
        }
        Ok(())
    }

    pub fn resize(&self, id: u32, cols: u16, rows: u16) -> anyhow::Result<()> {
        self.with_entry(id, |e| e.window.resize(cols.max(1), rows.max(1)))?
    }

    pub fn attach(&self, id: u32) -> anyhow::Result<Attachment> {
        self.with_entry(id, |e| e.window.attach())
    }

    pub fn snapshot(&self, id: u32) -> anyhow::Result<(Vec<u8>, u16, u16)> {
        self.with_entry(id, |e| {
            let (cols, rows) = e.window.size();
            (e.window.snapshot(), cols, rows)
        })
    }

    /// A client started viewing this window.
    pub fn focus(&self, id: u32) {
        let mut inner = self.inner.lock().unwrap();
        if let Some(entry) = inner.entries.get_mut(&id) {
            if entry.apply(StatusEvent::Focused) {
                self.publish(&inner);
            }
        }
    }

    fn signal(&self, id: u32, sig: i32) -> anyhow::Result<()> {
        let inner = self.inner.lock().unwrap();
        let entry = inner.entries.get(&id).ok_or_else(|| anyhow::anyhow!("no window with id {id}"))?;
        if entry.status == Status::Exited {
            return Ok(());
        }
        entry.window.signal(sig)
    }

    /// SIGTERM now, SIGKILL after `KILL_GRACE` if the child is still alive.
    pub fn kill(self: &Arc<Self>, id: u32) -> anyhow::Result<()> {
        self.signal(id, libc::SIGTERM)?;
        let me = Arc::clone(self);
        std::thread::spawn(move || {
            std::thread::sleep(KILL_GRACE);
            let _ = me.signal(id, libc::SIGKILL);
        });
        Ok(())
    }

    /// Kills immediately and forgets the window.
    pub fn remove(&self, id: u32) -> anyhow::Result<()> {
        let mut inner = self.inner.lock().unwrap();
        let entry = inner.entries.remove(&id).ok_or_else(|| anyhow::anyhow!("no window with id {id}"))?;
        if entry.status != Status::Exited {
            let _ = entry.window.signal(libc::SIGKILL);
        }
        drop(entry);
        tracing::info!(id, "window removed");
        self.publish(&inner);
        Ok(())
    }

    pub fn rename(&self, id: u32, name: String) -> anyhow::Result<()> {
        let name = name.trim().to_string();
        if name.is_empty() {
            anyhow::bail!("name must not be empty");
        }
        let mut inner = self.inner.lock().unwrap();
        if inner.entries.values().any(|e| e.id != id && e.name == name) {
            anyhow::bail!("a window named '{name}' already exists");
        }
        let entry = inner.entries.get_mut(&id).ok_or_else(|| anyhow::anyhow!("no window with id {id}"))?;
        entry.name = name;
        self.publish(&inner);
        Ok(())
    }

    /// SIGTERM every live window, wait up to `KILL_GRACE`, then SIGKILL the rest.
    pub async fn shutdown(&self) {
        let live: Vec<u32> = self.list().into_iter().filter(|w| w.status != Status::Exited).map(|w| w.id).collect();
        for id in &live {
            let _ = self.signal(*id, libc::SIGTERM);
        }
        let deadline = Instant::now() + KILL_GRACE;
        while Instant::now() < deadline {
            if self.list().iter().all(|w| w.status == Status::Exited) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        for id in &live {
            let _ = self.signal(*id, libc::SIGKILL);
        }
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p anthrex-daemon --test manager`
Expected: `7 passed`. `kill_terminates_and_remove_forgets` takes about 3 s because an interactive shell ignores SIGTERM and waits for the SIGKILL escalation; that is the intended behaviour.

- [ ] **Step 5: Commit**

```bash
git add crates/daemon
git commit -m "feat(daemon): add window manager with status tracking and kill escalation

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 9: Socket server

**Files:**
- Create: `crates/daemon/src/server.rs`, `crates/daemon/tests/server.rs`
- Modify: `crates/daemon/src/lib.rs`

**Interfaces:**
- Consumes: `manager::WindowManager`, `proto::{ClientMsg, DaemonMsg, PROTO_VERSION, read_frame, write_frame}`.
- Produces: `daemon::server::serve(listener: UnixListener, manager: Arc<WindowManager>, shutdown: CancellationToken) -> anyhow::Result<()>`; returns when `shutdown` is cancelled (by a signal or a client's `Shutdown`).

- [ ] **Step 1: Write the failing integration tests**

Create `crates/daemon/tests/server.rs`:

```rust
use daemon::manager::WindowManager;
use daemon::server::serve;
use proto::{ClientKind, ClientMsg, DaemonMsg, PROTO_VERSION, Runtime, Status, WindowSpec, read_frame, write_frame};
use std::path::PathBuf;
use std::time::Duration;
use tokio::net::UnixStream;
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};
use tokio_util::sync::CancellationToken;

struct TestDaemon {
    _dir: tempfile::TempDir,
    socket: PathBuf,
    shutdown: CancellationToken,
}

async fn start_daemon() -> TestDaemon {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("d.sock");
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    let (manager, mut events) = WindowManager::new(socket.clone(), "/bin/sh".into());
    let pump = manager.clone();
    tokio::spawn(async move {
        while let Some((id, ev)) = events.recv().await {
            pump.handle_event(id, ev);
        }
    });
    let shutdown = CancellationToken::new();
    tokio::spawn(serve(listener, manager, shutdown.clone()));
    TestDaemon { _dir: dir, socket, shutdown }
}

struct Client {
    rd: OwnedReadHalf,
    wr: OwnedWriteHalf,
}

impl Client {
    async fn connect(d: &TestDaemon, version: u32) -> (Self, DaemonMsg) {
        let stream = UnixStream::connect(&d.socket).await.unwrap();
        let (rd, wr) = stream.into_split();
        let mut c = Client { rd, wr };
        c.send(ClientMsg::Hello { proto_version: version, client: ClientKind::Cli }).await;
        let first = c.recv().await;
        (c, first)
    }

    async fn send(&mut self, m: ClientMsg) {
        write_frame(&mut self.wr, &m).await.unwrap();
    }

    async fn recv(&mut self) -> DaemonMsg {
        tokio::time::timeout(Duration::from_secs(5), read_frame(&mut self.rd))
            .await
            .expect("timed out")
            .unwrap()
            .expect("daemon closed")
    }

    async fn recv_until(&mut self, pred: impl Fn(&DaemonMsg) -> bool) -> DaemonMsg {
        tokio::time::timeout(Duration::from_secs(8), async {
            loop {
                let m = self.recv().await;
                if pred(&m) {
                    return m;
                }
            }
        })
        .await
        .expect("timed out waiting for message")
    }
}

fn shell_spec(name: &str) -> WindowSpec {
    WindowSpec {
        name: Some(name.into()),
        runtime: Runtime::Shell,
        cwd: std::env::temp_dir(),
        worktree_branch: None,
        model: None,
        initial_prompt: None,
    }
}

#[tokio::test]
async fn handshake_returns_welcome_with_empty_window_list() {
    let d = start_daemon().await;
    let (_c, welcome) = Client::connect(&d, PROTO_VERSION).await;
    match welcome {
        DaemonMsg::Welcome { windows, .. } => assert!(windows.is_empty()),
        other => panic!("expected Welcome, got {other:?}"),
    }
}

#[tokio::test]
async fn version_mismatch_is_rejected() {
    let d = start_daemon().await;
    let (_c, reply) = Client::connect(&d, PROTO_VERSION + 1).await;
    match reply {
        DaemonMsg::Error { request, message } => {
            assert_eq!(request, "hello");
            assert!(message.contains("anthrex daemon stop"));
        }
        other => panic!("expected Error, got {other:?}"),
    }
}

#[tokio::test]
async fn create_subscribe_input_and_kill_flow() {
    let d = start_daemon().await;
    let (mut c, _) = Client::connect(&d, PROTO_VERSION).await;

    c.send(ClientMsg::CreateWindow { spec: shell_spec("w"), cols: 80, rows: 24 }).await;
    let created = c.recv_until(|m| matches!(m, DaemonMsg::Created { .. })).await;
    let DaemonMsg::Created { window_id } = created else { unreachable!() };

    c.send(ClientMsg::Subscribe { window_id, cols: 100, rows: 30 }).await;
    let snap = c.recv_until(|m| matches!(m, DaemonMsg::Snapshot { .. })).await;
    let DaemonMsg::Snapshot { cols, rows, .. } = snap else { unreachable!() };
    assert_eq!((cols, rows), (100, 30));

    c.send(ClientMsg::Input { window_id, bytes: b"echo srv-$((1+1))-ok\n".to_vec() }).await;
    let mut seen = Vec::new();
    c.recv_until(|m| {
        if let DaemonMsg::Output { bytes, .. } = m {
            seen.extend_from_slice(bytes);
        }
        String::from_utf8_lossy(&seen).contains("srv-2-ok")
    })
    .await;

    c.send(ClientMsg::Kill { window_id }).await;
    let ack = c.recv_until(|m| matches!(m, DaemonMsg::Ack { .. })).await;
    assert_eq!(ack, DaemonMsg::Ack { request: "kill".into() });
    c.recv_until(|m| match m {
        DaemonMsg::WindowsChanged { windows } => windows.iter().any(|w| w.id == window_id && w.status == Status::Exited),
        _ => false,
    })
    .await;

    c.send(ClientMsg::Remove { window_id, remove_worktree: false, force: false }).await;
    c.recv_until(|m| matches!(m, DaemonMsg::Ack { request } if request == "remove")).await;
}

#[tokio::test]
async fn errors_are_reported_per_request() {
    let d = start_daemon().await;
    let (mut c, _) = Client::connect(&d, PROTO_VERSION).await;
    c.send(ClientMsg::Subscribe { window_id: 42, cols: 80, rows: 24 }).await;
    let reply = c.recv_until(|m| matches!(m, DaemonMsg::Error { .. })).await;
    let DaemonMsg::Error { request, message } = reply else { unreachable!() };
    assert_eq!(request, "subscribe");
    assert!(message.contains("42"));
    c.send(ClientMsg::Restart { window_id: 1 }).await;
    let DaemonMsg::Error { request, .. } = c.recv_until(|m| matches!(m, DaemonMsg::Error { .. })).await else { unreachable!() };
    assert_eq!(request, "restart");
}

#[tokio::test]
async fn shutdown_request_says_bye_and_stops_serving() {
    let d = start_daemon().await;
    let (mut c, _) = Client::connect(&d, PROTO_VERSION).await;
    c.send(ClientMsg::Shutdown).await;
    let bye = c.recv_until(|m| matches!(m, DaemonMsg::Bye { .. })).await;
    assert!(matches!(bye, DaemonMsg::Bye { .. }));
    tokio::time::timeout(Duration::from_secs(2), d.shutdown.cancelled()).await.expect("token cancelled");
}
```

Add to `crates/daemon/src/lib.rs`:

```rust
pub mod server;
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p anthrex-daemon --test server`
Expected: compile error, `unresolved import daemon::server`.

- [ ] **Step 3: Implement the server**

Create `crates/daemon/src/server.rs`:

```rust
//! Accepts client connections and speaks the protocol from spec section 4.

use crate::manager::WindowManager;
use bytes::Bytes;
use proto::{ClientMsg, DaemonMsg, PROTO_VERSION, read_frame, write_frame};
use std::sync::Arc;
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{broadcast, mpsc};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

/// Runs until `shutdown` is cancelled. Each connection gets its own task.
pub async fn serve(listener: UnixListener, manager: Arc<WindowManager>, shutdown: CancellationToken) -> anyhow::Result<()> {
    loop {
        tokio::select! {
            _ = shutdown.cancelled() => return Ok(()),
            accepted = listener.accept() => {
                let (stream, _) = accepted?;
                let manager = manager.clone();
                let shutdown = shutdown.clone();
                tokio::spawn(async move {
                    if let Err(e) = handle_client(stream, manager, shutdown).await {
                        tracing::warn!(error = %e, "client connection ended with error");
                    }
                });
            }
        }
    }
}

fn error(request: &str, message: impl Into<String>) -> DaemonMsg {
    DaemonMsg::Error { request: request.to_string(), message: message.into() }
}

fn ack_or_error(request: &str, result: anyhow::Result<()>) -> DaemonMsg {
    match result {
        Ok(()) => DaemonMsg::Ack { request: request.to_string() },
        Err(e) => error(request, e.to_string()),
    }
}

async fn handle_client(stream: UnixStream, manager: Arc<WindowManager>, shutdown: CancellationToken) -> anyhow::Result<()> {
    let (mut rd, mut wr) = stream.into_split();

    let Some(ClientMsg::Hello { proto_version, client }) = read_frame::<_, ClientMsg>(&mut rd).await? else {
        return Ok(()); // EOF or a client that skipped the handshake: drop silently.
    };
    if proto_version != PROTO_VERSION {
        let message = format!(
            "protocol version mismatch (client {proto_version}, daemon {PROTO_VERSION}); run `anthrex daemon stop` and try again"
        );
        write_frame(&mut wr, &error("hello", message)).await?;
        return Ok(());
    }
    tracing::debug!(?client, "client connected");
    write_frame(
        &mut wr,
        &DaemonMsg::Welcome { daemon_version: env!("CARGO_PKG_VERSION").to_string(), windows: manager.list() },
    )
    .await?;

    // All outgoing traffic goes through one channel so the writer is never shared.
    let (out_tx, mut out_rx) = mpsc::channel::<DaemonMsg>(256);
    let writer: JoinHandle<()> = tokio::spawn(async move {
        while let Some(msg) = out_rx.recv().await {
            if write_frame(&mut wr, &msg).await.is_err() {
                break;
            }
        }
    });

    let mut changes = manager.watch();
    let changes_out = out_tx.clone();
    let changes_task = tokio::spawn(async move {
        while changes.changed().await.is_ok() {
            let windows = changes.borrow_and_update().clone();
            if changes_out.send(DaemonMsg::WindowsChanged { windows }).await.is_err() {
                break;
            }
        }
    });

    let mut subscription: Option<JoinHandle<()>> = None;
    loop {
        let msg = tokio::select! {
            _ = shutdown.cancelled() => {
                let _ = out_tx.send(DaemonMsg::Bye { reason: "daemon shutting down".into() }).await;
                break;
            }
            frame = read_frame::<_, ClientMsg>(&mut rd) => match frame? {
                Some(m) => m,
                None => break,
            },
        };

        let reply = match msg {
            ClientMsg::Hello { .. } => Some(error("hello", "already greeted")),
            ClientMsg::ListWindows => Some(DaemonMsg::WindowsChanged { windows: manager.list() }),
            ClientMsg::CreateWindow { spec, cols, rows } => Some(match manager.create(spec, cols, rows) {
                Ok(info) => DaemonMsg::Created { window_id: info.id },
                Err(e) => error("create", e.to_string()),
            }),
            ClientMsg::Subscribe { window_id, cols, rows } => {
                if let Some(task) = subscription.take() {
                    task.abort();
                }
                match manager.resize(window_id, cols, rows).and_then(|_| manager.attach(window_id)) {
                    Ok(att) => {
                        manager.focus(window_id);
                        // Snapshot must be queued before the forwarder can queue live output.
                        let snapshot = DaemonMsg::Snapshot { window_id, cols: att.cols, rows: att.rows, bytes: att.snapshot };
                        if out_tx.send(snapshot).await.is_err() {
                            break;
                        }
                        subscription = Some(tokio::spawn(forward_output(window_id, att.output, out_tx.clone(), manager.clone())));
                        None
                    }
                    Err(e) => Some(error("subscribe", e.to_string())),
                }
            }
            ClientMsg::Unsubscribe => {
                if let Some(task) = subscription.take() {
                    task.abort();
                }
                Some(DaemonMsg::Ack { request: "unsubscribe".into() })
            }
            ClientMsg::Input { window_id, bytes } => manager.write_input(window_id, &bytes).err().map(|e| error("input", e.to_string())),
            ClientMsg::Resize { window_id, cols, rows } => manager.resize(window_id, cols, rows).err().map(|e| error("resize", e.to_string())),
            ClientMsg::Kill { window_id } => Some(ack_or_error("kill", manager.kill(window_id))),
            ClientMsg::Remove { window_id, .. } => Some(ack_or_error("remove", manager.remove(window_id))),
            ClientMsg::Rename { window_id, name } => Some(ack_or_error("rename", manager.rename(window_id, name))),
            ClientMsg::Restart { .. } => Some(error("restart", "restart is not supported by this daemon version")),
            ClientMsg::HookEvent { .. } => Some(error("hook", "hook events are not supported by this daemon version")),
            ClientMsg::Shutdown => {
                tracing::info!("shutdown requested by client");
                shutdown.cancel();
                None
            }
        };
        if let Some(reply) = reply {
            if out_tx.send(reply).await.is_err() {
                break;
            }
        }
    }

    if let Some(task) = subscription.take() {
        task.abort();
    }
    changes_task.abort();
    drop(out_tx);
    let _ = writer.await;
    tracing::debug!("client disconnected");
    Ok(())
}

/// Copies live PTY output to the client; a lagging client gets a fresh snapshot instead of the gap.
async fn forward_output(
    window_id: u32,
    mut output: broadcast::Receiver<Bytes>,
    out: mpsc::Sender<DaemonMsg>,
    manager: Arc<WindowManager>,
) {
    loop {
        match output.recv().await {
            Ok(chunk) => {
                if out.send(DaemonMsg::Output { window_id, bytes: chunk.to_vec() }).await.is_err() {
                    break;
                }
            }
            Err(broadcast::error::RecvError::Lagged(n)) => {
                tracing::debug!(window_id, missed = n, "client lagged; resending snapshot");
                match manager.snapshot(window_id) {
                    Ok((bytes, cols, rows)) => {
                        if out.send(DaemonMsg::Snapshot { window_id, cols, rows, bytes }).await.is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
            Err(broadcast::error::RecvError::Closed) => break,
        }
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p anthrex-daemon --test server`
Expected: `5 passed`.

- [ ] **Step 5: Commit**

```bash
git add crates/daemon
git commit -m "feat(daemon): serve the window protocol over a Unix socket

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 10: Daemon lifecycle and `anthrex daemon` commands

**Files:**
- Create: `crates/daemon/src/lifecycle.rs`, `crates/cli/src/spawn.rs`
- Modify: `crates/daemon/src/lib.rs`, `crates/cli/src/main.rs`

**Interfaces:**
- Produces: `daemon::{DaemonOptions { socket_path, data_dir }, run(DaemonOptions) -> anyhow::Result<()>}`, `daemon::lifecycle::prepare_socket(&Path) -> anyhow::Result<()>`; CLI `anthrex daemon start [--foreground]`, `anthrex daemon stop`, `anthrex daemon status`; `cli::spawn::ensure_daemon(socket: &Path) -> anyhow::Result<()>`.

- [ ] **Step 1: Write the failing tests for socket preparation**

Create `crates/daemon/src/lifecycle.rs` with only:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_socket_file_is_removed() {
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("sub").join("d.sock");
        std::fs::create_dir_all(sock.parent().unwrap()).unwrap();
        std::fs::write(&sock, b"not a socket").unwrap();
        prepare_socket(&sock).unwrap();
        assert!(!sock.exists());
        let mode = std::fs::metadata(sock.parent().unwrap()).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700);
    }

    #[test]
    fn live_socket_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("d.sock");
        let _listener = std::os::unix::net::UnixListener::bind(&sock).unwrap();
        let err = prepare_socket(&sock).unwrap_err();
        assert!(err.to_string().contains("already"));
        assert!(sock.exists());
    }
}
```

Add to `crates/daemon/src/lib.rs`:

```rust
pub mod lifecycle;
pub use lifecycle::{DaemonOptions, run};
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p anthrex-daemon lifecycle`
Expected: compile error, `cannot find function prepare_socket`.

- [ ] **Step 3: Implement the lifecycle**

Put above the test module in `crates/daemon/src/lifecycle.rs`:

```rust
//! Socket setup, logging, pid file, signals, and the top-level daemon loop. Spec section 3.7.

use crate::manager::WindowManager;
use crate::server;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::net::UnixListener;
use tokio_util::sync::CancellationToken;

pub struct DaemonOptions {
    pub socket_path: PathBuf,
    pub data_dir: PathBuf,
}

/// Creates the socket directory (mode 0700) and removes a stale socket file.
/// Fails if a live daemon answers on the socket.
pub fn prepare_socket(path: &Path) -> anyhow::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    if path.exists() {
        match std::os::unix::net::UnixStream::connect(path) {
            Ok(_) => anyhow::bail!("another daemon is already listening on {}", path.display()),
            Err(_) => std::fs::remove_file(path)?,
        }
    }
    Ok(())
}

fn init_logging(data_dir: &Path) -> tracing_appender::non_blocking::WorkerGuard {
    let file = tracing_appender::rolling::never(data_dir, "daemon.log");
    let (writer, guard) = tracing_appender::non_blocking(file);
    let filter = tracing_subscriber::EnvFilter::try_from_env("ANTHREX_LOG")
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).with_ansi(false).with_writer(writer).init();
    guard
}

/// Runs the daemon in the current process until a signal or a client asks it to stop.
pub async fn run(opts: DaemonOptions) -> anyhow::Result<()> {
    std::fs::create_dir_all(&opts.data_dir)?;
    let _log_guard = init_logging(&opts.data_dir);
    prepare_socket(&opts.socket_path)?;
    let listener = UnixListener::bind(&opts.socket_path)?;
    let pid_path = opts.data_dir.join("daemon.pid");
    std::fs::write(&pid_path, std::process::id().to_string())?;
    tracing::info!(socket = %opts.socket_path.display(), pid = std::process::id(), "daemon started");

    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
    let (manager, mut events) = WindowManager::new(opts.socket_path.clone(), shell);
    let shutdown = CancellationToken::new();

    let pump = manager.clone();
    tokio::spawn(async move {
        while let Some((id, ev)) = events.recv().await {
            pump.handle_event(id, ev);
        }
    });

    let ticker = manager.clone();
    let tick_token = shutdown.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(1));
        loop {
            tokio::select! {
                _ = tick_token.cancelled() => break,
                _ = interval.tick() => ticker.tick(),
            }
        }
    });

    let signal_token = shutdown.clone();
    tokio::spawn(async move {
        let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("install SIGTERM handler");
        tokio::select! {
            _ = term.recv() => {}
            _ = tokio::signal::ctrl_c() => {}
        }
        tracing::info!("signal received, shutting down");
        signal_token.cancel();
    });

    server::serve(listener, manager.clone(), shutdown.clone()).await?;
    tracing::info!("stopping agents");
    manager.shutdown().await;
    let _ = std::fs::remove_file(&opts.socket_path);
    let _ = std::fs::remove_file(&pid_path);
    tracing::info!("daemon stopped");
    Ok(())
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p anthrex-daemon lifecycle`
Expected: `2 passed`.

- [ ] **Step 5: Write the detached spawner**

Create `crates/cli/src/spawn.rs`:

```rust
//! Starts the daemon in the background when none is running.

use std::path::Path;
use std::process::Stdio;
use std::time::{Duration, Instant};
use tokio::net::UnixStream;

async fn is_up(socket: &Path) -> bool {
    UnixStream::connect(socket).await.is_ok()
}

/// Re-executes this binary as `anthrex daemon start --foreground` in its own session,
/// with stdio detached, so it outlives the calling process.
fn spawn_detached() -> anyhow::Result<()> {
    use std::os::unix::process::CommandExt;
    let exe = std::env::current_exe()?;
    let mut cmd = std::process::Command::new(exe);
    cmd.args(["daemon", "start", "--foreground"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // SAFETY: setsid is async-signal-safe and touches no Rust state between fork and exec.
    unsafe {
        cmd.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    cmd.spawn()?;
    Ok(())
}

/// Connects if a daemon is running; otherwise starts one and waits up to 3 s for its socket.
pub async fn ensure_daemon(socket: &Path) -> anyhow::Result<()> {
    if is_up(socket).await {
        return Ok(());
    }
    spawn_detached()?;
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        if is_up(socket).await {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    anyhow::bail!("the daemon did not start within 3 s; check {}", proto::paths::log_path().display())
}
```

- [ ] **Step 6: Wire the daemon subcommands into the CLI**

Replace `crates/cli/src/main.rs` with:

```rust
mod spawn;

use clap::{Parser, Subcommand};
use proto::{ClientKind, ClientMsg, DaemonMsg, PROTO_VERSION, read_frame, write_frame};
use std::path::PathBuf;
use tokio::net::UnixStream;

#[derive(Parser)]
#[command(name = "anthrex", version, about = "A terminal multiplexer for coding agents")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Manage the background daemon that owns the agents
    Daemon {
        #[command(subcommand)]
        action: DaemonAction,
    },
}

#[derive(Subcommand)]
enum DaemonAction {
    /// Start the daemon (detached unless --foreground)
    Start {
        #[arg(long)]
        foreground: bool,
    },
    /// Stop the daemon and every agent it owns
    Stop,
    /// Show whether a daemon is running
    Status,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let socket: PathBuf = proto::paths::socket_path();
    match cli.command {
        None => {
            println!("anthrex {} — attach comes in a later task", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Some(Command::Daemon { action }) => daemon_command(action, socket).await,
    }
}

async fn daemon_command(action: DaemonAction, socket: PathBuf) -> anyhow::Result<()> {
    match action {
        DaemonAction::Start { foreground: true } => {
            daemon::run(daemon::DaemonOptions { socket_path: socket, data_dir: proto::paths::data_dir() }).await
        }
        DaemonAction::Start { foreground: false } => {
            spawn::ensure_daemon(&socket).await?;
            println!("daemon running on {}", socket.display());
            Ok(())
        }
        DaemonAction::Stop => {
            let stream = UnixStream::connect(&socket).await.map_err(|_| anyhow::anyhow!("no daemon is running"))?;
            let (mut rd, mut wr) = stream.into_split();
            write_frame(&mut wr, &ClientMsg::Hello { proto_version: PROTO_VERSION, client: ClientKind::Cli }).await?;
            let _welcome: Option<DaemonMsg> = read_frame(&mut rd).await?;
            write_frame(&mut wr, &ClientMsg::Shutdown).await?;
            // Drain until the daemon closes the connection.
            while let Ok(Some(_)) = read_frame::<_, DaemonMsg>(&mut rd).await {}
            println!("daemon stopped");
            Ok(())
        }
        DaemonAction::Status => match UnixStream::connect(&socket).await {
            Ok(stream) => {
                let (mut rd, mut wr) = stream.into_split();
                write_frame(&mut wr, &ClientMsg::Hello { proto_version: PROTO_VERSION, client: ClientKind::Cli }).await?;
                match read_frame::<_, DaemonMsg>(&mut rd).await? {
                    Some(DaemonMsg::Welcome { daemon_version, windows }) => {
                        println!("running: version {daemon_version}, {} window(s), socket {}", windows.len(), socket.display());
                    }
                    Some(DaemonMsg::Error { message, .. }) => println!("running but incompatible: {message}"),
                    _ => println!("running but did not answer the handshake"),
                }
                Ok(())
            }
            Err(_) => {
                println!("not running (socket {})", socket.display());
                Ok(())
            }
        },
    }
}
```

- [ ] **Step 7: Verify the daemon round trip by hand**

Run, with an isolated socket and data dir so your real setup is untouched:

```bash
export ANTHREX_SOCKET=/tmp/anthrex-test.sock ANTHREX_DATA_DIR=/tmp/anthrex-test-data
cargo run -q -p anthrex -- daemon status
cargo run -q -p anthrex -- daemon start
cargo run -q -p anthrex -- daemon status
cargo run -q -p anthrex -- daemon stop
cargo run -q -p anthrex -- daemon status
tail -5 /tmp/anthrex-test-data/daemon.log
```

Expected, in order: `not running ...`; `daemon running on /tmp/anthrex-test.sock`; `running: version 0.1.0, 0 window(s), ...`; `daemon stopped`; `not running ...`; the log ends with `daemon stopped`.

- [ ] **Step 8: Commit**

```bash
git add crates/daemon crates/cli
git commit -m "feat: add daemon lifecycle and anthrex daemon start/stop/status

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 11: `anthrex new`, `ls`, `kill`, `rm`

**Files:**
- Create: `crates/cli/src/client.rs`
- Modify: `crates/cli/src/main.rs`

**Interfaces:**
- Produces: `cli::client::{CliClient, resolve_target, format_table}`:
  - `CliClient::connect(&Path) -> anyhow::Result<CliClient>` with `pub windows: Vec<WindowInfo>`, `pub daemon_version: String`
  - `CliClient::request(&mut self, ClientMsg) -> anyhow::Result<DaemonMsg>` returns the first non-`WindowsChanged` reply
  - `resolve_target(&[WindowInfo], &str) -> anyhow::Result<u32>`, `format_table(&[WindowInfo]) -> String`

- [ ] **Step 1: Write the failing unit tests**

Create `crates/cli/src/client.rs` with only:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use proto::{Runtime, Status};

    fn win(id: u32, name: &str) -> WindowInfo {
        WindowInfo {
            id,
            name: name.into(),
            runtime: Runtime::Shell,
            cwd: "/home/me/repo".into(),
            branch: None,
            status: Status::Idle,
            tool: None,
            since_secs: 5,
            last_output_secs: 5,
            has_session: false,
            exit: None,
        }
    }

    #[test]
    fn resolve_by_id_then_by_name() {
        let ws = vec![win(1, "api"), win(2, "7")];
        assert_eq!(resolve_target(&ws, "1").unwrap(), 1);
        assert_eq!(resolve_target(&ws, "api").unwrap(), 1);
        assert_eq!(resolve_target(&ws, "7").unwrap(), 2, "a numeric name still resolves when no such id exists");
        assert!(resolve_target(&ws, "nope").unwrap_err().to_string().contains("nope"));
    }

    #[test]
    fn table_has_header_and_one_row_per_window() {
        let out = format_table(&[win(1, "api"), win(2, "tests")]);
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), 3);
        assert!(lines[0].starts_with("ID"));
        assert!(lines[1].contains("api") && lines[1].contains("idle") && lines[1].contains("/home/me/repo"));
        assert!(lines[2].contains("tests"));
    }
}
```

Add `mod client;` at the top of `crates/cli/src/main.rs`.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p anthrex`
Expected: compile error, `cannot find function resolve_target`.

- [ ] **Step 3: Implement the client helpers**

Put above the test module in `crates/cli/src/client.rs`:

```rust
//! A small blocking-style client for one-shot CLI commands.

use proto::{ClientKind, ClientMsg, DaemonMsg, PROTO_VERSION, WindowInfo, read_frame, write_frame};
use std::path::Path;
use std::time::Duration;
use tokio::net::UnixStream;
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};

pub struct CliClient {
    rd: OwnedReadHalf,
    wr: OwnedWriteHalf,
    pub windows: Vec<WindowInfo>,
    pub daemon_version: String,
}

impl CliClient {
    pub async fn connect(socket: &Path) -> anyhow::Result<Self> {
        let stream = UnixStream::connect(socket)
            .await
            .map_err(|e| anyhow::anyhow!("cannot reach the daemon at {}: {e}", socket.display()))?;
        let (mut rd, mut wr) = stream.into_split();
        write_frame(&mut wr, &ClientMsg::Hello { proto_version: PROTO_VERSION, client: ClientKind::Cli }).await?;
        match read_frame::<_, DaemonMsg>(&mut rd).await? {
            Some(DaemonMsg::Welcome { daemon_version, windows }) => Ok(Self { rd, wr, windows, daemon_version }),
            Some(DaemonMsg::Error { message, .. }) => anyhow::bail!(message),
            Some(other) => anyhow::bail!("unexpected handshake reply: {other:?}"),
            None => anyhow::bail!("the daemon closed the connection during the handshake"),
        }
    }

    /// Sends one request and returns the first reply that is not a `WindowsChanged` broadcast.
    pub async fn request(&mut self, msg: ClientMsg) -> anyhow::Result<DaemonMsg> {
        write_frame(&mut self.wr, &msg).await?;
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                match read_frame::<_, DaemonMsg>(&mut self.rd).await? {
                    Some(DaemonMsg::WindowsChanged { .. }) => continue,
                    Some(reply) => return Ok(reply),
                    None => anyhow::bail!("the daemon closed the connection"),
                }
            }
        })
        .await
        .map_err(|_| anyhow::anyhow!("timed out waiting for the daemon"))?
    }
}

/// `target` is a window id or a window name. Ids win when both match.
pub fn resolve_target(windows: &[WindowInfo], target: &str) -> anyhow::Result<u32> {
    if let Ok(id) = target.parse::<u32>() {
        if windows.iter().any(|w| w.id == id) {
            return Ok(id);
        }
    }
    if let Some(w) = windows.iter().find(|w| w.name == target) {
        return Ok(w.id);
    }
    anyhow::bail!("no window with id or name '{target}'")
}

pub fn format_table(windows: &[WindowInfo]) -> String {
    let name_w = windows.iter().map(|w| w.name.len()).max().unwrap_or(4).max(4);
    let mut out = format!("{:<4} {:<name_w$} {:<7} {:<10} DIR\n", "ID", "NAME", "RUNTIME", "STATUS");
    for w in windows {
        out.push_str(&format!(
            "{:<4} {:<name_w$} {:<7} {:<10} {}\n",
            w.id,
            w.name,
            w.runtime.label(),
            w.status.label(),
            w.cwd.display()
        ));
    }
    out
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p anthrex`
Expected: `2 passed`.

- [ ] **Step 5: Add the subcommands**

In `crates/cli/src/main.rs`, extend the imports and enums:

```rust
use clap::{Parser, Subcommand, ValueEnum};
use proto::{Runtime, WindowSpec};
```

Add to `Cli` a global option:

```rust
    /// Directory new agents start in (default: the current directory)
    #[arg(long, global = true)]
    dir: Option<PathBuf>,
```

Add these variants to `Command`:

```rust
    /// Create a window and print its id
    New {
        #[arg(long, value_enum, default_value_t = RuntimeArg::Shell)]
        runtime: RuntimeArg,
        #[arg(long)]
        name: Option<String>,
        /// Create a git worktree on this branch (implemented in a later milestone)
        #[arg(long)]
        worktree: Option<String>,
        #[arg(long)]
        model: Option<String>,
        /// Initial prompt for claude or codex
        #[arg(long)]
        prompt: Option<String>,
    },
    /// List windows
    Ls,
    /// Kill a window's process (SIGTERM, then SIGKILL after 3 s)
    Kill { target: String },
    /// Kill and forget a window
    Rm {
        target: String,
        #[arg(long)]
        worktree: bool,
        #[arg(long)]
        force: bool,
    },
```

Add:

```rust
#[derive(Clone, Copy, ValueEnum)]
enum RuntimeArg {
    Claude,
    Codex,
    Shell,
}

impl From<RuntimeArg> for Runtime {
    fn from(r: RuntimeArg) -> Self {
        match r {
            RuntimeArg::Claude => Runtime::Claude,
            RuntimeArg::Codex => Runtime::Codex,
            RuntimeArg::Shell => Runtime::Shell,
        }
    }
}

fn expect_ack(reply: DaemonMsg) -> anyhow::Result<()> {
    match reply {
        DaemonMsg::Ack { .. } => Ok(()),
        DaemonMsg::Error { message, .. } => anyhow::bail!(message),
        other => anyhow::bail!("unexpected reply: {other:?}"),
    }
}
```

Replace the `match cli.command` in `main` with:

```rust
    let dir = match cli.dir {
        Some(d) => d,
        None => std::env::current_dir()?,
    }
    .canonicalize()?;
    match cli.command {
        None => {
            println!("anthrex {} — attach comes in a later task", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Some(Command::Daemon { action }) => daemon_command(action, socket).await,
        Some(Command::New { runtime, name, worktree, model, prompt }) => {
            spawn::ensure_daemon(&socket).await?;
            let mut c = client::CliClient::connect(&socket).await?;
            let spec = WindowSpec {
                name,
                runtime: runtime.into(),
                cwd: dir,
                worktree_branch: worktree,
                model,
                initial_prompt: prompt,
            };
            let (cols, rows) = crossterm::terminal::size().unwrap_or((80, 24));
            match c.request(ClientMsg::CreateWindow { spec, cols, rows }).await? {
                DaemonMsg::Created { window_id } => {
                    println!("{window_id}");
                    Ok(())
                }
                DaemonMsg::Error { message, .. } => anyhow::bail!(message),
                other => anyhow::bail!("unexpected reply: {other:?}"),
            }
        }
        Some(Command::Ls) => {
            let c = client::CliClient::connect(&socket).await?;
            print!("{}", client::format_table(&c.windows));
            Ok(())
        }
        Some(Command::Kill { target }) => {
            let mut c = client::CliClient::connect(&socket).await?;
            let id = client::resolve_target(&c.windows, &target)?;
            expect_ack(c.request(ClientMsg::Kill { window_id: id }).await?)
        }
        Some(Command::Rm { target, worktree, force }) => {
            let mut c = client::CliClient::connect(&socket).await?;
            let id = client::resolve_target(&c.windows, &target)?;
            expect_ack(c.request(ClientMsg::Remove { window_id: id, remove_worktree: worktree, force }).await?)
        }
    }
```

- [ ] **Step 6: Verify by hand**

```bash
export ANTHREX_SOCKET=/tmp/anthrex-test.sock ANTHREX_DATA_DIR=/tmp/anthrex-test-data
cargo run -q -p anthrex -- new --name first
cargo run -q -p anthrex -- new --runtime claude --name cc
cargo run -q -p anthrex -- ls
cargo run -q -p anthrex -- kill first
sleep 4
cargo run -q -p anthrex -- ls
cargo run -q -p anthrex -- rm first
cargo run -q -p anthrex -- rm cc
cargo run -q -p anthrex -- daemon stop
```

Expected: `1`, `2`, a three-line table (header, `first ... shell`, `cc ... claude`), then after the kill `first` shows `exited`, both `rm` calls print nothing, and stop succeeds.

- [ ] **Step 7: Commit**

```bash
git add crates/cli
git commit -m "feat(cli): add new, ls, kill and rm commands with daemon auto-start

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 12: Key encoding and prefix keymap

**Files:**
- Create: `crates/tui/src/keymap.rs`
- Modify: `crates/tui/src/lib.rs`

**Interfaces:**
- Produces: `tui::keymap::{encode_key(KeyEvent, app_cursor: bool) -> Option<Vec<u8>>, Keymap, KeyAction, Command}`:
  - `Keymap::new((KeyCode, KeyModifiers))`, `Keymap::default_prefix() -> (KeyCode, KeyModifiers)` (Ctrl-b), `pending(&self) -> bool`, `handle(&mut self, KeyEvent, app_cursor: bool) -> KeyAction`
  - `KeyAction::{Send(Vec<u8>), Run(Command), AwaitPrefix, Cancel, Nothing}`
  - `Command::{NextWindow, PrevWindow, FocusIndex(usize), NewWindow, KillWindow, RemoveWindow, ToggleSidebar, Detach, StopDaemon, Help}`

- [ ] **Step 1: Write the failing tests**

Create `crates/tui/src/keymap.rs` with only:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn key(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, mods)
    }

    #[test]
    fn plain_and_control_characters() {
        assert_eq!(encode_key(key(KeyCode::Char('a'), KeyModifiers::NONE), false), Some(b"a".to_vec()));
        assert_eq!(encode_key(key(KeyCode::Char('é'), KeyModifiers::NONE), false), Some("é".as_bytes().to_vec()));
        assert_eq!(encode_key(key(KeyCode::Char('c'), KeyModifiers::CONTROL), false), Some(vec![0x03]));
        assert_eq!(encode_key(key(KeyCode::Char('C'), KeyModifiers::CONTROL | KeyModifiers::SHIFT), false), Some(vec![0x03]));
        assert_eq!(encode_key(key(KeyCode::Char('['), KeyModifiers::CONTROL), false), Some(vec![0x1b]));
        assert_eq!(encode_key(key(KeyCode::Char(' '), KeyModifiers::CONTROL), false), Some(vec![0x00]));
        assert_eq!(encode_key(key(KeyCode::Char('x'), KeyModifiers::ALT), false), Some(vec![0x1b, b'x']));
        assert_eq!(encode_key(key(KeyCode::Enter, KeyModifiers::NONE), false), Some(vec![b'\r']));
        assert_eq!(encode_key(key(KeyCode::Backspace, KeyModifiers::NONE), false), Some(vec![0x7f]));
        assert_eq!(encode_key(key(KeyCode::Esc, KeyModifiers::NONE), false), Some(vec![0x1b]));
        assert_eq!(encode_key(key(KeyCode::Tab, KeyModifiers::NONE), false), Some(vec![b'\t']));
        assert_eq!(encode_key(key(KeyCode::BackTab, KeyModifiers::SHIFT), false), Some(b"\x1b[Z".to_vec()));
    }

    #[test]
    fn cursor_keys_honour_application_mode_and_modifiers() {
        assert_eq!(encode_key(key(KeyCode::Up, KeyModifiers::NONE), false), Some(b"\x1b[A".to_vec()));
        assert_eq!(encode_key(key(KeyCode::Up, KeyModifiers::NONE), true), Some(b"\x1bOA".to_vec()));
        assert_eq!(encode_key(key(KeyCode::Left, KeyModifiers::SHIFT), false), Some(b"\x1b[1;2D".to_vec()));
        assert_eq!(encode_key(key(KeyCode::Right, KeyModifiers::CONTROL), true), Some(b"\x1b[1;5C".to_vec()));
        assert_eq!(encode_key(key(KeyCode::Home, KeyModifiers::NONE), true), Some(b"\x1bOH".to_vec()));
        assert_eq!(encode_key(key(KeyCode::Delete, KeyModifiers::NONE), false), Some(b"\x1b[3~".to_vec()));
        assert_eq!(encode_key(key(KeyCode::PageUp, KeyModifiers::SHIFT), false), Some(b"\x1b[5;2~".to_vec()));
        assert_eq!(encode_key(key(KeyCode::F(1), KeyModifiers::NONE), false), Some(b"\x1bOP".to_vec()));
        assert_eq!(encode_key(key(KeyCode::F(5), KeyModifiers::NONE), false), Some(b"\x1b[15~".to_vec()));
        assert_eq!(encode_key(key(KeyCode::F(12), KeyModifiers::NONE), false), Some(b"\x1b[24~".to_vec()));
        assert_eq!(encode_key(key(KeyCode::CapsLock, KeyModifiers::NONE), false), None);
    }

    #[test]
    fn prefix_then_command() {
        let mut km = Keymap::new(Keymap::default_prefix());
        let prefix = key(KeyCode::Char('b'), KeyModifiers::CONTROL);
        assert_eq!(km.handle(prefix, false), KeyAction::AwaitPrefix);
        assert!(km.pending());
        assert_eq!(km.handle(key(KeyCode::Char('j'), KeyModifiers::NONE), false), KeyAction::Run(Command::NextWindow));
        assert!(!km.pending());
        assert_eq!(km.handle(prefix, false), KeyAction::AwaitPrefix);
        assert_eq!(km.handle(key(KeyCode::Char('3'), KeyModifiers::NONE), false), KeyAction::Run(Command::FocusIndex(2)));
        assert_eq!(km.handle(prefix, false), KeyAction::AwaitPrefix);
        assert_eq!(km.handle(key(KeyCode::Char('X'), KeyModifiers::SHIFT), false), KeyAction::Run(Command::RemoveWindow));
        assert_eq!(km.handle(prefix, false), KeyAction::AwaitPrefix);
        assert_eq!(km.handle(key(KeyCode::Char('?'), KeyModifiers::NONE), false), KeyAction::Run(Command::Help));
    }

    #[test]
    fn prefix_twice_sends_literal_and_escape_cancels() {
        let mut km = Keymap::new(Keymap::default_prefix());
        let prefix = key(KeyCode::Char('b'), KeyModifiers::CONTROL);
        km.handle(prefix, false);
        assert_eq!(km.handle(prefix, false), KeyAction::Send(vec![0x02]));
        km.handle(prefix, false);
        assert_eq!(km.handle(key(KeyCode::Esc, KeyModifiers::NONE), false), KeyAction::Cancel);
        assert!(!km.pending());
        km.handle(prefix, false);
        assert_eq!(km.handle(key(KeyCode::Char('z'), KeyModifiers::NONE), false), KeyAction::Nothing);
    }

    #[test]
    fn ordinary_keys_pass_through_and_releases_are_ignored() {
        let mut km = Keymap::new(Keymap::default_prefix());
        assert_eq!(km.handle(key(KeyCode::Char('q'), KeyModifiers::NONE), false), KeyAction::Send(b"q".to_vec()));
        let mut release = key(KeyCode::Char('q'), KeyModifiers::NONE);
        release.kind = crossterm::event::KeyEventKind::Release;
        assert_eq!(km.handle(release, false), KeyAction::Nothing);
    }
}
```

Add to `crates/tui/src/lib.rs`:

```rust
pub mod keymap;
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p anthrex-tui keymap`
Expected: compile error, `cannot find function encode_key`.

- [ ] **Step 3: Implement the keymap**

Put above the test module in `crates/tui/src/keymap.rs`:

```rust
//! Turns crossterm key events into PTY bytes, and implements the tmux-style prefix key.

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    NextWindow,
    PrevWindow,
    FocusIndex(usize),
    NewWindow,
    KillWindow,
    RemoveWindow,
    ToggleSidebar,
    Detach,
    StopDaemon,
    Help,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyAction {
    /// Write these bytes to the focused PTY.
    Send(Vec<u8>),
    Run(Command),
    /// The prefix was pressed; the next key picks a command.
    AwaitPrefix,
    /// Prefix mode was cancelled with Esc.
    Cancel,
    Nothing,
}

#[derive(Debug, Clone)]
pub struct Keymap {
    prefix: (KeyCode, KeyModifiers),
    pending: bool,
}

impl Keymap {
    pub fn default_prefix() -> (KeyCode, KeyModifiers) {
        (KeyCode::Char('b'), KeyModifiers::CONTROL)
    }

    pub fn new(prefix: (KeyCode, KeyModifiers)) -> Self {
        Self { prefix, pending: false }
    }

    pub fn pending(&self) -> bool {
        self.pending
    }

    fn is_prefix(&self, key: &KeyEvent) -> bool {
        key.code == self.prefix.0 && key.modifiers == self.prefix.1
    }

    pub fn handle(&mut self, key: KeyEvent, app_cursor: bool) -> KeyAction {
        if key.kind == KeyEventKind::Release {
            return KeyAction::Nothing;
        }
        if self.pending {
            self.pending = false;
            if self.is_prefix(&key) {
                return encode_key(KeyEvent::new(self.prefix.0, self.prefix.1), app_cursor)
                    .map(KeyAction::Send)
                    .unwrap_or(KeyAction::Nothing);
            }
            return match key.code {
                KeyCode::Char('j') | KeyCode::Char('n') => KeyAction::Run(Command::NextWindow),
                KeyCode::Char('k') | KeyCode::Char('p') => KeyAction::Run(Command::PrevWindow),
                KeyCode::Char(d @ '1'..='9') => KeyAction::Run(Command::FocusIndex(d as usize - '1' as usize)),
                KeyCode::Char('c') => KeyAction::Run(Command::NewWindow),
                KeyCode::Char('x') => KeyAction::Run(Command::KillWindow),
                KeyCode::Char('X') => KeyAction::Run(Command::RemoveWindow),
                KeyCode::Char('s') => KeyAction::Run(Command::ToggleSidebar),
                KeyCode::Char('d') => KeyAction::Run(Command::Detach),
                KeyCode::Char('Q') => KeyAction::Run(Command::StopDaemon),
                KeyCode::Char('?') => KeyAction::Run(Command::Help),
                KeyCode::Esc => KeyAction::Cancel,
                _ => KeyAction::Nothing,
            };
        }
        if self.is_prefix(&key) {
            self.pending = true;
            return KeyAction::AwaitPrefix;
        }
        encode_key(key, app_cursor).map(KeyAction::Send).unwrap_or(KeyAction::Nothing)
    }
}

/// Encodes a key the way an xterm-compatible terminal would send it.
/// `app_cursor` is the DECCKM state of the receiving program (cursor keys send `ESC O x`).
pub fn encode_key(key: KeyEvent, app_cursor: bool) -> Option<Vec<u8>> {
    use KeyCode::*;
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    let shift = key.modifiers.contains(KeyModifiers::SHIFT);
    let modifier = 1 + u8::from(shift) + 2 * u8::from(alt) + 4 * u8::from(ctrl);

    let cursor = |final_byte: char| -> Vec<u8> {
        if modifier == 1 {
            if app_cursor { format!("\x1bO{final_byte}") } else { format!("\x1b[{final_byte}") }
        } else {
            format!("\x1b[1;{modifier}{final_byte}")
        }
        .into_bytes()
    };
    let tilde = |code: u8| -> Vec<u8> {
        if modifier == 1 { format!("\x1b[{code}~") } else { format!("\x1b[{code};{modifier}~") }.into_bytes()
    };

    let bytes = match key.code {
        Char(c) => {
            let mut out = Vec::with_capacity(5);
            if alt {
                out.push(0x1b);
            }
            if ctrl {
                let byte = match c.to_ascii_lowercase() {
                    l @ 'a'..='z' => l as u8 - b'a' + 1,
                    ' ' | '@' | '2' => 0x00,
                    '[' | '3' => 0x1b,
                    '\\' | '4' => 0x1c,
                    ']' | '5' => 0x1d,
                    '^' | '6' => 0x1e,
                    '_' | '7' | '/' => 0x1f,
                    '?' | '8' => 0x7f,
                    _ => return None,
                };
                out.push(byte);
            } else {
                let mut buf = [0u8; 4];
                out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
            }
            out
        }
        Enter => vec![b'\r'],
        Tab => vec![b'\t'],
        BackTab => b"\x1b[Z".to_vec(),
        Backspace => {
            if alt { vec![0x1b, 0x7f] } else { vec![0x7f] }
        }
        Esc => vec![0x1b],
        Up => cursor('A'),
        Down => cursor('B'),
        Right => cursor('C'),
        Left => cursor('D'),
        Home => cursor('H'),
        End => cursor('F'),
        Insert => tilde(2),
        Delete => tilde(3),
        PageUp => tilde(5),
        PageDown => tilde(6),
        F(n) => match n {
            1 => b"\x1bOP".to_vec(),
            2 => b"\x1bOQ".to_vec(),
            3 => b"\x1bOR".to_vec(),
            4 => b"\x1bOS".to_vec(),
            5 => tilde(15),
            6 => tilde(17),
            7 => tilde(18),
            8 => tilde(19),
            9 => tilde(20),
            10 => tilde(21),
            11 => tilde(23),
            12 => tilde(24),
            _ => return None,
        },
        _ => return None,
    };
    Some(bytes)
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p anthrex-tui keymap`
Expected: `5 passed`.

- [ ] **Step 5: Commit**

```bash
git add crates/tui
git commit -m "feat(tui): add xterm key encoding and prefix keymap

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 13: Daemon connection for the client

**Files:**
- Create: `crates/tui/src/connection.rs`, `crates/tui/tests/connection.rs`
- Modify: `crates/tui/src/lib.rs`

**Interfaces:**
- Produces: `tui::connection::Connection`:
  - `Connection::connect(&Path) -> anyhow::Result<Connection>` performs the handshake; `pub windows: Vec<WindowInfo>`, `pub daemon_version: String`
  - `send(&self, ClientMsg) -> bool` (false once the socket is gone), `recv(&mut self) -> Option<DaemonMsg>` (None once the socket is gone)

- [ ] **Step 1: Write the failing integration test**

Create `crates/tui/tests/connection.rs`:

```rust
use daemon::manager::WindowManager;
use daemon::server::serve;
use proto::{ClientMsg, DaemonMsg, Runtime, WindowSpec};
use std::path::PathBuf;
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use tui::connection::Connection;

async fn start_daemon() -> (tempfile::TempDir, PathBuf, CancellationToken) {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("d.sock");
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    let (manager, mut events) = WindowManager::new(socket.clone(), "/bin/sh".into());
    let pump = manager.clone();
    tokio::spawn(async move {
        while let Some((id, ev)) = events.recv().await {
            pump.handle_event(id, ev);
        }
    });
    let token = CancellationToken::new();
    tokio::spawn(serve(listener, manager, token.clone()));
    (dir, socket, token)
}

async fn recv_until(conn: &mut Connection, pred: impl Fn(&DaemonMsg) -> bool) -> DaemonMsg {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let m = conn.recv().await.expect("connection closed");
            if pred(&m) {
                return m;
            }
        }
    })
    .await
    .expect("timed out")
}

#[tokio::test]
async fn connects_creates_and_receives_output() {
    let (_dir, socket, _token) = start_daemon().await;
    let mut conn = Connection::connect(&socket).await.unwrap();
    assert!(conn.windows.is_empty());
    assert!(!conn.daemon_version.is_empty());

    let spec = WindowSpec {
        name: Some("w".into()),
        runtime: Runtime::Shell,
        cwd: std::env::temp_dir(),
        worktree_branch: None,
        model: None,
        initial_prompt: None,
    };
    assert!(conn.send(ClientMsg::CreateWindow { spec, cols: 80, rows: 24 }).await);
    let DaemonMsg::Created { window_id } = recv_until(&mut conn, |m| matches!(m, DaemonMsg::Created { .. })).await else {
        unreachable!()
    };
    assert!(conn.send(ClientMsg::Subscribe { window_id, cols: 80, rows: 24 }).await);
    recv_until(&mut conn, |m| matches!(m, DaemonMsg::Snapshot { .. })).await;
    assert!(conn.send(ClientMsg::Input { window_id, bytes: b"echo conn-$((3+3))\n".to_vec() }).await);
    let mut seen = Vec::new();
    recv_until(&mut conn, |m| {
        if let DaemonMsg::Output { bytes, .. } = m {
            seen.extend_from_slice(bytes);
        }
        String::from_utf8_lossy(&seen).contains("conn-6")
    })
    .await;
    assert!(conn.send(ClientMsg::Remove { window_id, remove_worktree: false, force: false }).await);
}

#[tokio::test]
async fn recv_returns_none_after_daemon_shutdown() {
    let (_dir, socket, token) = start_daemon().await;
    let mut conn = Connection::connect(&socket).await.unwrap();
    token.cancel();
    recv_until(&mut conn, |m| matches!(m, DaemonMsg::Bye { .. })).await;
    let after = tokio::time::timeout(Duration::from_secs(5), conn.recv()).await.expect("timed out");
    assert!(after.is_none());
}

#[tokio::test]
async fn connect_fails_cleanly_without_a_daemon() {
    let dir = tempfile::tempdir().unwrap();
    let err = Connection::connect(&dir.path().join("missing.sock")).await.unwrap_err();
    assert!(err.to_string().contains("missing.sock"));
}
```

Add to `crates/tui/src/lib.rs`:

```rust
pub mod connection;
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p anthrex-tui --test connection`
Expected: compile error, `unresolved import tui::connection`.

- [ ] **Step 3: Implement the connection**

Create `crates/tui/src/connection.rs`:

```rust
//! Async client for the daemon socket: one reader task, one writer task, channels in between.

use proto::{ClientKind, ClientMsg, DaemonMsg, PROTO_VERSION, WindowInfo, read_frame, write_frame};
use std::path::Path;
use tokio::net::UnixStream;
use tokio::sync::mpsc;

pub struct Connection {
    tx: mpsc::Sender<ClientMsg>,
    rx: mpsc::Receiver<DaemonMsg>,
    pub windows: Vec<WindowInfo>,
    pub daemon_version: String,
}

impl Connection {
    pub async fn connect(socket: &Path) -> anyhow::Result<Self> {
        let stream = UnixStream::connect(socket)
            .await
            .map_err(|e| anyhow::anyhow!("cannot connect to the daemon at {}: {e}", socket.display()))?;
        let (mut rd, mut wr) = stream.into_split();
        write_frame(&mut wr, &ClientMsg::Hello { proto_version: PROTO_VERSION, client: ClientKind::Tui }).await?;
        let (daemon_version, windows) = match read_frame::<_, DaemonMsg>(&mut rd).await? {
            Some(DaemonMsg::Welcome { daemon_version, windows }) => (daemon_version, windows),
            Some(DaemonMsg::Error { message, .. }) => anyhow::bail!(message),
            Some(other) => anyhow::bail!("unexpected handshake reply: {other:?}"),
            None => anyhow::bail!("the daemon closed the connection during the handshake"),
        };

        let (tx, mut out_rx) = mpsc::channel::<ClientMsg>(256);
        tokio::spawn(async move {
            while let Some(msg) = out_rx.recv().await {
                if write_frame(&mut wr, &msg).await.is_err() {
                    break;
                }
            }
        });

        let (in_tx, rx) = mpsc::channel::<DaemonMsg>(1024);
        tokio::spawn(async move {
            loop {
                match read_frame::<_, DaemonMsg>(&mut rd).await {
                    Ok(Some(msg)) => {
                        if in_tx.send(msg).await.is_err() {
                            break;
                        }
                    }
                    Ok(None) | Err(_) => break,
                }
            }
            // Dropping in_tx closes the channel; recv() then yields None.
        });

        Ok(Self { tx, rx, windows, daemon_version })
    }

    /// Returns false if the connection is gone.
    pub async fn send(&self, msg: ClientMsg) -> bool {
        self.tx.send(msg).await.is_ok()
    }

    /// Returns None once the daemon has closed the connection.
    pub async fn recv(&mut self) -> Option<DaemonMsg> {
        self.rx.recv().await
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p anthrex-tui --test connection`
Expected: `3 passed`.

- [ ] **Step 5: Commit**

```bash
git add crates/tui
git commit -m "feat(tui): add async daemon connection

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 14: Application state and update logic

**Files:**
- Create: `crates/tui/src/app.rs`
- Modify: `crates/tui/src/lib.rs`

**Interfaces:**
- Consumes: `keymap::{Keymap, KeyAction, Command}`.
- Produces: `tui::app::{App, Effect, Modal, PendingAction, SCROLLBACK_LINES, TOAST_TTL, RESIZE_DEBOUNCE}`:
  - `App::new(windows: Vec<WindowInfo>, default_dir: PathBuf, prefix: (KeyCode, KeyModifiers)) -> App`
  - public fields: `windows`, `focused: Option<u32>`, `parser: vt100::Parser`, `sidebar_visible: bool`, `keymap: Keymap`, `modal: Option<Modal>`, `connected: bool`, `spinner_frame: usize`, `scroll_offset: usize`, `default_dir: PathBuf`
  - `set_terminal_size(&mut self, cols, rows) -> Vec<Effect>`, `on_daemon(&mut self, DaemonMsg) -> Vec<Effect>`, `on_key(&mut self, KeyEvent) -> Vec<Effect>`, `on_paste(&mut self, String) -> Vec<Effect>`, `on_scroll(&mut self, up: bool, col: u16, row: u16, main_inner: Rect) -> Vec<Effect>`, `on_tick(&mut self) -> Vec<Effect>`, `focus(&mut self, id) -> Vec<Effect>`, `request_focus(&mut self, id)`, `focused_window(&self) -> Option<&WindowInfo>`, `focused_index(&self) -> Option<usize>`, `elapsed_secs(&self, &WindowInfo) -> u64`, `toast_text(&self) -> Option<&str>`, `toast(&mut self, impl Into<String>)`
  - `Effect::{Send(ClientMsg), Quit}`; `Modal::{Confirm { message, action: PendingAction }, Help}`; `PendingAction::{Kill(u32), Remove(u32), StopDaemon}`
  - The `Rect` type is `ratatui::layout::Rect`; the renderer (Task 15) computes it.

- [ ] **Step 1: Write the failing tests**

Create `crates/tui/src/app.rs` with only:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use proto::{Runtime, Status};

    fn win(id: u32, name: &str, status: Status) -> WindowInfo {
        WindowInfo {
            id,
            name: name.into(),
            runtime: Runtime::Shell,
            cwd: "/tmp".into(),
            branch: None,
            status,
            tool: None,
            since_secs: 0,
            last_output_secs: 0,
            has_session: false,
            exit: None,
        }
    }

    fn app_with(windows: Vec<WindowInfo>) -> App {
        let mut app = App::new(windows, "/tmp".into(), Keymap::default_prefix());
        // The renderer reports the size on the first draw; simulate that.
        let _ = app.set_terminal_size(80, 24);
        app
    }

    fn key(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, mods)
    }

    fn press(app: &mut App, code: KeyCode, mods: KeyModifiers) -> Vec<Effect> {
        app.on_key(key(code, mods))
    }

    fn prefix(app: &mut App) {
        assert!(press(app, KeyCode::Char('b'), KeyModifiers::CONTROL).is_empty());
    }

    #[test]
    fn first_size_report_subscribes_to_the_first_window() {
        let mut app = App::new(vec![win(4, "a", Status::Idle), win(5, "b", Status::Idle)], "/tmp".into(), Keymap::default_prefix());
        assert_eq!(app.focused, None);
        let effects = app.set_terminal_size(100, 30);
        assert_eq!(effects, vec![Effect::Send(ClientMsg::Subscribe { window_id: 4, cols: 100, rows: 30 })]);
        assert_eq!(app.focused, Some(4));
        assert_eq!(app.parser.screen().size(), (30, 100));
    }

    #[test]
    fn later_size_changes_are_debounced_into_a_resize() {
        let mut app = app_with(vec![win(1, "a", Status::Idle)]);
        assert!(app.set_terminal_size(120, 40).is_empty());
        assert!(app.on_tick().is_empty(), "resize is not sent before the debounce window");
        std::thread::sleep(RESIZE_DEBOUNCE + Duration::from_millis(5));
        assert_eq!(app.on_tick(), vec![Effect::Send(ClientMsg::Resize { window_id: 1, cols: 120, rows: 40 })]);
        assert!(app.on_tick().is_empty());
    }

    #[test]
    fn snapshot_and_output_feed_the_focused_parser_only() {
        let mut app = app_with(vec![win(1, "a", Status::Idle), win(2, "b", Status::Idle)]);
        app.on_daemon(DaemonMsg::Snapshot { window_id: 1, cols: 80, rows: 24, bytes: b"hello".to_vec() });
        assert!(app.parser.screen().contents().starts_with("hello"));
        app.on_daemon(DaemonMsg::Output { window_id: 2, bytes: b"IGNORED".to_vec() });
        assert!(!app.parser.screen().contents().contains("IGNORED"));
        app.on_daemon(DaemonMsg::Output { window_id: 1, bytes: b" world".to_vec() });
        assert!(app.parser.screen().contents().starts_with("hello world"));
    }

    #[test]
    fn keys_go_to_the_focused_window_and_prefix_switches() {
        let mut app = app_with(vec![win(1, "a", Status::Idle), win(2, "b", Status::Idle)]);
        assert_eq!(press(&mut app, KeyCode::Char('l'), KeyModifiers::NONE), vec![Effect::Send(ClientMsg::Input { window_id: 1, bytes: b"l".to_vec() })]);
        prefix(&mut app);
        assert_eq!(press(&mut app, KeyCode::Char('j'), KeyModifiers::NONE), vec![Effect::Send(ClientMsg::Subscribe { window_id: 2, cols: 80, rows: 24 })]);
        assert_eq!(app.focused, Some(2));
        prefix(&mut app);
        press(&mut app, KeyCode::Char('j'), KeyModifiers::NONE);
        assert_eq!(app.focused, Some(1), "next wraps around");
        prefix(&mut app);
        press(&mut app, KeyCode::Char('2'), KeyModifiers::NONE);
        assert_eq!(app.focused, Some(2));
        prefix(&mut app);
        assert_eq!(press(&mut app, KeyCode::Char('d'), KeyModifiers::NONE), vec![Effect::Quit]);
    }

    #[test]
    fn new_window_creates_a_shell_in_the_default_dir_and_focuses_it_when_created() {
        let mut app = app_with(vec![]);
        prefix(&mut app);
        let effects = press(&mut app, KeyCode::Char('c'), KeyModifiers::NONE);
        match &effects[..] {
            [Effect::Send(ClientMsg::CreateWindow { spec, cols: 80, rows: 24 })] => {
                assert_eq!(spec.runtime, Runtime::Shell);
                assert_eq!(spec.cwd, PathBuf::from("/tmp"));
                assert_eq!(spec.name, None);
            }
            other => panic!("unexpected effects {other:?}"),
        }
        // The daemon may announce the window list before or after Created; both orders focus it.
        assert!(app.on_daemon(DaemonMsg::Created { window_id: 9 }).is_empty());
        let effects = app.on_daemon(DaemonMsg::WindowsChanged { windows: vec![win(9, "shell-9", Status::Starting)] });
        assert_eq!(effects, vec![Effect::Send(ClientMsg::Subscribe { window_id: 9, cols: 80, rows: 24 })]);
        assert_eq!(app.focused, Some(9));
    }

    #[test]
    fn kill_asks_for_confirmation_first() {
        let mut app = app_with(vec![win(1, "a", Status::Working)]);
        prefix(&mut app);
        assert!(press(&mut app, KeyCode::Char('x'), KeyModifiers::NONE).is_empty());
        assert!(matches!(app.modal, Some(Modal::Confirm { action: PendingAction::Kill(1), .. })));
        assert!(press(&mut app, KeyCode::Char('n'), KeyModifiers::NONE).is_empty());
        assert!(app.modal.is_none());
        prefix(&mut app);
        press(&mut app, KeyCode::Char('x'), KeyModifiers::NONE);
        assert_eq!(press(&mut app, KeyCode::Char('y'), KeyModifiers::NONE), vec![Effect::Send(ClientMsg::Kill { window_id: 1 })]);
        prefix(&mut app);
        press(&mut app, KeyCode::Char('Q'), KeyModifiers::SHIFT);
        assert_eq!(press(&mut app, KeyCode::Enter, KeyModifiers::NONE), vec![Effect::Send(ClientMsg::Shutdown), Effect::Quit]);
    }

    #[test]
    fn removed_focused_window_moves_focus_to_a_neighbour() {
        let mut app = app_with(vec![win(1, "a", Status::Idle), win(2, "b", Status::Idle), win(3, "c", Status::Idle)]);
        app.focus(2);
        let effects = app.on_daemon(DaemonMsg::WindowsChanged { windows: vec![win(1, "a", Status::Idle), win(3, "c", Status::Idle)] });
        assert_eq!(effects, vec![Effect::Send(ClientMsg::Subscribe { window_id: 3, cols: 80, rows: 24 })]);
        let effects = app.on_daemon(DaemonMsg::WindowsChanged { windows: vec![] });
        assert!(effects.is_empty());
        assert_eq!(app.focused, None);
    }

    #[test]
    fn background_status_changes_raise_toasts() {
        let mut app = app_with(vec![win(1, "a", Status::Working), win(2, "b", Status::Working)]);
        app.on_daemon(DaemonMsg::WindowsChanged { windows: vec![win(1, "a", Status::Attention), win(2, "b", Status::Attention)] });
        assert_eq!(app.toast_text(), Some("b needs attention"), "the focused window (1) never toasts");
        app.on_daemon(DaemonMsg::WindowsChanged { windows: vec![win(1, "a", Status::Attention), win(2, "b", Status::Done)] });
        assert_eq!(app.toast_text(), Some("b finished"));
        app.on_daemon(DaemonMsg::Error { request: "kill".into(), message: "no window with id 7".into() });
        assert_eq!(app.toast_text(), Some("no window with id 7"));
    }

    #[test]
    fn paste_uses_bracketed_mode_when_the_program_asked_for_it() {
        let mut app = app_with(vec![win(1, "a", Status::Idle)]);
        assert_eq!(app.on_paste("ab\ncd".into()), vec![Effect::Send(ClientMsg::Input { window_id: 1, bytes: b"ab\rcd".to_vec() })]);
        app.parser.process(b"\x1b[?2004h");
        assert_eq!(app.on_paste("x".into()), vec![Effect::Send(ClientMsg::Input { window_id: 1, bytes: b"\x1b[200~x\x1b[201~".to_vec() })]);
    }

    #[test]
    fn wheel_scrolls_the_local_scrollback_and_any_key_snaps_back() {
        let mut app = app_with(vec![win(1, "a", Status::Idle)]);
        for i in 0..40 {
            app.parser.process(format!("line {i}\r\n").as_bytes());
        }
        let area = ratatui::layout::Rect::new(0, 0, 80, 24);
        assert!(app.on_scroll(true, 5, 5, area).is_empty());
        assert_eq!(app.scroll_offset, 3);
        press(&mut app, KeyCode::Char('q'), KeyModifiers::NONE);
        assert_eq!(app.scroll_offset, 0);
        app.parser.process(b"\x1b[?1000h\x1b[?1006h");
        let effects = app.on_scroll(false, 5, 5, area);
        assert_eq!(effects, vec![Effect::Send(ClientMsg::Input { window_id: 1, bytes: b"\x1b[<65;6;6M".to_vec() })]);
    }
}
```

Add to `crates/tui/src/lib.rs`:

```rust
pub mod app;
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p anthrex-tui app`
Expected: compile error, `cannot find type App`.

- [ ] **Step 3: Implement the app**

Put above the test module in `crates/tui/src/app.rs`:

```rust
//! Client state and its pure update functions. Rendering lives in `ui`; I/O in `lib.rs`.

use crate::keymap::{Command, KeyAction, Keymap};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use proto::{ClientMsg, DaemonMsg, Runtime, Status, WindowInfo, WindowSpec};
use ratatui::layout::Rect;
use std::path::PathBuf;
use std::time::{Duration, Instant};

pub const SCROLLBACK_LINES: usize = 5000;
pub const TOAST_TTL: Duration = Duration::from_secs(4);
pub const RESIZE_DEBOUNCE: Duration = Duration::from_millis(30);

#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    Send(ClientMsg),
    Quit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PendingAction {
    Kill(u32),
    Remove(u32),
    StopDaemon,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Modal {
    Confirm { message: String, action: PendingAction },
    Help,
}

pub struct App {
    pub windows: Vec<WindowInfo>,
    pub focused: Option<u32>,
    pub parser: vt100::Parser,
    pub sidebar_visible: bool,
    pub keymap: Keymap,
    pub modal: Option<Modal>,
    pub connected: bool,
    pub spinner_frame: usize,
    pub scroll_offset: usize,
    pub default_dir: PathBuf,
    toast: Option<(String, Instant)>,
    windows_received_at: Instant,
    /// (cols, rows) of the main inner area; (0, 0) until the first draw.
    term_size: (u16, u16),
    pending_resize: Option<Instant>,
    pending_focus: Option<u32>,
}

impl App {
    pub fn new(windows: Vec<WindowInfo>, default_dir: PathBuf, prefix: (KeyCode, KeyModifiers)) -> Self {
        Self {
            windows,
            focused: None,
            parser: vt100::Parser::new(24, 80, SCROLLBACK_LINES),
            sidebar_visible: true,
            keymap: Keymap::new(prefix),
            modal: None,
            connected: true,
            spinner_frame: 0,
            scroll_offset: 0,
            default_dir,
            toast: None,
            windows_received_at: Instant::now(),
            term_size: (0, 0),
            pending_resize: None,
            pending_focus: None,
        }
    }

    pub fn focused_window(&self) -> Option<&WindowInfo> {
        self.focused.and_then(|id| self.windows.iter().find(|w| w.id == id))
    }

    pub fn focused_index(&self) -> Option<usize> {
        self.focused.and_then(|id| self.windows.iter().position(|w| w.id == id))
    }

    /// Seconds since the window's status changed, extrapolated from the last list we received.
    pub fn elapsed_secs(&self, w: &WindowInfo) -> u64 {
        w.since_secs + self.windows_received_at.elapsed().as_secs()
    }

    pub fn toast_text(&self) -> Option<&str> {
        self.toast.as_ref().map(|(t, _)| t.as_str())
    }

    pub fn toast(&mut self, text: impl Into<String>) {
        self.toast = Some((text.into(), Instant::now()));
    }

    /// Focus this window as soon as it appears in the list (used for `Created` and `attach <name>`).
    pub fn request_focus(&mut self, id: u32) {
        self.pending_focus = Some(id);
    }

    /// The renderer calls this with the main inner area after every draw.
    pub fn set_terminal_size(&mut self, cols: u16, rows: u16) -> Vec<Effect> {
        if cols == 0 || rows == 0 || (cols, rows) == self.term_size {
            return vec![];
        }
        let first = self.term_size == (0, 0);
        self.term_size = (cols, rows);
        self.parser.screen_mut().set_size(rows, cols);
        if first {
            return self.ensure_focus();
        }
        self.pending_resize = Some(Instant::now());
        vec![]
    }

    fn ensure_focus(&mut self) -> Vec<Effect> {
        if self.term_size == (0, 0) {
            return vec![];
        }
        if let Some(id) = self.pending_focus {
            if self.windows.iter().any(|w| w.id == id) {
                self.pending_focus = None;
                return self.focus(id);
            }
        }
        if self.focused_window().is_some() {
            return vec![];
        }
        match self.windows.first().map(|w| w.id) {
            Some(id) => self.focus(id),
            None => {
                self.focused = None;
                vec![]
            }
        }
    }

    pub fn focus(&mut self, id: u32) -> Vec<Effect> {
        if !self.windows.iter().any(|w| w.id == id) {
            return vec![];
        }
        self.focused = Some(id);
        self.scroll_offset = 0;
        let (cols, rows) = self.term_size;
        self.parser = vt100::Parser::new(rows.max(1), cols.max(1), SCROLLBACK_LINES);
        if self.term_size == (0, 0) {
            return vec![];
        }
        vec![Effect::Send(ClientMsg::Subscribe { window_id: id, cols, rows })]
    }

    fn focus_relative(&mut self, delta: isize) -> Vec<Effect> {
        if self.windows.is_empty() {
            return vec![];
        }
        let len = self.windows.len() as isize;
        let current = self.focused_index().map(|i| i as isize).unwrap_or(0);
        let next = (current + delta).rem_euclid(len) as usize;
        let id = self.windows[next].id;
        self.focus(id)
    }

    pub fn on_daemon(&mut self, msg: DaemonMsg) -> Vec<Effect> {
        match msg {
            DaemonMsg::Welcome { windows, .. } | DaemonMsg::WindowsChanged { windows } => self.replace_windows(windows),
            DaemonMsg::Created { window_id } => {
                if self.windows.iter().any(|w| w.id == window_id) {
                    self.focus(window_id)
                } else {
                    self.pending_focus = Some(window_id);
                    vec![]
                }
            }
            DaemonMsg::Snapshot { window_id, cols, rows, bytes } => {
                if Some(window_id) == self.focused {
                    self.parser = vt100::Parser::new(rows.max(1), cols.max(1), SCROLLBACK_LINES);
                    self.parser.process(&bytes);
                    self.scroll_offset = 0;
                }
                vec![]
            }
            DaemonMsg::Output { window_id, bytes } => {
                if Some(window_id) == self.focused {
                    self.parser.process(&bytes);
                }
                vec![]
            }
            DaemonMsg::Error { message, .. } => {
                self.toast(message);
                vec![]
            }
            DaemonMsg::Bye { reason } => {
                self.connected = false;
                self.toast(format!("daemon: {reason}"));
                vec![]
            }
            DaemonMsg::Ack { .. } => vec![],
        }
    }

    fn replace_windows(&mut self, windows: Vec<WindowInfo>) -> Vec<Effect> {
        for w in &windows {
            if Some(w.id) == self.focused {
                continue;
            }
            let previous = self.windows.iter().find(|old| old.id == w.id).map(|old| old.status);
            if previous != Some(w.status) {
                match w.status {
                    Status::Attention => self.toast(format!("{} needs attention", w.name)),
                    Status::Done => self.toast(format!("{} finished", w.name)),
                    _ => {}
                }
            }
        }
        let previous_index = self.focused_index();
        self.windows = windows;
        self.windows_received_at = Instant::now();

        if let Some(id) = self.pending_focus {
            if self.windows.iter().any(|w| w.id == id) {
                self.pending_focus = None;
                return self.focus(id);
            }
        }
        if self.focused_window().is_none() {
            if let Some(i) = previous_index {
                if !self.windows.is_empty() {
                    let id = self.windows[i.min(self.windows.len() - 1)].id;
                    return self.focus(id);
                }
            }
            return self.ensure_focus();
        }
        vec![]
    }

    pub fn on_key(&mut self, key: KeyEvent) -> Vec<Effect> {
        if let Some(modal) = self.modal.clone() {
            return self.on_modal_key(modal, key);
        }
        let app_cursor = self.parser.screen().application_cursor();
        match self.keymap.handle(key, app_cursor) {
            KeyAction::Send(bytes) => {
                self.scroll_to_live();
                match self.focused {
                    Some(id) => vec![Effect::Send(ClientMsg::Input { window_id: id, bytes })],
                    None => vec![],
                }
            }
            KeyAction::Run(cmd) => self.run(cmd),
            KeyAction::AwaitPrefix | KeyAction::Cancel | KeyAction::Nothing => vec![],
        }
    }

    fn on_modal_key(&mut self, modal: Modal, key: KeyEvent) -> Vec<Effect> {
        match modal {
            Modal::Help => {
                self.modal = None;
                vec![]
            }
            Modal::Confirm { action, .. } => match key.code {
                KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                    self.modal = None;
                    self.perform(action)
                }
                KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                    self.modal = None;
                    vec![]
                }
                _ => vec![],
            },
        }
    }

    fn perform(&mut self, action: PendingAction) -> Vec<Effect> {
        match action {
            PendingAction::Kill(id) => vec![Effect::Send(ClientMsg::Kill { window_id: id })],
            PendingAction::Remove(id) => vec![Effect::Send(ClientMsg::Remove { window_id: id, remove_worktree: false, force: false })],
            PendingAction::StopDaemon => vec![Effect::Send(ClientMsg::Shutdown), Effect::Quit],
        }
    }

    fn run(&mut self, cmd: Command) -> Vec<Effect> {
        match cmd {
            Command::NextWindow => self.focus_relative(1),
            Command::PrevWindow => self.focus_relative(-1),
            Command::FocusIndex(i) => match self.windows.get(i).map(|w| w.id) {
                Some(id) => self.focus(id),
                None => vec![],
            },
            Command::NewWindow => {
                let (cols, rows) = self.term_size;
                vec![Effect::Send(ClientMsg::CreateWindow {
                    spec: WindowSpec {
                        name: None,
                        runtime: Runtime::Shell,
                        cwd: self.default_dir.clone(),
                        worktree_branch: None,
                        model: None,
                        initial_prompt: None,
                    },
                    cols: cols.max(1),
                    rows: rows.max(1),
                })]
            }
            Command::KillWindow => self.confirm_focused("Kill", PendingAction::Kill),
            Command::RemoveWindow => self.confirm_focused("Remove", PendingAction::Remove),
            Command::ToggleSidebar => {
                self.sidebar_visible = !self.sidebar_visible;
                vec![]
            }
            Command::Detach => vec![Effect::Quit],
            Command::StopDaemon => {
                self.modal = Some(Modal::Confirm {
                    message: "Stop the daemon and kill every agent?".into(),
                    action: PendingAction::StopDaemon,
                });
                vec![]
            }
            Command::Help => {
                self.modal = Some(Modal::Help);
                vec![]
            }
        }
    }

    fn confirm_focused(&mut self, verb: &str, make: fn(u32) -> PendingAction) -> Vec<Effect> {
        if let Some((id, name)) = self.focused_window().map(|w| (w.id, w.name.clone())) {
            self.modal = Some(Modal::Confirm { message: format!("{verb} '{name}'?"), action: make(id) });
        }
        vec![]
    }

    pub fn on_paste(&mut self, text: String) -> Vec<Effect> {
        let Some(id) = self.focused else { return vec![] };
        self.scroll_to_live();
        let bracketed = self.parser.screen().bracketed_paste();
        let mut bytes = Vec::with_capacity(text.len() + 12);
        if bracketed {
            bytes.extend_from_slice(b"\x1b[200~");
        }
        bytes.extend_from_slice(text.replace("\r\n", "\r").replace('\n', "\r").as_bytes());
        if bracketed {
            bytes.extend_from_slice(b"\x1b[201~");
        }
        vec![Effect::Send(ClientMsg::Input { window_id: id, bytes })]
    }

    /// Mouse wheel over the main area. Forwarded as an SGR mouse report when the program
    /// enabled mouse tracking; otherwise scrolls the local scrollback view.
    pub fn on_scroll(&mut self, up: bool, column: u16, row: u16, main_inner: Rect) -> Vec<Effect> {
        if self.modal.is_some() || !main_inner.contains((column, row).into()) {
            return vec![];
        }
        if self.parser.screen().mouse_protocol_mode() != vt100::MouseProtocolMode::None {
            let Some(id) = self.focused else { return vec![] };
            let x = column - main_inner.x + 1;
            let y = row - main_inner.y + 1;
            let button = if up { 64 } else { 65 };
            return vec![Effect::Send(ClientMsg::Input { window_id: id, bytes: format!("\x1b[<{button};{x};{y}M").into_bytes() })];
        }
        let target = if up { self.scroll_offset + 3 } else { self.scroll_offset.saturating_sub(3) };
        self.parser.screen_mut().set_scrollback(target);
        self.scroll_offset = self.parser.screen().scrollback();
        vec![]
    }

    fn scroll_to_live(&mut self) {
        if self.scroll_offset != 0 {
            self.parser.screen_mut().set_scrollback(0);
            self.scroll_offset = 0;
        }
    }

    /// Called every 100 ms: advances the spinner, expires toasts, flushes a debounced resize.
    pub fn on_tick(&mut self) -> Vec<Effect> {
        self.spinner_frame = self.spinner_frame.wrapping_add(1);
        if self.toast.as_ref().is_some_and(|(_, at)| at.elapsed() >= TOAST_TTL) {
            self.toast = None;
        }
        if self.pending_resize.is_some_and(|at| at.elapsed() >= RESIZE_DEBOUNCE) {
            self.pending_resize = None;
            if let Some(id) = self.focused {
                let (cols, rows) = self.term_size;
                return vec![Effect::Send(ClientMsg::Resize { window_id: id, cols, rows })];
            }
        }
        vec![]
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p anthrex-tui app`
Expected: `10 passed`.

- [ ] **Step 5: Commit**

```bash
git add crates/tui
git commit -m "feat(tui): add application state with focus, prefix commands and toasts

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 15: Rendering: theme, sidebar, terminal, status bar, modals

**Files:**
- Create: `crates/tui/src/theme.rs`, `crates/tui/src/ui/mod.rs`, `crates/tui/src/ui/sidebar.rs`, `crates/tui/src/ui/terminal.rs`, `crates/tui/src/ui/statusbar.rs`, `crates/tui/src/ui/modal.rs`
- Modify: `crates/tui/src/lib.rs`

**Interfaces:**
- Consumes: `app::{App, Modal}`, `theme`.
- Produces: `tui::ui::{Layout { sidebar, sidebar_inner, main, main_inner, statusbar }, layout(area: Rect, sidebar_visible: bool) -> Layout, draw(frame: &mut Frame, app: &App) -> Layout, SIDEBAR_WIDTH}`, `tui::ui::sidebar::{hit_test(inner: Rect, app: &App, column: u16, row: u16) -> Option<usize>, CARD_HEIGHT, format_elapsed(u64) -> String}`, `tui::theme::{ACCENT, DIM, SPINNER, status_color, status_glyph, border, border_focused, title, muted}`.

- [ ] **Step 1: Write the failing rendering tests**

Create `crates/tui/src/ui/mod.rs` with only:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{App, Modal, PendingAction};
    use crate::keymap::Keymap;
    use proto::{Runtime, Status, WindowInfo};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn win(id: u32, name: &str, runtime: Runtime, status: Status) -> WindowInfo {
        WindowInfo {
            id,
            name: name.into(),
            runtime,
            cwd: "/tmp/repo".into(),
            branch: Some("feat/x".into()),
            status,
            tool: None,
            since_secs: 75,
            last_output_secs: 1,
            has_session: false,
            exit: None,
        }
    }

    fn render(app: &App, width: u16, height: u16) -> (String, Layout) {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        let mut layout = None;
        terminal.draw(|f| layout = Some(draw(f, app))).unwrap();
        (terminal.backend().to_string(), layout.unwrap())
    }

    #[test]
    fn layout_splits_sidebar_main_and_statusbar() {
        let l = layout(ratatui::layout::Rect::new(0, 0, 120, 40), true);
        assert_eq!(l.sidebar.width, SIDEBAR_WIDTH);
        assert_eq!(l.main.x, SIDEBAR_WIDTH);
        assert_eq!(l.main.width, 120 - SIDEBAR_WIDTH);
        assert_eq!(l.statusbar, ratatui::layout::Rect::new(0, 39, 120, 1));
        assert_eq!(l.main_inner, ratatui::layout::Rect::new(SIDEBAR_WIDTH + 1, 1, 120 - SIDEBAR_WIDTH - 2, 37));
        let hidden = layout(ratatui::layout::Rect::new(0, 0, 120, 40), false);
        assert_eq!(hidden.sidebar.width, 0);
        assert_eq!(hidden.main.width, 120);
    }

    #[test]
    fn sidebar_lists_cards_with_glyph_runtime_status_and_elapsed() {
        let mut app = App::new(
            vec![win(1, "api-worker", Runtime::Claude, Status::Working), win(2, "tests", Runtime::Codex, Status::Attention)],
            "/tmp".into(),
            Keymap::default_prefix(),
        );
        let _ = app.set_terminal_size(80, 24);
        let (out, _) = render(&app, 100, 20);
        assert!(out.contains("anthrex"));
        assert!(out.contains("1 api-worker"));
        assert!(out.contains("claude · working · 1m"));
        assert!(out.contains("◆ 2 tests"));
        assert!(out.contains("codex · attention · 1m"));
        assert!(out.contains("2 agents · 1 working · 1 attention"));
        assert!(out.contains("api-worker · claude · /tmp/repo (feat/x)"), "main title\n{out}");
    }

    #[test]
    fn empty_state_and_hidden_sidebar() {
        let mut app = App::new(vec![], "/tmp".into(), Keymap::default_prefix());
        let _ = app.set_terminal_size(80, 24);
        let (out, _) = render(&app, 100, 20);
        assert!(out.contains("no agents yet"));
        assert!(out.contains("No agents"));
        app.sidebar_visible = false;
        let (out, l) = render(&app, 100, 20);
        assert!(!out.contains("no agents yet"));
        assert_eq!(l.sidebar.width, 0);
    }

    #[test]
    fn statusbar_shows_prefix_state_and_toast() {
        let mut app = App::new(vec![win(1, "a", Runtime::Shell, Status::Idle)], "/tmp".into(), Keymap::default_prefix());
        let _ = app.set_terminal_size(80, 24);
        let (out, _) = render(&app, 100, 20);
        assert!(out.contains("C-b ?"));
        assert!(!out.contains("PREFIX"));
        app.on_key(crossterm::event::KeyEvent::new(crossterm::event::KeyCode::Char('b'), crossterm::event::KeyModifiers::CONTROL));
        app.toast("tests needs attention");
        let (out, _) = render(&app, 100, 20);
        assert!(out.contains("PREFIX"));
        assert!(out.contains("tests needs attention"));
    }

    #[test]
    fn modals_render_on_top() {
        let mut app = App::new(vec![win(1, "a", Runtime::Shell, Status::Idle)], "/tmp".into(), Keymap::default_prefix());
        let _ = app.set_terminal_size(80, 24);
        app.modal = Some(Modal::Confirm { message: "Kill 'a'?".into(), action: PendingAction::Kill(1) });
        let (out, _) = render(&app, 100, 20);
        assert!(out.contains("Kill 'a'?"));
        assert!(out.contains("y / Enter = yes"));
        app.modal = Some(Modal::Help);
        let (out, _) = render(&app, 100, 24);
        assert!(out.contains("send a literal C-b"));
    }

    #[test]
    fn sidebar_hit_test_maps_rows_to_cards() {
        let mut app = App::new(
            vec![win(1, "a", Runtime::Shell, Status::Idle), win(2, "b", Runtime::Shell, Status::Idle)],
            "/tmp".into(),
            Keymap::default_prefix(),
        );
        let _ = app.set_terminal_size(80, 24);
        let (_, l) = render(&app, 100, 20);
        assert_eq!(sidebar::hit_test(l.sidebar_inner, &app, 3, l.sidebar_inner.y), Some(0));
        assert_eq!(sidebar::hit_test(l.sidebar_inner, &app, 3, l.sidebar_inner.y + sidebar::CARD_HEIGHT), Some(1));
        assert_eq!(sidebar::hit_test(l.sidebar_inner, &app, 3, l.sidebar_inner.y + 3 * sidebar::CARD_HEIGHT), None);
        assert_eq!(sidebar::hit_test(l.sidebar_inner, &app, 60, l.sidebar_inner.y), None);
        assert_eq!(sidebar::format_elapsed(59), "59s");
        assert_eq!(sidebar::format_elapsed(3600), "1h");
    }
}
```

Add to `crates/tui/src/lib.rs`:

```rust
pub mod theme;
pub mod ui;
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p anthrex-tui ui`
Expected: compile error, `cannot find function layout` (and the missing submodules).

- [ ] **Step 3: Implement the theme**

Create `crates/tui/src/theme.rs`:

```rust
//! Colours and glyphs. Spec section 6.5: inherit the terminal background, one accent, unicode-only glyphs.

use proto::Status;
use ratatui::style::{Color, Modifier, Style};

pub const ACCENT: Color = Color::Rgb(0x89, 0xb4, 0xfa);
pub const DIM: Color = Color::DarkGray;
pub const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

pub fn status_color(status: Status) -> Color {
    match status {
        Status::Starting => Color::Gray,
        Status::Working => Color::Yellow,
        Status::Idle => Color::Gray,
        Status::Attention => Color::Rgb(0xfa, 0xb3, 0x87),
        Status::Done => Color::Green,
        Status::Exited => Color::DarkGray,
    }
}

pub fn status_glyph(status: Status, spinner_frame: usize) -> &'static str {
    match status {
        Status::Starting => "◌",
        Status::Working => SPINNER[spinner_frame % SPINNER.len()],
        Status::Idle => "○",
        Status::Attention => "◆",
        Status::Done => "✓",
        Status::Exited => "✕",
    }
}

pub fn border() -> Style {
    Style::default().fg(DIM)
}

pub fn border_focused() -> Style {
    Style::default().fg(ACCENT)
}

pub fn title() -> Style {
    Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
}

pub fn muted() -> Style {
    Style::default().fg(DIM)
}
```

- [ ] **Step 4: Implement the layout and draw entry point**

Put above the test module in `crates/tui/src/ui/mod.rs`:

```rust
//! Screen layout and the top-level draw. Spec section 6.1.

pub mod modal;
pub mod sidebar;
pub mod statusbar;
pub mod terminal;

use crate::app::App;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout as RLayout, Rect};

pub const SIDEBAR_WIDTH: u16 = 30;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Layout {
    pub sidebar: Rect,
    pub sidebar_inner: Rect,
    pub main: Rect,
    pub main_inner: Rect,
    pub statusbar: Rect,
}

fn inset(r: Rect) -> Rect {
    Rect {
        x: r.x.saturating_add(1),
        y: r.y.saturating_add(1),
        width: r.width.saturating_sub(2),
        height: r.height.saturating_sub(2),
    }
}

pub fn layout(area: Rect, sidebar_visible: bool) -> Layout {
    let [body, statusbar] = RLayout::vertical([Constraint::Min(3), Constraint::Length(1)]).areas(area);
    let (sidebar, main) = if sidebar_visible {
        let [s, m] = RLayout::horizontal([Constraint::Length(SIDEBAR_WIDTH), Constraint::Min(10)]).areas(body);
        (s, m)
    } else {
        (Rect::new(body.x, body.y, 0, body.height), body)
    };
    Layout { sidebar, sidebar_inner: inset(sidebar), main, main_inner: inset(main), statusbar }
}

/// Draws everything and returns the layout so the caller can size the PTY and hit-test the mouse.
pub fn draw(frame: &mut Frame, app: &App) -> Layout {
    let l = layout(frame.area(), app.sidebar_visible);
    if app.sidebar_visible {
        sidebar::render(frame, app, l.sidebar);
    }
    terminal::render(frame, app, l.main);
    statusbar::render(frame, app, l.statusbar);
    if let Some(m) = &app.modal {
        modal::render(frame, m, frame.area());
    }
    l
}
```

- [ ] **Step 5: Implement the sidebar**

Create `crates/tui/src/ui/sidebar.rs`:

```rust
use crate::app::App;
use crate::theme;
use proto::Status;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph};

/// Rows per agent card: name line plus detail line.
pub const CARD_HEIGHT: u16 = 2;

pub fn format_elapsed(secs: u64) -> String {
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else {
        format!("{}h", secs / 3600)
    }
}

fn summary(app: &App) -> String {
    let n = app.windows.len();
    let working = app.windows.iter().filter(|w| w.status == Status::Working).count();
    let attention = app.windows.iter().filter(|w| w.status == Status::Attention).count();
    let mut parts = vec![format!("{n} agent{}", if n == 1 { "" } else { "s" })];
    if working > 0 {
        parts.push(format!("{working} working"));
    }
    if attention > 0 {
        parts.push(format!("{attention} attention"));
    }
    parts.join(" · ")
}

pub fn render(frame: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme::border())
        .title(Line::from(Span::styled(" anthrex ", theme::title())));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.height < 2 {
        return;
    }

    let mut lines: Vec<Line> = Vec::new();
    for (i, w) in app.windows.iter().enumerate() {
        let focused = app.focused == Some(w.id);
        let bar = if focused { Span::styled("▎", Style::default().fg(theme::ACCENT)) } else { Span::raw(" ") };
        let glyph = Span::styled(theme::status_glyph(w.status, app.spinner_frame), Style::default().fg(theme::status_color(w.status)));
        let name_style = if focused { Style::default().add_modifier(Modifier::BOLD) } else { Style::default() };
        lines.push(Line::from(vec![bar.clone(), glyph, Span::raw(" "), Span::styled(format!("{} {}", i + 1, w.name), name_style)]));
        let detail = format!("  {} · {} · {}", w.runtime.label(), w.status.label(), format_elapsed(app.elapsed_secs(w)));
        lines.push(Line::from(vec![bar, Span::styled(detail, theme::muted())]));
    }
    if app.windows.is_empty() {
        lines.push(Line::from(Span::styled(" no agents yet", theme::muted())));
        lines.push(Line::from(Span::styled(" C-b c opens a shell", theme::muted())));
    }

    let list_area = Rect { height: inner.height.saturating_sub(2), ..inner };
    frame.render_widget(Paragraph::new(lines), list_area);
    let footer = Rect { y: inner.y + inner.height - 1, height: 1, ..inner };
    frame.render_widget(Paragraph::new(Line::from(Span::styled(summary(app), theme::muted()))), footer);
}

/// Which card (index into `app.windows`) is under a screen position inside the sidebar.
pub fn hit_test(inner: Rect, app: &App, column: u16, row: u16) -> Option<usize> {
    if column < inner.x || column >= inner.x + inner.width || row < inner.y || row >= inner.y + inner.height {
        return None;
    }
    let index = ((row - inner.y) / CARD_HEIGHT) as usize;
    (index < app.windows.len()).then_some(index)
}
```

- [ ] **Step 6: Implement the terminal pane**

Create `crates/tui/src/ui/terminal.rs`:

```rust
use crate::app::App;
use crate::theme;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph};
use std::path::Path;
use tui_term::widget::{Cursor, PseudoTerminal};

pub fn shorten_home(path: &Path) -> String {
    if let Some(home) = dirs::home_dir() {
        if let Ok(rest) = path.strip_prefix(&home) {
            return format!("~/{}", rest.display());
        }
    }
    path.display().to_string()
}

pub fn render(frame: &mut Frame, app: &App, area: Rect) {
    let title = match app.focused_window() {
        Some(w) => {
            let branch = w.branch.as_ref().map(|b| format!(" ({b})")).unwrap_or_default();
            format!(" {} · {} · {}{branch} ", w.name, w.runtime.label(), shorten_home(&w.cwd))
        }
        None => " no window ".to_string(),
    };
    let border = if app.modal.is_none() { theme::border_focused() } else { theme::border() };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(border)
        .title(Line::from(Span::styled(title, theme::title())));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if app.focused.is_none() {
        let hint = vec![
            Line::raw(""),
            Line::styled("  No agents. Press C-b c to open a shell here, or run `anthrex new`.", theme::muted()),
        ];
        frame.render_widget(Paragraph::new(hint), inner);
        return;
    }

    let screen = app.parser.screen();
    // We place the hardware cursor ourselves, so the widget's drawn cursor stays hidden.
    let widget = PseudoTerminal::new(screen).cursor(Cursor::default().visibility(false));
    frame.render_widget(widget, inner);

    if !screen.hide_cursor() && app.scroll_offset == 0 && app.modal.is_none() {
        let (row, col) = screen.cursor_position();
        if row < inner.height && col < inner.width {
            frame.set_cursor_position((inner.x + col, inner.y + row));
        }
    }
}
```

- [ ] **Step 7: Implement the status bar and modals**

Create `crates/tui/src/ui/statusbar.rs`:

```rust
use crate::app::App;
use crate::theme;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

const HINTS: [(&str, &str); 5] = [("C-b ?", "help"), ("C-b c", "new shell"), ("C-b j/k", "switch"), ("C-b x", "kill"), ("C-b d", "detach")];

pub fn render(frame: &mut Frame, app: &App, area: Rect) {
    let mut spans = Vec::new();
    if app.keymap.pending() {
        spans.push(Span::styled(" PREFIX ", Style::default().fg(Color::Black).bg(theme::ACCENT).add_modifier(Modifier::BOLD)));
        spans.push(Span::raw(" "));
    } else if !app.connected {
        spans.push(Span::styled(" DISCONNECTED ", Style::default().fg(Color::Black).bg(Color::Red).add_modifier(Modifier::BOLD)));
        spans.push(Span::raw(" "));
    } else {
        spans.push(Span::raw(" "));
    }
    for (key, what) in HINTS {
        spans.push(Span::styled(key, Style::default().fg(theme::ACCENT)));
        spans.push(Span::styled(format!(" {what}  "), theme::muted()));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);

    if let Some(text) = app.toast_text() {
        let width = (text.chars().count() as u16 + 1).min(area.width);
        let right = Rect { x: area.x + area.width - width, width, ..area };
        let toast = Span::styled(text.to_string(), Style::default().fg(theme::ACCENT).add_modifier(Modifier::BOLD));
        frame.render_widget(Paragraph::new(Line::from(toast)), right);
    }
}
```

Create `crates/tui/src/ui/modal.rs`:

```rust
use crate::app::Modal;
use crate::theme;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};

pub const HELP: &[(&str, &str)] = &[
    ("C-b j / k", "next / previous agent"),
    ("C-b 1-9", "focus agent by number"),
    ("C-b c", "new shell window"),
    ("C-b x", "kill agent"),
    ("C-b X", "remove agent"),
    ("C-b s", "toggle sidebar"),
    ("C-b d", "detach (agents keep running)"),
    ("C-b Q", "stop daemon and all agents"),
    ("C-b C-b", "send a literal C-b"),
    ("any key", "close this help"),
];

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    Rect { x: area.x + (area.width - width) / 2, y: area.y + (area.height - height) / 2, width, height }
}

pub fn render(frame: &mut Frame, modal: &Modal, area: Rect) {
    let (title, body): (&str, Vec<Line>) = match modal {
        Modal::Confirm { message, .. } => (
            " confirm ",
            vec![Line::raw(message.clone()), Line::raw(""), Line::styled("y / Enter = yes    n / Esc = no", theme::muted())],
        ),
        Modal::Help => (
            " keys ",
            HELP.iter()
                .map(|(key, what)| Line::from(vec![Span::styled(format!("{key:<11}"), Style::default().fg(theme::ACCENT)), Span::raw(*what)]))
                .collect(),
        ),
    };
    let width = body.iter().map(Line::width).max().unwrap_or(0).max(30) as u16 + 4;
    let height = body.len() as u16 + 2;
    let rect = centered(area, width, height);
    frame.render_widget(Clear, rect);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme::border_focused())
        .title(Line::from(Span::styled(title, theme::title())));
    frame.render_widget(Paragraph::new(body).block(block), rect);
}
```

- [ ] **Step 8: Run the tests to verify they pass**

Run: `cargo test -p anthrex-tui`
Expected: all tests pass, including the 6 new `ui` tests. If `sidebar_lists_cards...` fails on the elapsed text, it is because `App::elapsed_secs` adds wall-clock seconds since the list arrived; the test runs within the same second so `75s` rounds to `1m` as asserted.

- [ ] **Step 9: Commit**

```bash
git add crates/tui
git commit -m "feat(tui): render sidebar, focused terminal, status bar and modals

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 16: Event loop, `anthrex` attach, README and smoke test

**Files:**
- Modify: `crates/tui/src/lib.rs`, `crates/cli/src/main.rs`
- Create: `README.md`

**Interfaces:**
- Produces: `tui::{TuiOptions { socket_path, default_dir, focus: Option<String> }, run(TuiOptions) -> anyhow::Result<()>}`; CLI default command and `anthrex attach [target]`.

- [ ] **Step 1: Write the event loop**

Replace `crates/tui/src/lib.rs` with:

```rust
//! The anthrex terminal client: connects to the daemon and runs the ratatui event loop.

pub mod app;
pub mod connection;
pub mod keymap;
pub mod theme;
pub mod ui;

use app::{App, Effect};
use connection::Connection;
use crossterm::event::{DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture, Event, EventStream, MouseButton, MouseEventKind};
use futures::StreamExt;
use keymap::Keymap;
use ratatui::DefaultTerminal;
use std::path::PathBuf;
use std::time::Duration;

pub struct TuiOptions {
    pub socket_path: PathBuf,
    pub default_dir: PathBuf,
    /// Window id or name to focus on start.
    pub focus: Option<String>,
}

pub async fn run(opts: TuiOptions) -> anyhow::Result<()> {
    let mut conn = Connection::connect(&opts.socket_path).await?;
    let mut app = App::new(conn.windows.clone(), opts.default_dir.clone(), Keymap::default_prefix());
    if let Some(target) = &opts.focus {
        if let Some(w) = app.windows.iter().find(|w| w.name == *target || w.id.to_string() == *target) {
            app.request_focus(w.id);
        }
    }

    let mut terminal = ratatui::init();
    let _ = crossterm::execute!(std::io::stdout(), EnableMouseCapture, EnableBracketedPaste);
    let result = event_loop(&mut terminal, &mut conn, &mut app).await;
    let _ = crossterm::execute!(std::io::stdout(), DisableBracketedPaste, DisableMouseCapture);
    ratatui::restore();
    result
}

async fn apply(effects: Vec<Effect>, conn: &Connection) -> bool {
    for effect in effects {
        match effect {
            Effect::Send(msg) => {
                conn.send(msg).await;
            }
            Effect::Quit => return true,
        }
    }
    false
}

async fn draw(terminal: &mut DefaultTerminal, app: &mut App, conn: &Connection) -> anyhow::Result<ui::Layout> {
    let mut layout = None;
    terminal.draw(|frame| layout = Some(ui::draw(frame, app)))?;
    let layout = layout.expect("draw closure always runs");
    let effects = app.set_terminal_size(layout.main_inner.width, layout.main_inner.height);
    apply(effects, conn).await;
    Ok(layout)
}

async fn event_loop(terminal: &mut DefaultTerminal, conn: &mut Connection, app: &mut App) -> anyhow::Result<()> {
    let mut events = EventStream::new();
    let mut tick = tokio::time::interval(Duration::from_millis(100));
    loop {
        let layout = draw(terminal, app, conn).await?;
        let effects = tokio::select! {
            Some(event) = events.next() => match event? {
                Event::Key(key) => app.on_key(key),
                Event::Paste(text) => app.on_paste(text),
                Event::Mouse(mouse) => match mouse.kind {
                    MouseEventKind::Down(MouseButton::Left) if app.sidebar_visible && app.modal.is_none() => {
                        match ui::sidebar::hit_test(layout.sidebar_inner, app, mouse.column, mouse.row) {
                            Some(index) => { let id = app.windows[index].id; app.focus(id) }
                            None => vec![],
                        }
                    }
                    MouseEventKind::ScrollUp => app.on_scroll(true, mouse.column, mouse.row, layout.main_inner),
                    MouseEventKind::ScrollDown => app.on_scroll(false, mouse.column, mouse.row, layout.main_inner),
                    _ => vec![],
                },
                _ => vec![],
            },
            msg = conn.recv(), if app.connected => match msg {
                Some(msg) => app.on_daemon(msg),
                None => {
                    app.connected = false;
                    app.toast("connection to daemon lost; C-b d to exit");
                    vec![]
                }
            },
            _ = tick.tick() => app.on_tick(),
        };
        if apply(effects, conn).await {
            return Ok(());
        }
    }
}
```

- [ ] **Step 2: Wire attach into the CLI**

In `crates/cli/src/main.rs`, add the variant to `Command`:

```rust
    /// Attach to the daemon (the default when no command is given)
    Attach {
        /// Window id or name to focus
        target: Option<String>,
    },
```

Replace the `None => { println!(...) }` arm and add the `Attach` arm in `main`:

```rust
        None => attach(socket, dir, None).await,
        Some(Command::Attach { target }) => attach(socket, dir, target).await,
```

Add the function:

```rust
async fn attach(socket: PathBuf, dir: PathBuf, target: Option<String>) -> anyhow::Result<()> {
    spawn::ensure_daemon(&socket).await?;
    tui::run(tui::TuiOptions { socket_path: socket, default_dir: dir, focus: target }).await
}
```

- [ ] **Step 3: Build and run the full test suite**

Run: `cargo build && cargo test`
Expected: everything builds; all tests across the four crates pass.

- [ ] **Step 4: Smoke test by hand**

In a real terminal (not inside another multiplexer for the first run):

```bash
export ANTHREX_SOCKET=/tmp/anthrex-test.sock ANTHREX_DATA_DIR=/tmp/anthrex-test-data
cargo run -q -p anthrex
```

Check each of these, then tick the step:

1. The sidebar shows "no agents yet"; the main pane shows the "No agents" hint; the status bar shows key hints.
2. `C-b c` opens a shell window; the card appears and gets focus; typing `ls` and Enter shows output; the card's status goes working then idle after about 3 s.
3. `C-b c` again, then `C-b j` / `C-b k` / `C-b 1` switch windows; the main title changes; each window keeps its own contents.
4. Run `printf '\a'` in the focused shell, then `C-b j`: the previous window shows `◆` and a toast appears when its bell fires while unfocused (run `sleep 2; printf '\a'` and switch away first).
5. Mouse: clicking a card focuses it; the wheel scrolls a shell's history after `seq 1 200`.
6. `C-b s` hides and shows the sidebar and the shell reflows to the new width.
7. `C-b x`, then `y`: the card turns `✕ exited` within about 3 s. `C-b X`, `y` removes it.
8. `C-b ?` shows the help overlay; any key closes it.
9. `C-b d` detaches. `cargo run -q -p anthrex -- ls` still lists the windows. `cargo run -q -p anthrex` reattaches and shows the same screens.
10. `cargo run -q -p anthrex -- new --runtime claude --name cc` then reattach: Claude Code starts inside the window and is usable (answer its trust prompt in the window if it asks).
11. `C-b Q`, `y` stops the daemon; the client exits; `daemon status` reports not running.

- [ ] **Step 5: Write the README**

Create `README.md`:

````markdown
# anthrex

A terminal multiplexer for coding agents. Each window is a real `claude`, `codex` or shell session in its own PTY, owned by a background daemon so it survives closing the UI. A sidebar shows every agent with live status; the main pane shows the focused one at full size.

## Build

```bash
cargo build --release
# binary: target/release/anthrex
```

Requires Rust 1.92 or newer, macOS or Linux.

## Use

```bash
anthrex                                   # attach (starts the daemon if needed)
anthrex new --runtime claude --name api   # create an agent window from any shell
anthrex new --runtime codex --dir ~/repo
anthrex ls
anthrex kill api
anthrex rm api
anthrex daemon stop                       # stops the daemon and every agent
```

Inside the UI, the prefix key is `Ctrl-b`:

| Keys | Action |
|------|--------|
| `C-b j` / `C-b k` | next / previous window |
| `C-b 1`..`9` | focus window by number |
| `C-b c` | new shell window |
| `C-b x` / `C-b X` | kill / remove the focused window |
| `C-b s` | toggle the sidebar |
| `C-b d` | detach (agents keep running) |
| `C-b Q` | stop the daemon and all agents |
| `C-b C-b` | send a literal Ctrl-b |
| `C-b ?` | help |

Mouse: click a sidebar card to focus it; the wheel scrolls (forwarded to programs that use the mouse). Because mouse capture is on, use your terminal's shift-drag to select text.

## Files

| What | macOS | Linux |
|------|-------|-------|
| socket | `$TMPDIR/anthrex-<uid>/daemon.sock` | `$XDG_RUNTIME_DIR/anthrex/daemon.sock` |
| log, pid | `~/Library/Application Support/anthrex/` | `~/.local/share/anthrex/` |

`ANTHREX_SOCKET` and `ANTHREX_DATA_DIR` override these. `ANTHREX_LOG=debug` raises the daemon log level.

## Status

Milestone 1 of 5 (see `docs/superpowers/specs/`). Coming next: hook-driven agent status, the new-agent dialog with git worktrees, persistence across daemon restarts, CI.
````

- [ ] **Step 6: Commit**

```bash
git add crates/tui crates/cli README.md
git commit -m "feat: add the anthrex client event loop, attach command and README

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

## Plan self-review notes

- **Spec coverage for this milestone:** sections 2.1 to 2.3 (Tasks 1, 4, 10), 3.1 to 3.3 (Tasks 5, 7, 8), 3.4 "all" and Shell rows (Task 6), 3.7 (Task 10), 4 (Tasks 2, 3, 9), 5 except `hook` and `restart` (Tasks 10, 11), 6.1, 6.2 except rename and restart, 6.4 toasts without the outer bell, 6.5 theme (Tasks 12 to 16). Left for later plans as the spec's section 11 orders them: launcher hooks and the full status table (plan 2), dialogs and worktrees (plan 3), persistence, config file, rename, restart and bell settings (plan 4), CI (plan 5).
- **Names used across tasks:** `WindowManager::{create, list, watch, handle_event, tick, write_input, resize, attach, snapshot, focus, kill, remove, rename, shutdown}`, `Window::{spawn, write_input, resize, size, attach, snapshot, screen_text, signal, pid}`, `App::{set_terminal_size, on_daemon, on_key, on_paste, on_scroll, on_tick, focus, request_focus}`, `Keymap::{new, default_prefix, pending, handle}`, `Connection::{connect, send, recv}`, `ui::{layout, draw, Layout}`, `ui::sidebar::{hit_test, CARD_HEIGHT, format_elapsed}` are spelled identically in every task that mentions them.
