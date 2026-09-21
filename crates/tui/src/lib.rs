//! The anthrex terminal client: connects to the daemon and runs the ratatui event loop.

pub mod app;
pub mod connection;
pub mod dialog;
pub mod graph;
pub mod inspector;
pub mod keymap;
mod mouse;
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
    let mut conn = Connection::connect(&opts.socket_path).await?;
    let mut app = App::new(
        conn.windows.clone(),
        opts.default_dir.clone(),
        Keymap::default_prefix(),
    );
    app.home_dir = dirs::home_dir();
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
    event_loop(&mut terminal, &mut conn, &mut app).await
}

/// Hands each effect to the connection. Returns true when the client should exit.
///
/// `Connection::send` never suspends, so a daemon that has stopped reading can never
/// freeze the event loop - `Effect::Quit` still runs. A dropped `Input` is not worth a
/// toast (the next keystroke will try again), but a dropped command would silently do
/// nothing, so that one is reported.
fn apply(effects: Vec<Effect>, conn: &Connection, app: &mut App) -> bool {
    for effect in effects {
        match effect {
            Effect::Send(msg) => {
                let is_input = matches!(msg, proto::ClientMsg::Input { .. });
                if !conn.send(msg) && !is_input {
                    app.toast("daemon is not responding");
                }
            }
            Effect::Quit => return true,
        }
    }
    false
}

fn draw<B: ratatui::backend::Backend>(
    terminal: &mut ratatui::Terminal<B>,
    app: &mut App,
    conn: &Connection,
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

async fn event_loop(
    terminal: &mut DefaultTerminal,
    conn: &mut Connection,
    app: &mut App,
) -> anyhow::Result<()> {
    let mut events = EventStream::new();
    let mut tick = tokio::time::interval(Duration::from_millis(100));
    loop {
        let layout = draw(terminal, app, conn)?;
        let effects = tokio::select! {
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
        if apply(effects, conn, app) {
            return Ok(());
        }
    }
}
