//! Decision 14: `FakeHost`, GitHub scripted, and it panics if asked to land anything.
//!
//! [`FakeHost`] is [`GhHost`] over [`FakeGh`], so a test runs `GhHost`'s real argv, its
//! allow-list and `gh_parse` against `gh`-shaped answers, and only the far end is fake.
//! [`FakeGh`] passes `git` to [`SystemRunner`] unchanged (the remote is a local bare
//! repository the test repository's config maps the GitHub URL onto) and interprets
//! `gh` against `FakeGithub` (`github.rs`), whose state is `<dir>/github.json`; every
//! `gh` argv is appended to `<dir>/calls.jsonl`. Asked to merge, approve or enable
//! auto-merge, it appends the argv to `<dir>/forbidden.jsonl` and panics: the allow-list
//! stops those first, so the panic is the last guard. Any other `gh` argv it does not
//! know exits 1 with `FakeGh: unsupported: <argv>`. It never reaches the network.
//! Blocking: call only from `spawn_blocking`.

mod ctl;
mod gh_api;
mod gh_pr;
mod github;
mod landing;
mod rules;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_ci_merge;
#[cfg(test)]
mod tests_landing;
#[cfg(test)]
mod tests_lifecycle;

use std::path::{Path, PathBuf};
use std::time::Duration;

pub use ctl::{FakeGithubCtl, MergeMethodArg};
pub use github::{
    FAKE_LOGIN, FakeComment, FakePr, FakeReply, FakeRepo, FakeReview, FakeThread, FakeThreadComment,
};
pub use rules::{CiRule, CiRun, FileContains};

use super::runner::{Capture, Cut, Program, RunOutput, Runner, SystemRunner};
use super::{
    CodeHost, DeleteBranchReq, FetchOutcome, FetchReq, GhHost, HostError, HostRepo, LogFile,
    OpenPrReq, PrRef, PrView, PreflightReq, PushOutcome, PushReq, ReplyReq, RepoPermission,
};
use github::FakeGithub;

/// `FakeGh`'s `SystemRunner` runs `git` only; its `gh` path never exists.
const NO_GH: &str = "/nonexistent/anthrex-fake-host/gh";

/// `gh --version` (M9.2.1: the version anthrex was checked against).
const VERSION: &str =
    "gh version 2.92.0 (2026-04-28)\nhttps://github.com/cli/cli/releases/tag/v2.92.0\n";

/// The `CodeHost` of every test that delivers: `GhHost` over [`FakeGh`].
pub struct FakeHost {
    inner: GhHost<FakeGh>,
}

impl FakeHost {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        FakeHost {
            inner: GhHost::new(FakeGh::new(dir), "gh", "git"),
        }
    }

    pub fn dir(&self) -> &Path {
        &self.inner.runner().dir
    }
}

impl CodeHost for FakeHost {
    fn preflight(&self, req: &PreflightReq) -> Result<HostRepo, HostError> {
        self.inner.preflight(req)
    }
    fn push(&self, req: &PushReq) -> Result<PushOutcome, HostError> {
        self.inner.push(req)
    }
    fn fetch(&self, req: &FetchReq) -> Result<FetchOutcome, HostError> {
        self.inner.fetch(req)
    }
    fn open_pr(&self, req: &OpenPrReq) -> Result<PrRef, HostError> {
        self.inner.open_pr(req)
    }
    fn view_pr(&self, repo: &HostRepo, number: u64) -> Result<PrView, HostError> {
        self.inner.view_pr(repo, number)
    }
    fn failed_logs(
        &self,
        repo: &HostRepo,
        ci_run: u64,
        max_bytes: u64,
        out: &Path,
    ) -> Result<LogFile, HostError> {
        self.inner.failed_logs(repo, ci_run, max_bytes, out)
    }
    fn rerun_failed(&self, repo: &HostRepo, ci_run: u64) -> Result<(), HostError> {
        self.inner.rerun_failed(repo, ci_run)
    }
    fn reply(&self, req: &ReplyReq) -> Result<u64, HostError> {
        self.inner.reply(req)
    }
    fn retarget(&self, repo: &HostRepo, number: u64, base: &str) -> Result<(), HostError> {
        self.inner.retarget(repo, number, base)
    }
    fn permission(&self, repo: &HostRepo, user: &str) -> Result<RepoPermission, HostError> {
        self.inner.permission(repo, user)
    }
    fn delete_branch(&self, req: &DeleteBranchReq) -> Result<(), HostError> {
        self.inner.delete_branch(req)
    }
}

/// The `Runner` at the far end of [`FakeHost`].
pub struct FakeGh {
    dir: PathBuf,
    system: SystemRunner,
}

impl FakeGh {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        FakeGh {
            dir: dir.into(),
            system: SystemRunner::new(NO_GH, "git"),
        }
    }

    fn gh(
        &self,
        dir: &Path,
        argv: &[String],
        env: &[(String, String)],
    ) -> Result<Answer, HostError> {
        let args: Vec<&str> = argv.iter().map(String::as_str).collect();
        // The guard comes first, so a landing ask always panics: a failure to record
        // it is named in the panic, never answered as an error.
        if let Some(verb) = landing::landing(&args, dir) {
            let recorded = [github::CALLS, github::FORBIDDEN]
                .into_iter()
                .filter_map(|file| github::append(&self.dir, file, argv).err())
                .collect::<Vec<_>>();
            panic!(
                "FakeHost: anthrex asked to {verb} gh {}; anthrex never lands anything{}",
                argv.join(" "),
                if recorded.is_empty() {
                    String::new()
                } else {
                    format!(" ({})", recorded.join("; "))
                }
            );
        }
        github::append(&self.dir, github::CALLS, argv).map_err(HostError::Failed)?;
        let Some(cmd) = gh_pr::parse(&args).or_else(|| gh_api::parse(&args)) else {
            return Ok(Answer::fail(format!(
                "FakeGh: unsupported: gh {}\n",
                argv.join(" ")
            )));
        };
        let host = env
            .iter()
            .find(|(k, _)| k == "GH_HOST")
            .map_or("github.com", |(_, v)| v.as_str())
            .to_string();
        github::with_state(&self.dir, |state| answer(state, &cmd, &host))
            .and_then(|a| a)
            .map_err(HostError::Failed)
    }
}

impl Runner for FakeGh {
    fn run(
        &self,
        program: Program,
        dir: &Path,
        argv: &[String],
        env: &[(String, String)],
        timeout: Duration,
        cap: Capture,
    ) -> Result<RunOutput, HostError> {
        if program == Program::Git {
            return self.system.run(program, dir, argv, env, timeout, cap);
        }
        let answer = self.gh(dir, argv, env)?;
        let mut out = RunOutput {
            success: answer.ok,
            stdout: answer.stdout.into_bytes(),
            stderr: answer.stderr,
            cut: None,
        };
        match cap {
            Capture::Bytes(max) if out.stdout.len() > max => Err(HostError::Failed(format!(
                "gh {} printed more output than anthrex reads",
                argv.join(" ")
            ))),
            Capture::Bytes(_) => Ok(out),
            // As `subprocess` does: only the two ends are kept.
            Capture::HeadTail { head, tail } => {
                let total = out.stdout.len();
                if total > head + tail {
                    let mut kept = out.stdout[..head].to_vec();
                    kept.extend_from_slice(&out.stdout[total - tail..]);
                    out.stdout = kept;
                    out.cut = Some(Cut {
                        at: head,
                        dropped: (total - head - tail) as u64,
                    });
                }
                Ok(out)
            }
        }
    }
}

/// What `gh` printed and whether it exited 0.
pub(super) struct Answer {
    pub ok: bool,
    pub stdout: String,
    pub stderr: String,
}

impl Answer {
    pub fn out(stdout: String) -> Answer {
        Answer {
            ok: true,
            stdout,
            stderr: String::new(),
        }
    }

    pub fn fail(stderr: String) -> Answer {
        Answer {
            ok: false,
            stdout: String::new(),
            stderr,
        }
    }
}

/// A `gh` command `FakeGh` answers (every row of the brief's "GhHost commands", parsed
/// strictly: an unknown flag or field is "unsupported").
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Cmd {
    Version,
    AuthStatus {
        host: String,
    },
    RepoView {
        repo: String,
    },
    PrList {
        repo: String,
        head: String,
        state: String,
        fields: Vec<String>,
    },
    PrCreate {
        repo: String,
        base: String,
        head: String,
        title: String,
        body_file: String,
    },
    PrView {
        number: u64,
        repo: String,
        fields: Vec<String>,
    },
    PrComment {
        number: u64,
        repo: String,
        body_file: String,
    },
    PrEdit {
        number: u64,
        repo: String,
        base: String,
    },
    RunView {
        id: u64,
        repo: String,
    },
    RunRerun {
        id: u64,
        repo: String,
    },
    Threads {
        owner: String,
        name: String,
        number: u64,
    },
    ListComments {
        repo: String,
        number: u64,
        review: bool,
    },
    Reply {
        repo: String,
        number: u64,
        comment: u64,
        body: String,
    },
    Permission {
        repo: String,
        user: String,
    },
}

/// One `gh` answer: the login and rate-limit gates (`gh --version` asks GitHub
/// nothing), GitHub's reaction to the bare repository, then the command.
fn answer(state: &mut FakeGithub, cmd: &Cmd, host: &str) -> Result<Answer, String> {
    let host = match cmd {
        Cmd::Version => return Ok(Answer::out(VERSION.to_string())),
        Cmd::AuthStatus { host } => host.as_str(),
        _ => host,
    };
    if !state.logged_in.iter().any(|h| h == host) {
        return Ok(Answer::fail(match cmd {
            Cmd::AuthStatus { .. } => format!("You are not logged into any accounts on {host}\n"),
            _ => "To get started with GitHub CLI, please run:  gh auth login\nAlternatively, populate the GH_TOKEN environment variable with a GitHub API authentication token.\n".to_string(),
        }));
    }
    if state.rate_limited > 0 {
        state.rate_limited -= 1;
        // REST answers HTTP 403 with gh's own prefix; GraphQL its own text (M9.2.1
        // check 3, from the documentation).
        let rest = matches!(
            cmd,
            Cmd::RunView { .. }
                | Cmd::RunRerun { .. }
                | Cmd::ListComments { .. }
                | Cmd::Reply { .. }
                | Cmd::Permission { .. }
        );
        return Ok(Answer::fail(if rest {
            "gh: API rate limit exceeded for user ID 1. (HTTP 403)\n".to_string()
        } else {
            "GraphQL: API rate limit exceeded for user ID 1.\n".to_string()
        }));
    }
    state.refresh()?;
    match cmd {
        Cmd::Threads { .. }
        | Cmd::ListComments { .. }
        | Cmd::Reply { .. }
        | Cmd::Permission { .. } => gh_api::answer(state, cmd, host),
        _ => gh_pr::answer(state, cmd, host),
    }
}
