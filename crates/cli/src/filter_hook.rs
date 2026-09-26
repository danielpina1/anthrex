//! `anthrex filter-hook --mode <m> --log-dir <dir> [--prefix <p>]…` (milestone 8b
//! decision 28): a Claude worker's `PreToolUse` hook. It reads the payload from stdin
//! and prints `daemon::output_filter::rewrite`'s answer when the call is a matching
//! `Bash` command, else nothing. It always exits 0, never writes to stderr, and gives
//! up at `FILTER_HOOK_DEADLINE`, reading included.

use std::ffi::OsString;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::time::Instant;

use daemon::output_filter::{
    FILTER_HOOK_DEADLINE, FILTER_HOOK_PAYLOAD_MAX, FilterHook, parse_mode, rewrite,
};

pub fn run(args: Vec<OsString>, started: Instant) {
    std::panic::set_hook(Box::new(|_| {}));
    let (done, answered) = std::sync::mpsc::channel();
    // The main thread owns the deadline, even when stdin is held open; the caller's
    // `process::exit` also ends the reader.
    let _ = std::thread::Builder::new()
        .name("filter-hook".into())
        .spawn(move || {
            let _ = done.send(answer(args));
        });
    let wait = FILTER_HOOK_DEADLINE.saturating_sub(started.elapsed());
    if let Ok(Some(out)) = answered.recv_timeout(wait) {
        let mut stdout = std::io::stdout().lock();
        let _ = writeln!(stdout, "{out}");
        let _ = stdout.flush();
    }
}

fn answer(args: Vec<OsString>) -> Option<String> {
    let hook = parse(args)?;
    let mut payload = Vec::new();
    std::io::stdin()
        .lock()
        .take(FILTER_HOOK_PAYLOAD_MAX as u64 + 1)
        .read_to_end(&mut payload)
        .ok()?;
    if payload.len() > FILTER_HOOK_PAYLOAD_MAX {
        return None;
    }
    let payload: serde_json::Value = serde_json::from_slice(&payload).ok()?;
    let exe = std::env::current_exe().ok()?;
    rewrite(&payload, &exe, &hook).map(|out| out.to_string())
}

fn parse(args: Vec<OsString>) -> Option<FilterHook> {
    let mut args = args.into_iter();
    let (mut mode, mut log_dir, mut prefixes) = (None, None, Vec::new());
    while let Some(flag) = args.next() {
        let value = args.next()?.into_string().ok()?;
        match flag.to_str()? {
            "--mode" => mode = Some(parse_mode(&value)?),
            "--log-dir" => log_dir = Some(PathBuf::from(value)),
            "--prefix" => prefixes.push(value),
            _ => return None,
        }
    }
    Some(FilterHook {
        mode: mode?,
        prefixes,
        log_dir: log_dir?,
    })
}
