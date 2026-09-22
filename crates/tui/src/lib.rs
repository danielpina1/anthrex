//! The anthrex terminal client: connects to the daemon and runs the ratatui event loop.

pub mod app;
pub mod connection;
pub mod dialog;
pub mod graph;
pub mod inspector;
pub mod keymap;
mod mouse;
pub mod reconnect;
pub mod settings;
pub mod spawn;
pub mod theme;
pub mod tree;
mod tree_input;
pub mod ui;

use app::{App, Effect};
use connection::Connection;
use crossterm::event::{
    DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture, Event,
    EventStream, MouseButton, MouseEventKind,
};
use futures::StreamExt;
use proto::DaemonMsg;
use ratatui::DefaultTerminal;
use settings::UiSettings;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub struct TuiOptions {
    pub socket_path: PathBuf,
    pub default_dir: PathBuf,
    /// Window id or name to focus on start.
    pub focus: Option<String>,
    pub settings: UiSettings,
    /// Every config `Problem` the CLI's `attach` found, already formatted (decision 7).
    pub config_problems: Vec<String>,
    /// This binary's own path, so a manual `C-b r` (decision 32) can start the daemon
    /// through `tui::spawn::ensure_daemon` when a plain connect keeps failing. `None`
    /// in tests, which must never spawn a real daemon process by accident.
    pub daemon_exe: Option<PathBuf>,
}

/// Disables mouse capture and bracketed paste on stdout, ignoring any error. Called both from
/// the drop guard on normal/error exit and from the panic hook, so it must be safe to call
/// more than once and must not touch anything that requires an intact runtime.
fn disable_mouse_and_paste() {
    let _ = crossterm::execute!(
        std::io::stdout(),
        DisableBracketedPaste,
        DisableMouseCapture
    );
}

/// Restores the terminal exactly once, on every way out of `run`'s scope: normal return, an
/// `Err` from the event loop, or unwinding from a panic. Without this, a panic inside the
/// event loop would skip plain cleanup statements and leave the user's shell reading raw mouse
/// and bracketed-paste escape sequences until they run `reset`.
struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        disable_mouse_and_paste();
        ratatui::restore();
    }
}

pub async fn run(opts: TuiOptions) -> anyhow::Result<()> {
    let conn = Connection::connect(&opts.socket_path).await?;
    let mut app = App::new(
        conn.windows.clone(),
        opts.default_dir.clone(),
        opts.settings,
    );
    app.home_dir = dirs::home_dir();
    // Decision 7: every config problem the CLI found, shown once at start.
    app.report_config_problems(opts.config_problems);
    if let Some(target) = &opts.focus {
        match app
            .windows
            .iter()
            .find(|w| w.name == *target || w.id.to_string() == *target)
        {
            Some(w) => app.request_focus(w.id),
            None => app.toast(format!("no window named '{target}'")),
        }
    }

    let mut terminal = ratatui::init();
    let _ = crossterm::execute!(std::io::stdout(), EnableMouseCapture, EnableBracketedPaste);

    // ratatui's own panic hook restores raw mode and the alternate screen, then prints the
    // panic message, but it knows nothing about mouse capture or bracketed paste. Disable both
    // before that message is printed, then defer to the hook ratatui (or anyone before us)
    // already installed.
    let prev_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        disable_mouse_and_paste();
        prev_hook(info);
    }));

    // Held across the event loop so cleanup runs exactly once on every exit path.
    let _guard = TerminalGuard;
    // Decision 30: `conn` becomes `Option<Connection>` from here on — `None` whenever
    // the link is down, so `Connection::recv`'s own reader task and channel get torn
    // down and rebuilt by every fresh `reconnect::attempt` rather than kept limping.
    let mut conn = Some(conn);
    event_loop(
        &mut terminal,
        &mut conn,
        &mut app,
        &opts.socket_path,
        opts.daemon_exe,
    )
    .await
}

/// Hands each effect to the connection. Returns true when the client should exit.
///
/// `Connection::send` never suspends, so a daemon that has stopped reading can never
/// freeze the event loop - `Effect::Quit` still runs. Decision 35: every refused send
/// (queue full, or no connection at all while disconnected) is reported through
/// `App::on_send_failed`, which decides per-message-type what if anything to toast.
///
/// `Effect::Reconnect` is handled by `event_loop` itself before it calls `apply` — it
/// needs the socket path, the daemon executable and the in-flight attempt task, none
/// of which this function has — so reaching that arm here is a defensive no-op, not
/// the real handling.
fn apply(effects: Vec<Effect>, conn: Option<&Connection>, app: &mut App) -> bool {
    for effect in effects {
        match effect {
            Effect::Send(msg) => {
                let sent = conn.is_some_and(|c| c.send(msg.clone()));
                if !sent && apply(app.on_send_failed(&msg), conn, app) {
                    return true;
                }
            }
            // `bell.attention` / `bell.done` (decision 4): a bare BEL byte on the outer
            // terminal, which every terminal emulator's own bell setting then decides
            // whether to actually ring. `App` has already checked which of the two
            // settings applies and that the window was not the focused one.
            Effect::Bell => {
                let _ = std::io::stdout().write_all(b"\x07");
                let _ = std::io::stdout().flush();
            }
            Effect::Quit => return true,
            Effect::Reconnect => {}
        }
    }
    false
}

fn draw<B: ratatui::backend::Backend>(
    terminal: &mut ratatui::Terminal<B>,
    app: &mut App,
    conn: Option<&Connection>,
) -> Result<ui::Layout, B::Error> {
    let mut layout = None;
    terminal.draw(|frame| {
        // Terminal::draw has already handled a resize. Settle the viewport before
        // rendering so the next mouse event sees the same rows as this frame.
        let next = ui::layout(
            frame.area(),
            if app.sidebar_visible {
                app.sidebar_width
            } else {
                0
            },
        );
        let effects = app.set_terminal_size(next.main_inner.width, next.main_inner.height);
        app.set_tree_viewports(next.sidebar_list.height, next.main_inner.height);
        app.set_graph_viewport(next.main);
        apply(effects, conn, app);
        layout = Some(ui::draw(frame, app));
    })?;
    Ok(layout.expect("draw closure always runs"))
}

#[cfg(test)]
mod draw_tests;

/// Spawns one reconnect attempt on its own task, so a daemon that never answers can
/// never freeze the event loop (decision 31).
fn spawn_attempt(
    socket: PathBuf,
    daemon_exe: Option<PathBuf>,
) -> tokio::task::JoinHandle<anyhow::Result<Connection>> {
    tokio::spawn(async move { reconnect::attempt(&socket, daemon_exe).await })
}

/// Everything about the connection's lifecycle beyond keyboard, mouse and the
/// once-a-frame tick: the retry schedule, the in-flight attempt task, and the last
/// `Bye` reason seen (decisions 30-32). Kept as its own small piece of state — rather
/// than three more local variables in `event_loop` — so `step` can be driven from a
/// test against a real socket with no terminal at all (see `reconnect_tests.rs`),
/// which is the only way this repo's `AGENTS.md` hard rule 6 ("test real behaviour")
/// can be honoured for an async selection loop this shaped.
struct ConnectionDriver {
    schedule: Option<reconnect::RetrySchedule>,
    inflight: Option<tokio::task::JoinHandle<anyhow::Result<Connection>>>,
    last_bye_reason: Option<String>,
}

impl ConnectionDriver {
    fn new() -> Self {
        Self {
            schedule: None,
            inflight: None,
            last_bye_reason: None,
        }
    }

    /// Waits for whichever comes next: a message on the (still open) connection, the
    /// link actually closing, the retry schedule coming due, or an in-flight attempt
    /// finishing — and returns the `App` effects it produced.
    ///
    /// Each branch's future is wrapped in its own `async { ... }` block rather than
    /// written as a bare `conn.as_mut().unwrap().recv()`-style expression: `tokio::select!`
    /// evaluates every branch's expression up front (even a disabled one) to build its
    /// future, and only skips *polling* the disabled ones. A bare `.unwrap()` would
    /// therefore panic on a `None` guard-skipped branch; wrapping it in `async {}`
    /// defers that body to poll time, where the guard has already ruled it out.
    async fn step(
        &mut self,
        conn: &mut Option<Connection>,
        app: &mut App,
        socket: &Path,
    ) -> Vec<Effect> {
        let due_at = self.schedule.map(|s| s.next_due());
        let conn_ready = conn.is_some();
        let due_ready = due_at.is_some() && self.inflight.is_none();
        let inflight_ready = self.inflight.is_some();

        tokio::select! {
            msg = async { conn.as_mut().unwrap().recv().await }, if conn_ready => {
                self.on_recv(conn, app, msg)
            }
            _ = async {
                tokio::time::sleep_until(tokio::time::Instant::from_std(due_at.unwrap())).await
            }, if due_ready => {
                self.inflight = Some(spawn_attempt(socket.to_path_buf(), None));
                vec![]
            }
            result = async { self.inflight.as_mut().unwrap().await }, if inflight_ready => {
                self.on_attempt_finished(conn, app, result)
            }
            // Decision 31's give-up path leaves every guard above false at once
            // (`conn = None`, `inflight = None`, `schedule = None`) — a `select!`
            // with no `else` and every arm disabled panics the instant it is
            // polled in that state, which happened live exactly 30s after a
            // daemon disappeared. The obvious alternative, `else => vec![]`,
            // returns immediately on every poll and turns `event_loop`'s outer
            // loop into a redraw-every-pass busy spin (measured 95.7-97.3% CPU) —
            // a regression that is *harder* to notice than the panic, since the
            // status bar still looks correct. Parking forever is correct instead:
            // nothing here can make progress until something re-arms the driver
            // (a manual `C-b r` via `reconnect_now`, called from `event_loop`
            // before the next `step`), so this branch must simply never resolve
            // on its own and let the outer `select!` keep servicing keyboard/tick
            // events. Measured CPU with this arm: ~3-4% (see the item's report).
            else => {
                std::future::pending::<()>().await;
                unreachable!("parks forever; only re-armed state reaches step again")
            }
        }
    }

    /// A closed receive drops the connection, calls `app.on_link_lost`, and opens the
    /// automatic retry window (decision 30) — unless the effects it returned include
    /// `Effect::Quit`, i.e. this was `C-b Q`'s own wait ending, not an unplanned drop.
    fn on_recv(
        &mut self,
        conn: &mut Option<Connection>,
        app: &mut App,
        msg: Option<DaemonMsg>,
    ) -> Vec<Effect> {
        match msg {
            Some(DaemonMsg::Bye { reason }) => {
                self.last_bye_reason = Some(reason.clone());
                app.on_daemon(DaemonMsg::Bye { reason })
            }
            Some(msg) => app.on_daemon(msg),
            None => {
                *conn = None;
                let reason = self
                    .last_bye_reason
                    .take()
                    .unwrap_or_else(|| "connection closed".into());
                let effects = app.on_link_lost(reason);
                if !effects.contains(&Effect::Quit) {
                    self.schedule = Some(reconnect::RetrySchedule::after_drop(Instant::now()));
                }
                effects
            }
        }
    }

    /// On success, installs the new connection and hands its window list to
    /// `App::on_reconnected` (decision 33). On failure, records it against the
    /// schedule and reports it through `App::on_reconnect_failed`, which decides
    /// whether that was the one that gives up (decision 31).
    fn on_attempt_finished(
        &mut self,
        conn: &mut Option<Connection>,
        app: &mut App,
        result: Result<anyhow::Result<Connection>, tokio::task::JoinError>,
    ) -> Vec<Effect> {
        self.inflight = None;
        let outcome = match result {
            Ok(inner) => inner,
            Err(join_err) => Err(anyhow::anyhow!("reconnect task failed: {join_err}")),
        };
        match outcome {
            Ok(new_conn) => {
                let windows = new_conn.windows.clone();
                *conn = Some(new_conn);
                self.schedule = None;
                app.on_reconnected(windows)
            }
            Err(err) => {
                let now = Instant::now();
                let keep_going = self.schedule.as_mut().is_some_and(|s| s.after_failure(now));
                if !keep_going {
                    self.schedule = None;
                }
                app.on_reconnect_failed(err.to_string(), !keep_going)
            }
        }
    }

    /// `Effect::Reconnect` (decision 32, `C-b r`): starts an attempt at once and opens
    /// a fresh 30 s window.
    ///
    /// Fix wave 10, item 2: the user's intent in pressing `C-b r` is "try now, and
    /// start the daemon if it is not running" — decision 32's whole reason a manual
    /// attempt is allowed to carry `daemon_exe` where an automatic one never does. If
    /// an automatic retry (spawned by the "due" branch in `step`, always with
    /// `daemon_exe: None` per decision 31) is already running, that in-flight attempt
    /// cannot be handed this press's `daemon_exe` after the fact — a running future's
    /// captured arguments cannot be mutated, and reworking `reconnect::attempt` to poll
    /// a late-bound signal is a bigger change than this fix needs. So this press
    /// supersedes it instead: abort the stale attempt and spawn a fresh one that
    /// actually carries `daemon_exe`. The stale attempt was never going to start the
    /// daemon anyway (it is automatic), so it has nothing this fresh one loses by being
    /// replaced; `JoinHandle::abort` (AGENTS.md's own facts-learned-the-hard-way) takes
    /// effect at the aborted task's next yield point, and its `Connection`, if it had
    /// already connected, still drops and cleans itself up normally.
    fn reconnect_now(&mut self, socket: &Path, daemon_exe: Option<PathBuf>) {
        self.schedule = Some(reconnect::RetrySchedule::manual(Instant::now()));
        if let Some(stale) = self.inflight.take() {
            stale.abort();
        }
        self.inflight = Some(spawn_attempt(socket.to_path_buf(), daemon_exe));
    }
}

async fn event_loop(
    terminal: &mut DefaultTerminal,
    conn: &mut Option<Connection>,
    app: &mut App,
    socket_path: &Path,
    daemon_exe: Option<PathBuf>,
) -> anyhow::Result<()> {
    let mut events = EventStream::new();
    let mut tick = tokio::time::interval(Duration::from_millis(100));
    let mut driver = ConnectionDriver::new();
    loop {
        let layout = draw(terminal, app, conn.as_ref())?;
        let mut effects = tokio::select! {
            Some(event) = events.next() => match event? {
                Event::Key(key) => app.on_key(key),
                Event::Paste(text) => app.on_paste(text),
                Event::Mouse(mouse) => match mouse.kind {
                    MouseEventKind::Down(MouseButton::Left) => app.on_click(mouse.column, mouse.row, &layout),
                    MouseEventKind::Drag(MouseButton::Left) => app.on_drag(mouse.column, mouse.row, &layout),
                    MouseEventKind::ScrollUp => app.on_scroll(true, mouse.column, mouse.row, &layout),
                    MouseEventKind::ScrollDown => app.on_scroll(false, mouse.column, mouse.row, &layout),
                    _ => vec![],
                },
                _ => vec![],
            },
            _ = tick.tick() => app.on_tick(),
            effects = driver.step(conn, app, socket_path) => effects,
        };
        let reconnect_requested = {
            let before = effects.len();
            effects.retain(|effect| !matches!(effect, Effect::Reconnect));
            effects.len() != before
        };
        if reconnect_requested {
            driver.reconnect_now(socket_path, daemon_exe.clone());
        }
        if apply(effects, conn.as_ref(), app) {
            return Ok(());
        }
    }
}

/// `ANTHREX_DATA_DIR`/`ANTHREX_SOCKET` are process-global. Every test in this crate's
/// `--lib` binary that touches them (`spawn::tests`, `reconnect_tests`) shares this one
/// lock rather than each defining its own — two independent, module-private locks
/// would not actually serialize against each other, since `cargo test` runs every
/// `#[test]`/`#[tokio::test]` in a binary concurrently by default. A `tokio::sync::Mutex`
/// rather than `std::sync::Mutex`: `reconnect_tests`'s use of it holds the guard across
/// `.await` points (waiting out a real `ensure_daemon` attempt with the variables set),
/// which a `std` guard cannot do; plain `#[test]`s (`spawn::tests`) use
/// [`tokio::sync::Mutex::blocking_lock`] instead, which needs no runtime.
#[cfg(test)]
pub(crate) static ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[cfg(test)]
mod reconnect_tests;
