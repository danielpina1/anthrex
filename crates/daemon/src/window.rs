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

/// Input chunks that may be waiting for the PTY before `write_input` starts refusing them.
///
/// A program in raw mode that never reads its stdin (an agent busy thinking, a `sleep`
/// under `stty raw`) stalls writes to the PTY master after about a kilobyte on macOS.
/// The writer thread absorbs that stall; this queue bounds how much unwritten input the
/// daemon holds on its behalf before it tells the client the program is not listening.
pub const INPUT_QUEUE_CAPACITY: usize = 256;

/// One bell or title change, in the order vt100's callbacks fired for them.
///
/// A single PTY read can bundle several escape sequences (e.g. an OSC title set
/// terminated by BEL, immediately followed by a lone bell byte). Recording each
/// callback invocation in order - rather than diffing "changed since last chunk"
/// counters after the whole chunk is processed - keeps the emitted `WindowEvent`s
/// in the same order the bytes actually arrived, instead of a fixed Bell-then-Title
/// order that could reorder events relative to each other.
enum CallbackEvent {
    Bell,
    Title(String),
}

/// Bell and title arrive through vt100's callback trait, so we log them here in order.
#[derive(Default)]
struct ScreenCallbacks {
    events: Vec<CallbackEvent>,
}

impl vt100::Callbacks for ScreenCallbacks {
    fn audible_bell(&mut self, _: &mut vt100::Screen) {
        self.events.push(CallbackEvent::Bell);
    }

    fn set_window_title(&mut self, _: &mut vt100::Screen, title: &[u8]) {
        self.events.push(CallbackEvent::Title(String::from_utf8_lossy(title).into_owned()));
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
    /// Bounded hand-off to the writer thread. Sending never blocks the caller.
    input_tx: std::sync::mpsc::SyncSender<Vec<u8>>,
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
        Self::spawn_with_output_capacity(id, plan, cols, rows, events, OUTPUT_CHANNEL_CAPACITY)
    }

    /// `spawn` with the broadcast capacity chosen by the caller. Tests use a tiny capacity
    /// to make a subscriber lag deterministically.
    pub fn spawn_with_output_capacity(
        id: u32,
        plan: &LaunchPlan,
        cols: u16,
        rows: u16,
        events: mpsc::UnboundedSender<(u32, WindowEvent)>,
        output_capacity: usize,
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
        let mut writer = pair.master.take_writer()?;
        let parser = Arc::new(Mutex::new(vt100::Parser::new_with_callbacks(
            rows,
            cols,
            0,
            ScreenCallbacks::default(),
        )));
        let (output_tx, _) = broadcast::channel(output_capacity);

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
                    let batch = {
                        // Hold the lock across process + send so `attach` can never
                        // observe a chunk in the screen without also receiving it, or vice versa.
                        let mut p = reader_parser.lock().unwrap();
                        p.process(&chunk);
                        let _ = reader_tx.send(chunk);
                        std::mem::take(&mut p.callbacks_mut().events)
                    };
                    let _ = reader_events.send((id, WindowEvent::Output));
                    for ev in batch {
                        let event = match ev {
                            CallbackEvent::Bell => WindowEvent::Bell,
                            CallbackEvent::Title(t) => WindowEvent::Title(t),
                        };
                        let _ = reader_events.send((id, event));
                    }
                }
            })?;

        // A dedicated writer thread owns the PTY master's write side. Writes to it can
        // block for seconds when the child is not reading stdin, so no caller - and in
        // particular no tokio worker holding the manager's mutex - may perform them.
        let (input_tx, input_rx) = std::sync::mpsc::sync_channel::<Vec<u8>>(INPUT_QUEUE_CAPACITY);
        std::thread::Builder::new()
            .name(format!("pty-write-{id}"))
            .spawn(move || {
                // Ends when the channel closes (the Window was dropped) or a write fails
                // (the child is gone and the master reports EIO).
                for chunk in input_rx {
                    if writer.write_all(&chunk).is_err() || writer.flush().is_err() {
                        break;
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

        Ok(Self { master: pair.master, input_tx, pid, parser, output_tx })
    }

    pub fn pid(&self) -> Option<u32> {
        self.pid
    }

    /// Queues `bytes` for the PTY. Never blocks: the writer thread does the blocking write.
    pub fn write_input(&self, bytes: &[u8]) -> anyhow::Result<()> {
        use std::sync::mpsc::TrySendError;
        match self.input_tx.try_send(bytes.to_vec()) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_)) => {
                anyhow::bail!("input queue full; the program is not reading input")
            }
            Err(TrySendError::Disconnected(_)) => anyhow::bail!("the window is no longer accepting input"),
        }
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

    /// Escape codes that put a fresh parser into this screen's state.
    ///
    /// `state_formatted()` in vt100 0.16 is already `contents_formatted()` followed by
    /// `input_mode_formatted()`, so it is the whole snapshot on its own - emitting any of
    /// those a second time would write the screen twice. What it does not encode is
    /// `?1049`, so the alternate screen has to be entered first, or a client attaching to
    /// vim, less or codex would believe it is on the primary screen.
    fn snapshot_of(screen: &vt100::Screen) -> Vec<u8> {
        let mut out = Vec::new();
        if screen.alternate_screen() {
            out.extend_from_slice(b"\x1b[?1049h");
        }
        out.extend_from_slice(&screen.state_formatted());
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
