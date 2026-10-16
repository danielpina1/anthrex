//! The ax-leak-2 fix: a test-only trace of who made each task temporary directory, so
//! a leak gate that finds `/tmp/ax-<uid>/<hash>` left behind names its creator.
//!
//! A directory's name is a hash of its repository's path (`tmp::task_tmp`) and says
//! nothing about the test that made it. With [`TRACE_ENV`] set to a file, every process
//! of the daemon's code (a test binary, or a daemon a test started, which inherits the
//! variable) appends one line to that file for each task temporary directory it creates:
//! [`FIELDS`], tab-separated. `scripts/leak_gate.py check` reads the same file and
//! prints the line of each leaked directory next to it.
//!
//! Off, and costing one cached environment lookup, unless the variable is set; it writes
//! nothing but that one file. Nothing in a user's daemon sets it.

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

/// The variable naming the trace file.
pub const TRACE_ENV: &str = "AX_TMP_TRACE";

/// A trace line's fields, in order. `scripts/leak_gate.py` reads them by these names
/// (its `TRACE_FIELDS`, pinned by `the_leak_gate_reads_these_fields`).
pub const FIELDS: [&str; 6] = ["created", "hash", "repo", "exe", "test", "pid"];

fn trace_file() -> Option<&'static Path> {
    static FILE: OnceLock<Option<PathBuf>> = OnceLock::new();
    FILE.get_or_init(|| {
        std::env::var_os(TRACE_ENV)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    })
    .as_deref()
}

/// The repository each traced name was hashed from, by the name's path.
fn named() -> &'static Mutex<HashMap<PathBuf, PathBuf>> {
    static NAMED: OnceLock<Mutex<HashMap<PathBuf, PathBuf>>> = OnceLock::new();
    NAMED.get_or_init(Default::default)
}

/// `task_tmp`'s hook: remembers that `tmp` is named from `repo_dir`, when tracing.
pub(super) fn named_from(tmp: &Path, repo_dir: &Path) {
    if trace_file().is_some() {
        crate::lock(named()).insert(tmp.to_path_buf(), repo_dir.to_path_buf());
    }
}

/// `engine_child`'s hook: the task temporary directory `tmp` was just made. Blocking (a
/// one-line append), as its caller is.
pub(crate) fn created(tmp: &Path) {
    let Some(file) = trace_file() else {
        return;
    };
    let repo = crate::lock(named()).get(tmp).cloned();
    let exe = std::env::current_exe().ok();
    let exe = exe.as_deref().and_then(Path::file_name).map(Path::new);
    let test = std::env::var("NEXTEST_TEST_NAME").ok();
    write_creation(file, tmp, repo.as_deref(), exe, test.as_deref());
}

/// Appends `tmp`'s line to `file` in one write: its hash, the repository it was named
/// from (`-` when unknown), the creating binary, the test (nextest's
/// `NEXTEST_TEST_NAME`, `-` outside one) and this process's id. A failure is ignored:
/// the trace is a diagnostic and must never fail what it watches.
pub(crate) fn write_creation(
    file: &Path,
    tmp: &Path,
    repo: Option<&Path>,
    exe: Option<&Path>,
    test: Option<&str>,
) {
    let shown = |p: Option<&Path>| p.map_or("-".into(), |p| p.display().to_string());
    let hash = tmp.file_name().map(|n| n.to_string_lossy().to_string());
    let line = format!(
        "created\t{}\t{}\t{}\t{}\t{}\n",
        hash.as_deref().unwrap_or("-"),
        shown(repo),
        shown(exe),
        test.unwrap_or("-"),
        std::process::id()
    );
    // One `write` on an `O_APPEND` file: lines from concurrent processes never mix.
    let _ = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(file)
        .and_then(|mut f| f.write_all(line.as_bytes()));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_creation_is_one_line_naming_its_repository_binary_and_test() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("trace.tsv");
        let tmp = Path::new("/tmp/ax-1/0123456789abcdef");
        write_creation(
            &file,
            tmp,
            Some(Path::new("/d/runs/r1/tasks/t1")),
            Some(Path::new("anthrex")),
            Some("it::a_test"),
        );
        write_creation(&file, tmp, None, None, None);
        let text = std::fs::read_to_string(&file).unwrap();
        let pid = std::process::id();
        assert_eq!(
            text,
            format!(
                "created\t0123456789abcdef\t/d/runs/r1/tasks/t1\tanthrex\tit::a_test\t{pid}\n\
                 created\t0123456789abcdef\t-\t-\t-\t{pid}\n"
            )
        );
    }

    /// The leak gate parses what this writes: its `TRACE_FIELDS` line must name these
    /// fields in this order.
    #[test]
    fn the_leak_gate_reads_these_fields() {
        let script = include_str!("../../../../../scripts/leak_gate.py");
        let want = format!(
            "TRACE_FIELDS = ({})",
            FIELDS.map(|f| format!("\"{f}\"")).join(", ")
        );
        assert!(
            script.lines().any(|l| l == want),
            "scripts/leak_gate.py has no `{want}` line"
        );
    }
}
