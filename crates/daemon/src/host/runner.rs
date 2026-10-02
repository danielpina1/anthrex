//! How [`super::GhHost`] runs a command (decision 6). The one place in `host/` a process
//! starts: [`SystemRunner`] spawns `gh` through `subprocess` (a scrubbed `GIT_*`
//! environment, null stdin, its own process group, a deadline) and runs `git` through
//! `worktree::run_git` (`-C <dir> --no-optional-locks`, the same scrubbing, `LC_ALL=C`,
//! `GIT_TERMINAL_PROMPT=0`), AGENTS.md rule 11. `FakeGh` (task M9.2.5) is the other
//! `Runner`. Blocking: call only from `spawn_blocking`.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use super::HostError;
use crate::subprocess::{self, Outcome};
use crate::worktree::{self, WorktreeError};

/// Stderr kept from a `gh` command; `classify` reads its last lines.
const GH_STDERR_MAX: usize = 16 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Program {
    Gh,
    Git,
}

impl Program {
    pub fn name(self) -> &'static str {
        match self {
            Program::Gh => "gh",
            Program::Git => "git",
        }
    }
}

/// How much stdout a command may print.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Capture {
    /// At most this many bytes; more fails the command.
    Bytes(usize),
    /// Any size: the first `head` bytes and the last `tail` are kept (decision 27 step 1).
    HeadTail { head: usize, tail: usize },
}

/// Where a [`Capture::HeadTail`] output was cut: `stdout[..at]` is the head, the rest the
/// tail, and `dropped` bytes between them were read and thrown away.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cut {
    pub at: usize,
    pub dropped: u64,
}

/// One command's result. A command that ran and exited non-zero is `Ok` with `success`
/// false; `Err` means it never ran to its end (not installed, timed out, over its cap).
/// `stdout` is kept whatever the exit status (a refused `git push --porcelain` prints
/// why on stdout).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RunOutput {
    pub success: bool,
    pub stdout: Vec<u8>,
    pub stderr: String,
    pub cut: Option<Cut>,
}

impl RunOutput {
    pub fn stdout_text(&self) -> String {
        String::from_utf8_lossy(&self.stdout).into_owned()
    }
}

/// `argv` excludes the program; for `git` it excludes `-C <dir> --no-optional-locks`,
/// which `worktree::run_git` adds. `env` is added to the daemon's environment (`gh`
/// only; `git` gets `run_git`'s).
pub trait Runner: Send + Sync {
    fn run(
        &self,
        program: Program,
        dir: &Path,
        argv: &[String],
        env: &[(String, String)],
        timeout: Duration,
        cap: Capture,
    ) -> Result<RunOutput, HostError>;
}

/// Production's runner: the real `gh` and `git` binaries.
#[derive(Debug, Clone)]
pub struct SystemRunner {
    gh: PathBuf,
    git: PathBuf,
}

impl SystemRunner {
    pub fn new(gh: impl Into<PathBuf>, git: impl Into<PathBuf>) -> Self {
        SystemRunner {
            gh: gh.into(),
            git: git.into(),
        }
    }

    fn run_gh(
        &self,
        dir: &Path,
        argv: &[String],
        env: &[(String, String)],
        timeout: Duration,
        cap: Capture,
    ) -> Result<RunOutput, HostError> {
        let mut command = Command::new(&self.gh);
        command.args(argv).current_dir(dir);
        for (key, value) in env {
            command.env(key, value);
        }
        let shown = || shown(Program::Gh, argv);
        let (outcome, stdout, stderr, spawn_error, cut) = match cap {
            Capture::Bytes(max) => {
                let (c, stdout) =
                    subprocess::run_captured_keeping(&mut command, max, GH_STDERR_MAX, timeout);
                (c.outcome, stdout, c.stderr, c.spawn_error, None)
            }
            Capture::HeadTail { head, tail } => {
                let (outcome, kept, stderr, spawn_error) = subprocess::run_captured_head_tail(
                    &mut command,
                    head,
                    tail,
                    GH_STDERR_MAX,
                    timeout,
                );
                let cut = kept.dropped().then(|| Cut {
                    at: kept.head.len(),
                    dropped: kept.total - (kept.head.len() + kept.tail.len()) as u64,
                });
                let mut stdout = kept.head;
                stdout.extend_from_slice(&kept.tail);
                (outcome, stdout, stderr, spawn_error, cut)
            }
        };
        if let Some(kind) = spawn_error {
            return Err(if kind == std::io::ErrorKind::NotFound {
                HostError::Missing(format!(
                    "gh is not installed (looked for {})",
                    self.gh.display()
                ))
            } else {
                HostError::Failed(format!("could not start {}: {kind}", shown()))
            });
        }
        match outcome {
            Outcome::Complete(_) => Ok(RunOutput {
                success: true,
                stdout,
                stderr,
                cut,
            }),
            Outcome::Failed => Ok(RunOutput {
                success: false,
                stdout,
                stderr,
                cut,
            }),
            Outcome::TimedOut(_) => Err(timed_out(&shown(), timeout)),
            Outcome::Truncated(_) => Err(HostError::Failed(format!(
                "{} printed more output than anthrex reads",
                shown()
            ))),
        }
    }

    fn run_git(
        &self,
        dir: &Path,
        argv: &[String],
        env: &[(String, String)],
        timeout: Duration,
        cap: Capture,
    ) -> Result<RunOutput, HostError> {
        let shown = shown(Program::Git, argv);
        let Capture::Bytes(max) = cap else {
            return Err(HostError::Failed(format!(
                "{shown}: git output is never cut"
            )));
        };
        if !env.is_empty() {
            return Err(HostError::Failed(format!(
                "{shown}: git runs with run_git's environment only"
            )));
        }
        let args: Vec<&OsStr> = argv.iter().map(OsStr::new).collect();
        let deadline = Instant::now() + timeout;
        match worktree::run_git_keeping_stdout(self.git.as_os_str(), dir, &args, deadline, max) {
            Ok(output) => Ok(RunOutput {
                success: output.success,
                stdout: output.stdout.into_bytes(),
                stderr: output.stderr,
                cut: None,
            }),
            Err(WorktreeError::GitMissing) => Err(HostError::Missing(format!(
                "git is not installed (looked for {})",
                self.git.display()
            ))),
            Err(WorktreeError::TimedOut { .. }) => Err(timed_out(&shown, timeout)),
            Err(other) => Err(HostError::Failed(other.to_string())),
        }
    }
}

impl Runner for SystemRunner {
    fn run(
        &self,
        program: Program,
        dir: &Path,
        argv: &[String],
        env: &[(String, String)],
        timeout: Duration,
        cap: Capture,
    ) -> Result<RunOutput, HostError> {
        match program {
            Program::Gh => self.run_gh(dir, argv, env, timeout, cap),
            Program::Git => self.run_git(dir, argv, env, timeout, cap),
        }
    }
}

/// `<program> <argv>`, cut to 200 characters, for an error message.
pub(crate) fn shown(program: Program, argv: &[String]) -> String {
    let all = format!("{} {}", program.name(), argv.join(" "));
    match all.char_indices().nth(200) {
        Some((cut, _)) => format!("{}…", &all[..cut]),
        None => all,
    }
}

fn timed_out(shown: &str, timeout: Duration) -> HostError {
    HostError::TimedOut(format!(
        "{shown} did not answer within {} s",
        timeout.as_secs()
    ))
}
