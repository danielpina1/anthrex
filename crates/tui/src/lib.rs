//! The anthrex terminal client: connects to the daemon and runs the ratatui event loop.

pub mod actions_request;
pub mod app;
pub mod connection;
pub mod connection_driver;
pub mod conversation;
mod conversation_label;
pub mod dialog;
pub mod doc_note;
pub mod graph;
pub mod inspector;
pub mod keymap;
mod mouse;
pub mod profile_view;
pub mod profile_words;
pub mod reconnect;
pub mod run_edit;
pub mod run_goal;
pub mod run_iterate;
pub mod safe_text;
pub mod settings;
pub mod spawn;
pub mod text_area;
pub mod theme;
pub mod tree;
mod tree_input;
pub mod ui;

use app::{App, Effect};
use connection::Connection;
use connection_driver::ConnectionDriver;
use crossterm::event::{
    DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture, Event,
    EventStream, MouseButton, MouseEventKind,
};
use futures::StreamExt;
use ratatui::DefaultTerminal;
use settings::UiSettings;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

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
    // Decision 1: the connection's one run subscription, first of all.
    let first = vec![app.run_subscription(), app.settings_fetch()];
    apply(first, Some(&conn), &mut app);
    app.home_dir = dirs::home_dir();
    app.utc_offset_secs = local_utc_offset_secs();
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
        let next = ui::layout_for(app, frame.area());
        let effects = app.set_terminal_size(next.main_inner.width, next.main_inner.height);
        app.set_tree_viewports(next.sidebar_list.height, next.main_inner.height);
        app.set_graph_viewport(next.main);
        app.set_body_area(next.body);
        apply(effects, conn, app);
        layout = Some(ui::draw(frame, app));
    })?;
    Ok(layout.expect("draw closure always runs"))
}

#[cfg(test)]
mod draw_tests;

/// The local zone's offset from UTC now, in seconds, for the conversation view's turn
/// times (task M6.5.13) and `anthrex run status`'s promotion time. Reading the zone is
/// I/O (`/etc/localtime`, `TZ`), so it happens here, once, and `App` only holds the number. 0 (UTC) if the zone cannot be read.
// `tm_gmtoff` is a C `long`: `i64` here, `i32` on a 32-bit target, so the cast stays.
#[allow(clippy::unnecessary_cast)]
pub fn local_utc_offset_secs() -> i64 {
    // SAFETY: `time` with a null pointer only returns the time; `localtime_r` writes into
    // the `tm` we own and returns null on failure, which is checked.
    unsafe {
        let now = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        if libc::localtime_r(&now, &mut tm).is_null() {
            return 0;
        }
        tm.tm_gmtoff as i64
    }
}

#[cfg(test)]
mod clock_tests {
    /// Every real zone is within UTC-12 and UTC+14, and on a whole quarter hour.
    #[test]
    fn the_local_offset_is_a_real_zone_offset() {
        let offset = super::local_utc_offset_secs();
        assert!((-12 * 3600..=14 * 3600).contains(&offset), "{offset}");
        assert_eq!(offset % 900, 0, "{offset}");
    }
}

/// Spawns one reconnect attempt on its own task, so a daemon that never answers can
/// never freeze the event loop (decision 31).
fn spawn_attempt(
    socket: PathBuf,
    daemon_exe: Option<PathBuf>,
) -> tokio::task::JoinHandle<anyhow::Result<Connection>> {
    tokio::spawn(async move { reconnect::attempt(&socket, daemon_exe).await })
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
#[cfg(test)]
mod theme_kit_tests;
