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
