//! A small blocking-style client for one-shot CLI commands.

use proto::{ClientKind, ClientMsg, DaemonMsg, PROTO_VERSION, WindowInfo, read_frame, write_frame};
use std::path::Path;
use std::time::Duration;
use tokio::net::UnixStream;
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
const CREATE_REPLY_ALLOWANCE: Duration = Duration::from_secs(2);

/// What a window launch can legitimately wait on the daemon's startup before it starts:
/// `WindowManager::create` and `WindowManager::restart` wait on
/// `daemon::launch::LaunchGate`, which the Codex version probe holds for at most
/// `CODEX_PROBE_TIMEOUT` after the daemon binds its socket.
///
/// This term is new, and it is new *here* rather than being new in absolute terms: the
/// same five seconds used to be spent inside `CliClient::connect`'s handshake instead,
/// where it had no budget of its own at all and simply raced `proto::HANDSHAKE_TIMEOUT`
/// — the defect the gate exists to remove. Moving the wait to where it belongs means the
/// budgets that cover that wait have to say so.
///
/// It applies to every reply budget below that wraps a request the daemon answers by
/// launching a process — `CreateWindow` and `Restart`, and no others — because
/// `spawn::ensure_daemon` returns as soon as the socket *file* exists, which
/// `lifecycle::run` creates before the probe starts. A CLI invocation that starts the
/// daemon itself is therefore the *likely* case for paying this, not an exotic one.
const LAUNCH_GATE_WAIT: Duration = daemon::lifecycle::CODEX_PROBE_TIMEOUT;

///     LAUNCH_GATE_WAIT (5) + DETECT_TIMEOUT (5) + CREATE_REPLY_ALLOWANCE (2) = 12 s
pub const CREATE_WINDOW_REPLY_TIMEOUT: Duration = LAUNCH_GATE_WAIT
    .saturating_add(daemon::project::DETECT_TIMEOUT)
    .saturating_add(CREATE_REPLY_ALLOWANCE);

/// Room left over the longer of the two worst cases below, so the CLI's own deadline
/// never fires before the daemon's (decision 38 of the M5 worktrees brief).
const WORKTREE_TIMEOUT_MARGIN: Duration = Duration::from_secs(12);

/// What `rm --worktree` can cost the daemon: `remove_with_worktree` opens one
/// `OPERATION_TIMEOUT` deadline and shares it between the dirty check and
/// `worktree::remove`, and spends `KILL_GRACE` on its own deadline waiting for the agent
/// to exit in between.
///
///     OPERATION_TIMEOUT (30) + KILL_GRACE (3) = 33 s
const REMOVAL_WORST_CASE: Duration =
    daemon::worktree::OPERATION_TIMEOUT.saturating_add(daemon::manager::KILL_GRACE);

/// What `new --worktree` can cost the daemon — the term-by-term derivation, because two
/// earlier reviews disagreed about it (35 s and 50 s) and the constant sat between them:
///
///     LAUNCH_GATE_WAIT (5) + DETECT_TIMEOUT (5) + OPERATION_TIMEOUT (30)
///       + CLEANUP_TIMEOUT (10) = 50 s
///
/// - **`LAUNCH_GATE_WAIT` (5 s)** — `WindowManager::create` waits on the launch gate
///   before phase A, so this sits ahead of everything below rather than overlapping any
///   of it. See [`LAUNCH_GATE_WAIT`] for why it is charged here now and was not before.
///
/// - **`DETECT_TIMEOUT` (5 s)** — `server::requests::create` awaits
///   `project::resolve_roots(spec.cwd)` before the manager is called at all.
/// - **`OPERATION_TIMEOUT` (30 s)** — `spawn_window` opens the deadline as
///   `Instant::now() + OPERATION_TIMEOUT` and hands it to `worktree::create`, which
///   shares it across every git command it runs. This term is 30 s and **not** 35: the
///   root detection design decision 5 keeps outside the operation deadline is *inside*
///   this wall-clock window, because the deadline is an `Instant` fixed before it runs.
///   Being outside the deadline means it is not cut short by it, not that it is served
///   after it — and 5 s cannot push a 30 s window past 30 s. (Since the whole-branch
///   review's finding 4 that call is gone entirely; the arithmetic held either way,
///   which is why counting it twice was the error.)
/// - **`CLEANUP_TIMEOUT` (10 s)** — a `git worktree add` that fails or times out runs
///   `discard_new` on a *fresh* deadline before returning, and so does a `Window::spawn`
///   that fails once the worktree exists. The two are disjoint, so this counts once, but
///   it counts: it is the term both earlier derivations dropped.
///
/// The sum is a supremum rather than an attainable time — a detection that actually
/// reaches `DETECT_TIMEOUT` falls back and the create is then refused without touching
/// git — which is exactly why the budget must *exceed* it rather than match it.
const CREATE_WORST_CASE: Duration = LAUNCH_GATE_WAIT
    .saturating_add(daemon::project::DETECT_TIMEOUT)
    .saturating_add(daemon::worktree::OPERATION_TIMEOUT)
    .saturating_add(daemon::worktree::CLEANUP_TIMEOUT);

/// `Duration::max` is not a `const fn`; this is.
const fn longer(a: Duration, b: Duration) -> Duration {
    if a.as_nanos() >= b.as_nanos() { a } else { b }
}

/// The reply budget for any request that can make the daemon run git: `new --worktree`
/// and `rm --worktree`. It must exceed the daemon's own worst case for *either*
/// operation, or the CLI reports a timeout for a request the daemon is still working on —
/// and a `new --worktree` that times out client-side loses the one message that says
/// whether a checkout was left on disk (design decision 16's two suffixes).
///
/// Both call sites take the same budget, so it is the larger of the two paths plus
/// [`WORKTREE_TIMEOUT_MARGIN`]: max(50, 33) + 12 = 62 s. It was 45 s, derived from the
/// removal path alone and applied to both, which left the create path with no margin at
/// all — 45 s against a then-45 s supremum.
pub const WORKTREE_REQUEST_TIMEOUT: Duration =
    longer(CREATE_WORST_CASE, REMOVAL_WORST_CASE).saturating_add(WORKTREE_TIMEOUT_MARGIN);

/// What `restart` can cost the daemon (`manager::restart`, task M6.7/M6.8's brief for the
/// exact derivation): phase A and D are lock-only and cost nothing measurable; phase B, for
/// a live window, kills it and polls `child_alive` up to
/// `daemon::manager::KILL_GRACE` (3 s) + 2 s of its own margin — 5 s, decision 18's own
/// number; phase C spawns a fresh PTY with no git involved, the same cheap `Window::spawn`
/// `create`'s phase B pays, never separately budgeted elsewhere in this file. 15 s is a
/// literal from the milestone brief, not derived from `RESTART_WORST_CASE` (see this
/// module's tests), but it must still clear it with real margin and never coincide with a
/// constant the restart path itself is built from (`docs/timing-budgets.md`'s standing
/// rule 1) — asserted below.
/// `WindowManager::restart` also waits on the launch gate, ahead of its phase A, so
/// [`LAUNCH_GATE_WAIT`] is a term here too — the test below asserts 15 s still clears the
/// result with real margin rather than assuming it.
pub const RESTART_REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
#[cfg(test)]
const RESTART_WORST_CASE: Duration = LAUNCH_GATE_WAIT
    .saturating_add(daemon::manager::KILL_GRACE)
    .saturating_add(Duration::from_secs(2));

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
        write_frame(
            &mut wr,
            &ClientMsg::Hello {
                proto_version: PROTO_VERSION,
                client: ClientKind::Cli,
            },
        )
        .await?;
        // Decision 29: a daemon that accepted the connection but never answers must not
        // hang the client forever either.
        let welcome = tokio::time::timeout(
            proto::HANDSHAKE_TIMEOUT,
            read_frame::<_, DaemonMsg>(&mut rd),
        )
        .await
        .map_err(|_| {
            anyhow::anyhow!(
                "timed out waiting for the daemon's handshake at {}",
                socket.display()
            )
        })?;
        match welcome? {
            Some(DaemonMsg::Welcome {
                daemon_version,
                windows,
            }) => Ok(Self {
                rd,
                wr,
                windows,
                daemon_version,
            }),
            Some(DaemonMsg::Error { message, .. }) => anyhow::bail!(message),
            Some(other) => anyhow::bail!("unexpected handshake reply: {other:?}"),
            None => anyhow::bail!("the daemon closed the connection during the handshake"),
        }
    }

    /// Writes one frame without waiting for a reply.
    pub async fn send(&mut self, msg: ClientMsg) -> anyhow::Result<()> {
        write_frame(&mut self.wr, &msg).await?;
        Ok(())
    }

    /// Sends one request and returns the first reply that is not a `WindowsChanged` or
    /// `Git` broadcast.
    pub async fn request(&mut self, msg: ClientMsg) -> anyhow::Result<DaemonMsg> {
        self.request_with_timeout(msg, REQUEST_TIMEOUT).await
    }

    /// Sends one request with an explicit reply deadline.
    pub async fn request_with_timeout(
        &mut self,
        msg: ClientMsg,
        reply_timeout: Duration,
    ) -> anyhow::Result<DaemonMsg> {
        self.send(msg).await?;
        tokio::time::timeout(reply_timeout, async {
            loop {
                match read_frame::<_, DaemonMsg>(&mut self.rd).await? {
                    // Both are broadcasts a one-shot request/reply client has no use
                    // for, and either can land between the request and its reply: a
                    // fresh client's git snapshot arrives right after `Welcome`, and a
                    // window's own creation can trigger both at once.
                    Some(DaemonMsg::WindowsChanged { .. }) | Some(DaemonMsg::Git { .. }) => {
                        continue;
                    }
                    Some(reply) => return Ok(reply),
                    None => anyhow::bail!("the daemon closed the connection"),
                }
            }
        })
        .await
        .map_err(|_| anyhow::anyhow!("timed out waiting for the daemon"))?
    }

    /// Reads and discards frames until the daemon closes the connection, bounded by a 5 s timeout.
    pub async fn wait_close(&mut self) {
        let _ = tokio::time::timeout(Duration::from_secs(5), async {
            while let Ok(Some(_)) = read_frame::<_, DaemonMsg>(&mut self.rd).await {}
        })
        .await;
    }
}

/// `target` is a window id or a window name. Ids win when both match.
pub fn resolve_target(windows: &[WindowInfo], target: &str) -> anyhow::Result<u32> {
    if let Ok(id) = target.parse::<u32>()
        && windows.iter().any(|w| w.id == id)
    {
        return Ok(id);
    }
    if let Some(w) = windows.iter().find(|w| w.name == target) {
        return Ok(w.id);
    }
    anyhow::bail!("no window with id or name '{target}'")
}

pub fn format_table(windows: &[WindowInfo]) -> String {
    let name_w = windows
        .iter()
        .map(|w| w.name.len())
        .max()
        .unwrap_or(4)
        .max(4);
    let branch_w = windows
        .iter()
        .map(|w| w.branch.as_deref().unwrap_or("-").len())
        .max()
        .unwrap_or(6)
        .max(6);
    let mut out = format!(
        "{:<4} {:<name_w$} {:<7} {:<10} {:<branch_w$} DIR\n",
        "ID", "NAME", "RUNTIME", "STATUS", "BRANCH"
    );
    for w in windows {
        out.push_str(&format!(
            "{:<4} {:<name_w$} {:<7} {:<10} {:<branch_w$} {}\n",
            w.id,
            w.name,
            w.runtime.label(),
            w.status.label(),
            w.branch.as_deref().unwrap_or("-"),
            w.cwd.display()
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Decision 29: a daemon that accepted the connection but never answers the handshake
    /// must not hang the CLI client forever either, same as `tui::connection::Connection`.
    #[tokio::test]
    async fn connect_times_out_on_a_silent_server() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("silent.sock");
        let listener = tokio::net::UnixListener::bind(&socket).unwrap();
        let _silent_server = tokio::spawn(async move {
            let (_stream, _) = listener.accept().await.unwrap();
            tokio::time::sleep(Duration::from_secs(30)).await;
        });

        let started = std::time::Instant::now();
        let result = tokio::time::timeout(Duration::from_secs(7), CliClient::connect(&socket))
            .await
            .expect("connect did not return within 7 s");
        let err = match result {
            Ok(_) => panic!("expected a timeout error, got a connection"),
            Err(e) => e,
        };
        assert!(
            err.to_string().contains("timed out"),
            "unexpected error: {err}"
        );
        assert!(
            started.elapsed() < Duration::from_secs(7),
            "{:?}",
            started.elapsed()
        );
    }

    /// M4.5.6 review finding: `request_with_timeout` skips `WindowsChanged` and `Git`
    /// broadcasts, but nothing pinned that deterministically — the daemon integration
    /// tests only caught it when a real broadcast happened to race a real reply. This
    /// drives `CliClient` against a hand-scripted fake daemon (a real `UnixListener`,
    /// no real daemon behind it) that writes both broadcast kinds *ahead of* the
    /// genuine reply on purpose, and asserts the reply still comes back.
    #[tokio::test]
    async fn request_skips_broadcasts_ahead_of_the_genuine_reply() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("d.sock");
        let listener = tokio::net::UnixListener::bind(&socket).unwrap();

        let fake_daemon = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let (mut rd, mut wr) = stream.into_split();

            let hello = read_frame::<_, ClientMsg>(&mut rd).await.unwrap().unwrap();
            assert!(matches!(hello, ClientMsg::Hello { .. }));
            write_frame(
                &mut wr,
                &DaemonMsg::Welcome {
                    daemon_version: "test".into(),
                    windows: vec![],
                },
            )
            .await
            .unwrap();

            let request = read_frame::<_, ClientMsg>(&mut rd).await.unwrap().unwrap();
            assert_eq!(request, ClientMsg::Kill { window_id: 7 });

            // Two broadcasts ahead of the genuine reply - exactly the interleaving a
            // freshly registered git root or a live window list can produce.
            write_frame(&mut wr, &DaemonMsg::WindowsChanged { windows: vec![] })
                .await
                .unwrap();
            write_frame(
                &mut wr,
                &DaemonMsg::Git {
                    root: "/tmp/repo".into(),
                    state: None,
                },
            )
            .await
            .unwrap();
            write_frame(
                &mut wr,
                &DaemonMsg::Ack {
                    request: "kill".into(),
                },
            )
            .await
            .unwrap();
        });

        let mut client = CliClient::connect(&socket).await.unwrap();
        let reply = client
            .request(ClientMsg::Kill { window_id: 7 })
            .await
            .unwrap();
        assert_eq!(
            reply,
            DaemonMsg::Ack {
                request: "kill".into()
            }
        );

        fake_daemon.await.unwrap();
    }
    use proto::{Runtime, Status};

    fn win(id: u32, name: &str) -> WindowInfo {
        WindowInfo {
            id,
            name: name.into(),
            runtime: Runtime::Shell,
            cwd: "/home/me/repo".into(),
            project: "/home/me/repo".into(),
            worktree: None,
            branch: None,
            status: Status::Idle,
            tool: None,
            since_secs: 5,
            last_output_secs: 5,
            session_id: None,
            model: None,
            subagents: vec![],
            exit: None,
            kind: proto::WindowKind::Pty,
            run: None,
        }
    }

    #[test]
    fn resolve_by_id_then_by_name() {
        let ws = vec![win(1, "api"), win(2, "7"), win(7, "other")];
        assert_eq!(resolve_target(&ws, "1").unwrap(), 1);
        assert_eq!(resolve_target(&ws, "api").unwrap(), 1);
        assert_eq!(
            resolve_target(&ws, "7").unwrap(),
            7,
            "an id match wins even when another window's name is the same string"
        );
        assert!(
            resolve_target(&ws, "nope")
                .unwrap_err()
                .to_string()
                .contains("nope")
        );

        let ws_without_id_7 = vec![win(1, "api"), win(2, "7")];
        assert_eq!(
            resolve_target(&ws_without_id_7, "7").unwrap(),
            2,
            "a numeric name still resolves when no such id exists"
        );
    }

    #[test]
    fn table_has_a_branch_column() {
        let mut worktree_window = win(1, "api");
        worktree_window.branch = Some("feat/x".into());
        let out = format_table(&[worktree_window, win(2, "tests")]);
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), 3);

        let header = lines[0];
        assert!(header.starts_with("ID"), "header: {header:?}");
        let status_at = header.find("STATUS").expect("header: {header:?}");
        let branch_at = header.find("BRANCH").unwrap_or_else(|| {
            panic!("header is missing a BRANCH column, between STATUS and DIR: {header:?}")
        });
        let dir_at = header.rfind("DIR").expect("header: {header:?}");
        assert!(
            status_at < branch_at && branch_at < dir_at,
            "BRANCH must sit between STATUS and DIR: {header:?}"
        );

        assert!(
            lines[1].contains("api")
                && lines[1].contains("idle")
                && lines[1].contains("feat/x")
                && lines[1].contains("/home/me/repo"),
            "a worktree window shows its branch: {:?}",
            lines[1]
        );
        let tests_branch_field = lines[2]
            .split_whitespace()
            .nth(4)
            .unwrap_or_else(|| panic!("row is missing a branch field: {:?}", lines[2]));
        assert_eq!(
            tests_branch_field, "-",
            "a window without a worktree shows '-' in the branch column: {:?}",
            lines[2]
        );
    }

    #[test]
    fn create_timeout_adds_allowance_without_changing_the_default() {
        assert_eq!(REQUEST_TIMEOUT, Duration::from_secs(5));
        assert_eq!(
            CREATE_WINDOW_REPLY_TIMEOUT,
            LAUNCH_GATE_WAIT + daemon::project::DETECT_TIMEOUT + CREATE_REPLY_ALLOWANCE,
        );
        assert_eq!(
            CREATE_WINDOW_REPLY_TIMEOUT,
            Duration::from_secs(12),
            "LAUNCH_GATE_WAIT (5) + DETECT_TIMEOUT (5) + CREATE_REPLY_ALLOWANCE (2)"
        );
        assert!(
            WORKTREE_REQUEST_TIMEOUT > CREATE_WINDOW_REPLY_TIMEOUT
                && WORKTREE_REQUEST_TIMEOUT > REQUEST_TIMEOUT,
            "WORKTREE_REQUEST_TIMEOUT must be the longest of the three, since it is the \
             only one that must outlast the daemon's own worktree operation deadlines",
        );
    }

    /// The assertion the whole-branch review found missing: the budget is pinned against
    /// the **daemon's** constants, not against the CLI's other two. Comparing the three
    /// CLI timeouts with each other cannot notice that one of them has fallen below the
    /// work it is waiting for, which is how 45 s survived while the create path's
    /// supremum was also 45 s.
    ///
    /// The two sums are spelled out numerically as well as symbolically on purpose: a
    /// change to any daemon constant should fail *here*, with the arithmetic in front of
    /// whoever made it, rather than in a timeout on a user's slow repository.
    #[test]
    fn the_budget_clears_the_daemons_worst_case_on_both_git_paths() {
        assert_eq!(
            REMOVAL_WORST_CASE,
            Duration::from_secs(33),
            "OPERATION_TIMEOUT (30) + KILL_GRACE (3)"
        );
        assert_eq!(
            CREATE_WORST_CASE,
            Duration::from_secs(50),
            "LAUNCH_GATE_WAIT (5) + DETECT_TIMEOUT (5) + OPERATION_TIMEOUT (30) \
             + CLEANUP_TIMEOUT (10)"
        );
        assert!(
            WORKTREE_REQUEST_TIMEOUT > CREATE_WORST_CASE,
            "the create path is the longer of the two and the one the old 45 s only \
             matched: {WORKTREE_REQUEST_TIMEOUT:?} vs {CREATE_WORST_CASE:?}",
        );
        assert!(
            WORKTREE_REQUEST_TIMEOUT > REMOVAL_WORST_CASE,
            "{WORKTREE_REQUEST_TIMEOUT:?} vs {REMOVAL_WORST_CASE:?}",
        );
        assert_eq!(WORKTREE_REQUEST_TIMEOUT, Duration::from_secs(62));
    }

    /// M6.8: `restart`'s 15 s budget must exceed `RESTART_WORST_CASE` (the daemon's own
    /// kill-wait deadline, decision 18) with real margin, and must not equal any
    /// production constant the restart path is built from — `docs/timing-budgets.md`'s
    /// standing rule 1, the defect shape that investigation found three times elsewhere
    /// in this workspace.
    #[test]
    fn restart_timeout_clears_the_kill_wait_with_real_margin() {
        assert_eq!(
            RESTART_WORST_CASE,
            Duration::from_secs(10),
            "LAUNCH_GATE_WAIT (5) + KILL_GRACE (3) + wait_for_exit's own 2s margin \
             (decision 18)"
        );
        assert!(
            RESTART_REQUEST_TIMEOUT > RESTART_WORST_CASE,
            "{RESTART_REQUEST_TIMEOUT:?} vs {RESTART_WORST_CASE:?}",
        );
        assert_ne!(
            RESTART_REQUEST_TIMEOUT,
            daemon::manager::KILL_GRACE,
            "must never coincide with the constant the restart path itself runs against"
        );
        assert_ne!(
            RESTART_REQUEST_TIMEOUT, RESTART_WORST_CASE,
            "must never coincide with the deadline it is supposed to exceed"
        );
        // 1.5x margin over the worst case, down from 3x when the launch gate added its
        // own term: 15 s against 10 s, five seconds of absolute headroom. Still a
        // supremum rather than an attainable time — the gate term is only ever paid
        // inside the daemon's first `CODEX_PROBE_TIMEOUT`, and the `KILL_GRACE` term only
        // by a child that ignores every signal up to SIGKILL, so a restart paying both in
        // full is a restart issued in the daemon's first five seconds against a window
        // that also has to be killed the hard way. The remaining work it has to cover is
        // the process spawn phase C pays, which `create`'s own phase B measures at tens of
        // milliseconds, never the seconds git can cost.
        assert_eq!(RESTART_REQUEST_TIMEOUT, Duration::from_secs(15));
    }
}
