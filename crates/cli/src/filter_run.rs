//! `anthrex filter-run --mode <m> --log-dir <dir> -c <command>` (milestone 8b decision
//! 27): runs `<shell> -c <command>` unchanged ([`shell`]: the Bash tool's bash or zsh,
//! else `/bin/sh`), logs every byte of its output, and prints
//! only `daemon::output_filter::apply`'s view of it, then where the full log is. It
//! exits as the command did. A log it cannot create never stops the command: it then
//! runs unfiltered.

use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::io::{Read, Write};
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};

use daemon::output_filter::{apply, parse_mode};
use proto::OutputFilter;

/// The log stops growing here, with [`LOG_CUT`] as its last line.
const LOG_MAX: u64 = 64 * 1024 * 1024;
const LOG_CUT: &[u8] = b"[anthrex] log cut at 64 MiB\n";
/// Every line is kept up to this many; past it, only the last [`LINES_TAIL`].
const LINES_MAX: usize = 20_000;
const LINES_TAIL: usize = 5_000;
/// The bytes of one line kept for the view: enough for `LINE_MAX_CHARS` characters of
/// up to four bytes each, which is all `apply` keeps.
const LINE_BYTES: usize = daemon::output_filter::LINE_MAX_CHARS * 4 + 48;

struct Options {
    mode: OutputFilter,
    log_dir: PathBuf,
    command: OsString,
}

pub fn main(args: Vec<OsString>) -> i32 {
    let Some(options) = parse(args) else {
        eprintln!(
            "usage: anthrex filter-run --mode <failures-only|tail|none> --log-dir <dir> -c <command>"
        );
        return 2;
    };
    let program = shell(std::env::var_os("SHELL").as_deref());
    let program = if program.is_file() {
        program
    } else {
        PathBuf::from("/bin/sh")
    };
    let mut command = Command::new(program);
    command
        .arg("-c")
        .arg(&options.command)
        .stdin(Stdio::inherit());
    let (path, log) = match open_log(&options.log_dir) {
        Ok(opened) => opened,
        Err(error) => {
            eprintln!("[anthrex] could not write the log: {error}");
            return match command.status() {
                Ok(status) => exit_code(status),
                Err(error) => spawn_failed(error),
            };
        }
    };
    match filtered(command, &options, &path, log) {
        Ok(code) => code,
        Err(error) => spawn_failed(error),
    }
}

fn spawn_failed(error: std::io::Error) -> i32 {
    eprintln!("[anthrex] could not run the shell: {error}");
    127
}

fn parse(args: Vec<OsString>) -> Option<Options> {
    let mut args = args.into_iter();
    let (mut mode, mut log_dir, mut command) = (None, None, None);
    while let Some(flag) = args.next() {
        let value = args.next()?;
        match flag.to_str()? {
            "--mode" => mode = Some(parse_mode(value.to_str()?)?),
            "--log-dir" => log_dir = Some(PathBuf::from(value)),
            "-c" => command = Some(value),
            _ => return None,
        }
    }
    Some(Options {
        mode: mode?,
        log_dir: log_dir?,
        command: command?,
    })
}

/// Creates `<dir>/<unix ms>-<pid>.log`, and `<dir>` when missing.
fn open_log(dir: &Path) -> std::io::Result<(PathBuf, File)> {
    std::fs::create_dir_all(dir)?;
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or_default();
    let path = dir.join(format!("{ms}-{}.log", std::process::id()));
    let file = create_log(&path)?;
    Ok((path, file))
}

/// Creates the log file, never opening one that exists (or a link in its place).
fn create_log(path: &Path) -> std::io::Result<File> {
    File::options().write(true).create_new(true).open(path)
}

/// The shell the command runs under (ruling on review I1): the Bash tool's own, `$SHELL`,
/// when it is an absolute path to `bash` or `zsh` (Claude Code's own rule), so the
/// wrapped command parses and globs exactly as the original would; `/bin/sh` otherwise.
fn shell(value: Option<&OsStr>) -> PathBuf {
    let path = value.map(Path::new);
    match path {
        Some(path)
            if path.is_absolute()
                && matches!(
                    path.file_name().and_then(OsStr::to_str),
                    Some("bash" | "zsh")
                ) =>
        {
            path.to_path_buf()
        }
        _ => PathBuf::from("/bin/sh"),
    }
}

/// Runs the command with stdout and stderr on one pipe, logs and collects its output,
/// prints the view, and returns the command's exit code.
fn filtered(
    mut command: Command,
    options: &Options,
    path: &Path,
    log: File,
) -> std::io::Result<i32> {
    let (mut reader, writer) = std::io::pipe()?;
    command.stdout(writer.try_clone()?).stderr(writer);
    let mut child = command.spawn()?;
    // Our copies of the pipe's write end go with the command, so the read ends when
    // the command's last writer does.
    drop(command);
    let mut log = Log::new(log);
    let mut lines = Lines::default();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                log.write(&buf[..n]);
                lines.push(&buf[..n]);
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => break,
        }
    }
    let code = exit_code(child.wait()?);
    let (total, kept) = lines.finish();
    let mut stdout = std::io::stdout().lock();
    for line in apply(options.mode, &kept, code) {
        let _ = writeln!(stdout, "{line}");
    }
    let _ = writeln!(
        stdout,
        "[anthrex] full output: {} ({total} lines, exit {code})",
        path.display()
    );
    let _ = stdout.flush();
    Ok(code)
}

/// The command's code, or 128 + the signal that killed it.
fn exit_code(status: ExitStatus) -> i32 {
    status
        .code()
        .or_else(|| status.signal().map(|signal| 128 + signal))
        .unwrap_or(1)
}

/// The full log, cut at [`LOG_MAX`]. A write error stops the logging, never the command.
struct Log {
    file: Option<File>,
    written: u64,
    last: u8,
}

impl Log {
    fn new(file: File) -> Self {
        Self {
            file: Some(file),
            written: 0,
            last: b'\n',
        }
    }

    fn write(&mut self, bytes: &[u8]) {
        let Some(file) = self.file.as_mut() else {
            return;
        };
        let room = (LOG_MAX - self.written).min(bytes.len() as u64) as usize;
        let mut ok = file.write_all(&bytes[..room]).is_ok();
        self.written += room as u64;
        if room > 0 {
            self.last = bytes[room - 1];
        }
        if ok && room < bytes.len() {
            if self.last != b'\n' {
                ok = file.write_all(b"\n").is_ok();
            }
            ok = ok && file.write_all(LOG_CUT).is_ok();
            self.written = LOG_MAX;
            self.file = None;
        }
        if !ok {
            self.file = None;
        }
    }
}

/// The output's lines: every one up to [`LINES_MAX`], then only the last
/// [`LINES_TAIL`]; each kept to its first [`LINE_BYTES`] bytes.
#[derive(Default)]
struct Lines {
    kept: Vec<Vec<u8>>,
    current: Vec<u8>,
    open: bool,
    total: usize,
}

impl Lines {
    fn push(&mut self, bytes: &[u8]) {
        for piece in bytes.split_inclusive(|b| *b == b'\n') {
            let (text, ends) = match piece.strip_suffix(b"\n") {
                Some(text) => (text, true),
                None => (piece, false),
            };
            let room = LINE_BYTES.saturating_sub(self.current.len());
            self.current
                .extend_from_slice(&text[..room.min(text.len())]);
            self.open = true;
            if ends {
                self.end_line();
            }
        }
    }

    fn end_line(&mut self) {
        let mut line = std::mem::take(&mut self.current);
        if line.last() == Some(&b'\r') {
            line.pop();
        }
        self.kept.push(line);
        self.open = false;
        self.total += 1;
        if self.kept.len() > LINES_MAX {
            self.kept.drain(..self.kept.len() - LINES_TAIL);
        }
    }

    /// The line count, and the kept lines as text.
    fn finish(mut self) -> (usize, Vec<String>) {
        if self.open {
            self.end_line();
        }
        if self.total > LINES_MAX && self.kept.len() > LINES_TAIL {
            self.kept.drain(..self.kept.len() - LINES_TAIL);
        }
        let kept = self
            .kept
            .iter()
            .map(|line| String::from_utf8_lossy(line).into_owned())
            .collect();
        (self.total, kept)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_keep_every_line_then_the_last_5000() {
        let mut lines = Lines::default();
        let text: String = (1..=20_000).map(|i| format!("{i}\n")).collect();
        lines.push(text.as_bytes());
        lines.push(b"tail without newline");
        let (total, kept) = lines.finish();
        assert_eq!(total, 20_001);
        assert_eq!(kept.len(), 5_000);
        assert_eq!(kept[0], "15002");
        assert_eq!(kept.last().unwrap(), "tail without newline");

        let mut few = Lines::default();
        few.push(b"a\r\nb\n\nc");
        assert_eq!(
            few.finish(),
            (4, vec!["a".into(), "b".into(), "".into(), "c".into()])
        );
    }

    #[test]
    fn a_long_line_is_kept_to_its_first_bytes() {
        let mut lines = Lines::default();
        lines.push(&vec![b'x'; LINE_BYTES * 3]);
        lines.push(b"\n");
        let (total, kept) = lines.finish();
        assert_eq!((total, kept[0].len()), (1, LINE_BYTES));
    }

    /// Review I1: the Bash tool's own shell when it is bash or zsh, else `/bin/sh`.
    #[test]
    fn the_command_runs_under_bash_or_zsh_from_shell_else_bin_sh() {
        let pick = |value: Option<&str>| shell(value.map(OsStr::new));
        for yes in [
            "/bin/zsh",
            "/bin/bash",
            "/opt/homebrew/bin/zsh",
            "/usr/local/bin/bash",
        ] {
            assert_eq!(pick(Some(yes)), PathBuf::from(yes));
        }
        for no in [
            None,
            Some(""),
            Some("zsh"),
            Some("bin/bash"),
            Some("/bin/fish"),
            Some("/usr/bin/zsh-5.9"),
            Some("/bin/sh"),
            Some("/bin/dash"),
        ] {
            assert_eq!(pick(no), PathBuf::from("/bin/sh"), "{no:?}");
        }
    }

    /// Review M1: the log is created, never reused: an existing file or a symbolic link
    /// at its path is refused and left as it was.
    #[test]
    fn create_log_never_replaces_a_file() {
        let dir = Path::new("/tmp").join(format!("ax-create-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let existing = dir.join("1-2.log");
        std::fs::write(&existing, "keep").unwrap();
        let err = create_log(&existing).unwrap_err();
        let link = dir.join("3-4.log");
        std::os::unix::fs::symlink(&existing, &link).unwrap();
        let link_err = create_log(&link).unwrap_err();
        let kept = std::fs::read_to_string(&existing).unwrap();
        let fresh = create_log(&dir.join("5-6.log")).is_ok();
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(err.kind(), std::io::ErrorKind::AlreadyExists);
        assert_eq!(link_err.kind(), std::io::ErrorKind::AlreadyExists);
        assert_eq!(kept, "keep");
        assert!(fresh);
    }

    #[test]
    fn the_log_is_cut_at_its_limit_with_a_last_line() {
        let dir = Path::new("/tmp").join(format!("ax-log-{}", std::process::id()));
        let (path, file) = open_log(&dir).unwrap();
        let mut log = Log::new(file);
        log.written = LOG_MAX - 3;
        log.write(b"abcdef");
        log.write(b"more");
        let written = std::fs::read(&path).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(written, b"abc\n[anthrex] log cut at 64 MiB\n");
    }
}
