//! What the fake says on stderr, also kept where a test can show it. A headless
//! session's stderr reaches only the daemon (a window's diagnostics, gone with the
//! window), so a fake that failed on CI said why to no one. With `FAKE_AGENT_ERROR_LOG`
//! set, every line is appended there too, with the time, the pid, the script and the
//! cwd (`RunHarness::trail` prints it when a run test fails).

use std::io::Write;
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

/// The script this process runs, once it is known.
static SCRIPT: OnceLock<String> = OnceLock::new();

/// Names this process's script in every later line.
pub fn set_script(name: &str) {
    let _ = SCRIPT.set(name.to_owned());
}

/// `text` appended to `FAKE_AGENT_ERROR_LOG` when it is set, then on stderr. Never fails
/// and never panics: the file comes first, so a stderr the daemon has stopped reading
/// (`eprintln!` would panic on it) cannot lose the line.
pub fn emit(text: &str) {
    keep(text);
    let _ = writeln!(std::io::stderr(), "{text}");
}

/// Every panic's message kept as [`emit`] keeps a line (2026-10-06): the default hook
/// prints it on stderr only, which reaches the daemon and not the test.
pub fn keep_panics() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        keep(&format!("fake-agent: {info}"));
        default(info);
    }));
}

/// `text` appended to `FAKE_AGENT_ERROR_LOG` with the time, the pid, the script and
/// the cwd, when it is set.
fn keep(text: &str) {
    let Some(path) = std::env::var_os("FAKE_AGENT_ERROR_LOG") else {
        return;
    };
    let ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or_default();
    let script = SCRIPT.get().map(String::as_str).unwrap_or("-");
    let cwd = std::env::current_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    let line = format!(
        "{ms} pid {} {script} in {cwd}: {text}\n",
        std::process::id()
    );
    // One `write` in append mode, so processes sharing the file do not interleave.
    let _ = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .and_then(|mut file| file.write_all(line.as_bytes()));
}

/// [`emit`] with `format!` arguments.
macro_rules! say {
    ($($arg:tt)*) => {
        $crate::diag::emit(&format!($($arg)*))
    };
}
pub(crate) use say;
