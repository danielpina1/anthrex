//! Decision 6: [`GhHost`], the [`CodeHost`] that runs the user's own `gh` and `git`. Every
//! argv is exactly a row of the brief's "GhHost commands" (with rulings R-2 and R-10),
//! and every one passes [`allow::check`] before the [`Runner`] sees it. Blocking: call
//! only from `spawn_blocking`. Preflight is in `gh_preflight.rs`, the `git` side (push,
//! fetch, adoption, branch delete) in `gh_git.rs`.

use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use super::allow::{self, AllowCtx};
use super::gh_parse::{self, last_line};
use super::runner::{Capture, Program, RunOutput, Runner};
use super::{
    CodeHost, DeleteBranchReq, FetchOutcome, FetchReq, HOST_READ_TIMEOUT, HOST_WRITE_TIMEOUT,
    HostError, HostRepo, LOG_TIMEOUT, LogFile, OpenPrReq, PrRef, PrState, PrView, PreflightReq,
    PushOutcome, PushReq, ReplyReq, ReplyTarget, RepoPermission,
};

/// Decision 7 and ruling R-2: the one GraphQL call anthrex makes, a query (never a
/// mutation) reading the review threads, the reviews and the conversation comments,
/// each with `fullDatabaseId` (a string; `databaseId` is deprecated) and the author's
/// `__typename` (ruling R-3).
pub const THREADS_QUERY: &str = "query($owner: String!, $name: String!, $number: Int!) { repository(owner: $owner, name: $name) { pullRequest(number: $number) { reviewThreads(last: 100) { nodes { isResolved path line comments(first: 50) { nodes { fullDatabaseId body diffHunk author { __typename login } } } } } reviews(last: 100) { nodes { fullDatabaseId state body author { __typename login } } } comments(last: 100) { nodes { fullDatabaseId body author { __typename login } } } } } }";

/// `gh pr view`'s fields: state, checks and mergeability (ruling R-2 moved reviews and
/// comments to [`THREADS_QUERY`]).
pub const PR_VIEW_FIELDS: &str =
    "state,mergedAt,mergeCommit,baseRefName,headRefOid,mergeable,reviewDecision,statusCheckRollup";
/// Ruling R-10: the owner fields keep a fork's PR of the same branch name out.
pub const PR_LIST_FIELDS: &str =
    "number,url,state,baseRefName,headRepositoryOwner,isCrossRepository";

/// The oldest `gh` anthrex was checked against (M9.2.1: gh 2.92.0).
pub const MIN_GH_VERSION: (u32, u32, u32) = (2, 92, 0);

/// Decision 27 step 1: the head of a failed log that is always kept.
pub const LOG_HEAD_BYTES: u64 = 64 * 1024;
/// Room kept under the cap for the line that marks the cut.
const CUT_LINE_MAX: u64 = 64;

pub(super) const GH_OUTPUT_MAX: usize = 32 * 1024 * 1024;
const GIT_OUTPUT_MAX: usize = 1024 * 1024;

/// Decision 12's variables, beside `GH_HOST`.
const GH_ENV: [(&str, &str); 8] = [
    ("GH_PROMPT_DISABLED", "1"),
    ("GH_NO_UPDATE_NOTIFIER", "1"),
    ("GH_NO_EXTENSION_UPDATE_NOTIFIER", "1"),
    ("GH_SPINNER_DISABLED", "1"),
    ("GH_PAGER", "cat"),
    ("NO_COLOR", "1"),
    ("CLICOLOR", "0"),
    // AGENTS.md rule 11: a git that gh runs in the checkout takes no optional lock.
    ("GIT_OPTIONAL_LOCKS", "0"),
];

pub struct GhHost<R: Runner> {
    runner: R,
    pub(super) gh: PathBuf,
    pub(super) git: PathBuf,
}

impl<R: Runner> GhHost<R> {
    pub fn new(runner: R, gh: impl Into<PathBuf>, git: impl Into<PathBuf>) -> Self {
        GhHost {
            runner,
            gh: gh.into(),
            git: git.into(),
        }
    }

    pub fn runner(&self) -> &R {
        &self.runner
    }

    pub(super) fn gh(
        &self,
        host: &str,
        ctx: &AllowCtx<'_>,
        dir: &Path,
        argv: Vec<String>,
        timeout: Duration,
        cap: Capture,
    ) -> Result<RunOutput, HostError> {
        allow::check(Program::Gh, &argv, ctx)?;
        let mut env = vec![("GH_HOST".to_string(), host.to_string())];
        env.extend(GH_ENV.iter().map(|(k, v)| (k.to_string(), v.to_string())));
        self.runner.run(Program::Gh, dir, &argv, &env, timeout, cap)
    }

    /// A `gh` command that must succeed; a failure is classified (decision 11).
    pub(super) fn gh_ok(
        &self,
        repo: &HostRepo,
        ctx: &AllowCtx<'_>,
        argv: Vec<String>,
        timeout: Duration,
    ) -> Result<RunOutput, HostError> {
        let out = self.gh(
            &repo.host,
            ctx,
            &repo.root,
            argv,
            timeout,
            Capture::Bytes(GH_OUTPUT_MAX),
        )?;
        if out.success {
            Ok(out)
        } else {
            Err(gh_parse::classify(&out.stderr))
        }
    }

    pub(super) fn git(
        &self,
        ctx: &AllowCtx<'_>,
        dir: &Path,
        flags: &[&str],
        args: &[&str],
        timeout: Duration,
    ) -> Result<RunOutput, HostError> {
        let argv: Vec<String> = flags.iter().chain(args).map(|a| a.to_string()).collect();
        allow::check(Program::Git, &argv, ctx)?;
        self.runner.run(
            Program::Git,
            dir,
            &argv,
            &[],
            timeout,
            Capture::Bytes(GIT_OUTPUT_MAX),
        )
    }
}

impl<R: Runner> CodeHost for GhHost<R> {
    fn preflight(&self, req: &PreflightReq) -> Result<HostRepo, HostError> {
        self.check_preflight(req)
    }

    fn push(&self, req: &PushReq) -> Result<PushOutcome, HostError> {
        self.push_stage(req)
    }

    fn fetch(&self, req: &FetchReq) -> Result<FetchOutcome, HostError> {
        self.fetch_ref(req)
    }

    fn open_pr(&self, req: &OpenPrReq) -> Result<PrRef, HostError> {
        let repo = &req.repo;
        let full = repo.full();
        let ctx = AllowCtx {
            repo: Some(&full),
            ..run_ctx(repo, Some(&req.run_id))
        };
        let listed = self.gh_ok(
            repo,
            &ctx,
            strings(&[
                "pr",
                "list",
                "--repo",
                &full,
                "--head",
                &req.head,
                "--state",
                "all",
                "--json",
                PR_LIST_FIELDS,
            ]),
            HOST_READ_TIMEOUT,
        )?;
        let ours: Vec<_> = gh_parse::pr_list(&listed.stdout_text())?
            .into_iter()
            .filter(|pr| pr.owner.eq_ignore_ascii_case(&repo.owner) && !pr.cross_repository)
            .collect();
        let existing = ours
            .iter()
            .filter(|pr| pr.state == PrState::Open)
            .max_by_key(|pr| pr.number)
            .or_else(|| ours.iter().max_by_key(|pr| pr.number));
        if let Some(pr) = existing {
            return Ok(PrRef {
                number: pr.number,
                url: pr.url.clone(),
                state: pr.state,
                existed: true,
            });
        }
        let body_file = req.body_file.to_string_lossy();
        let created = self.gh_ok(
            repo,
            &ctx,
            strings(&[
                "pr",
                "create",
                "--repo",
                &full,
                "--base",
                &req.base,
                "--head",
                &req.head,
                "--title",
                &req.title,
                "--body-file",
                &body_file,
            ]),
            HOST_WRITE_TIMEOUT,
        )?;
        let url = last_line(&created.stdout_text());
        let number = gh_parse::pr_url_number(&url)
            .ok_or_else(|| HostError::Rejected("gh output changed: pr create url".to_string()))?;
        Ok(PrRef {
            number,
            url,
            state: PrState::Open,
            existed: false,
        })
    }

    fn view_pr(&self, repo: &HostRepo, number: u64) -> Result<PrView, HostError> {
        let full = repo.full();
        let ctx = AllowCtx {
            repo: Some(&full),
            ..run_ctx(repo, None)
        };
        let n = number.to_string();
        let view = self.gh_ok(
            repo,
            &ctx,
            strings(&["pr", "view", &n, "--repo", &full, "--json", PR_VIEW_FIELDS]),
            HOST_READ_TIMEOUT,
        )?;
        let threads = self.gh_ok(
            repo,
            &ctx,
            strings(&[
                "api",
                "graphql",
                "-f",
                &format!("query={THREADS_QUERY}"),
                "-f",
                &format!("owner={}", repo.owner),
                "-f",
                &format!("name={}", repo.name),
                "-F",
                &format!("number={number}"),
            ]),
            HOST_READ_TIMEOUT,
        )?;
        gh_parse::pr_view(number, &view.stdout_text(), &threads.stdout_text())
    }

    fn failed_logs(
        &self,
        repo: &HostRepo,
        ci_run: u64,
        max_bytes: u64,
        out: &Path,
    ) -> Result<LogFile, HostError> {
        let full = repo.full();
        let ctx = AllowCtx {
            repo: Some(&full),
            ..run_ctx(repo, None)
        };
        let budget = max_bytes.saturating_sub(CUT_LINE_MAX);
        let head = LOG_HEAD_BYTES.min(budget / 2);
        let tail = budget - head;
        let id = ci_run.to_string();
        let ran = self.gh(
            &repo.host,
            &ctx,
            &repo.root,
            strings(&["run", "view", &id, "--repo", &full, "--log-failed"]),
            LOG_TIMEOUT,
            Capture::HeadTail {
                head: head as usize,
                tail: tail as usize,
            },
        )?;
        if !ran.success {
            return Err(gh_parse::classify(&ran.stderr));
        }
        let mut bytes = ran.stdout.clone();
        if let Some(cut) = ran.cut {
            let line = format!("\n… {} bytes of the log cut here …\n", cut.dropped);
            bytes.splice(cut.at..cut.at, line.into_bytes());
        }
        write_private(out, &bytes)
            .map_err(|e| HostError::Failed(format!("cannot write {}: {e}", out.display())))?;
        let from = bytes
            .len()
            .saturating_sub(crate::decider::CI_SUMMARY_INPUT_BYTES);
        Ok(LogFile {
            path: out.to_path_buf(),
            bytes: bytes.len() as u64,
            truncated: ran.cut.is_some(),
            tail: String::from_utf8_lossy(&bytes[from..]).into_owned(),
        })
    }

    fn rerun_failed(&self, repo: &HostRepo, ci_run: u64) -> Result<(), HostError> {
        let full = repo.full();
        let ctx = AllowCtx {
            repo: Some(&full),
            ..run_ctx(repo, None)
        };
        let id = ci_run.to_string();
        let argv = strings(&["run", "rerun", &id, "--repo", &full, "--failed"]);
        match self.gh_ok(repo, &ctx, argv, HOST_WRITE_TIMEOUT) {
            Ok(_) => Ok(()),
            // Decision 10: a run GitHub is already re-running needs nothing more.
            Err(e) if e.text().contains("already running") => Ok(()),
            Err(e) => Err(e),
        }
    }

    fn reply(&self, req: &ReplyReq) -> Result<u64, HostError> {
        let repo = &req.repo;
        let full = repo.full();
        let ctx = AllowCtx {
            repo: Some(&full),
            ..run_ctx(repo, None)
        };
        let n = req.number;
        let listing = match req.target {
            ReplyTarget::Thread { .. } => format!("repos/{full}/pulls/{n}/comments"),
            ReplyTarget::Conversation => format!("repos/{full}/issues/{n}/comments"),
        };
        // Only the user's own comment is a reply anthrex posted (the fix round's ruling).
        let user = self.gh_ok(repo, &ctx, strings(&["api", "user"]), HOST_READ_TIMEOUT)?;
        let me = gh_parse::user_login(&user.stdout_text())?;
        let seen = self.gh_ok(
            repo,
            &ctx,
            strings(&["api", &listing, "--paginate"]),
            HOST_READ_TIMEOUT,
        )?;
        if let Some(id) = gh_parse::find_marker(&seen.stdout_text(), &req.marker, &me)? {
            return Ok(id);
        }
        let text = format!("{}\n\n{}", req.body, req.marker);
        match req.target {
            ReplyTarget::Thread { comment_id } => {
                let path = format!("repos/{full}/pulls/{n}/comments/{comment_id}/replies");
                let posted = self.gh_ok(
                    repo,
                    &ctx,
                    strings(&["api", "-X", "POST", &path, "-f", &format!("body={text}")]),
                    HOST_WRITE_TIMEOUT,
                )?;
                gh_parse::created_comment_id(&posted.stdout_text())
            }
            ReplyTarget::Conversation => {
                let file = BodyFile::write(&text)
                    .map_err(|e| HostError::Failed(format!("cannot write a reply body: {e}")))?;
                let path = file.0.to_string_lossy().into_owned();
                let posted = self.gh_ok(
                    repo,
                    &ctx,
                    strings(&[
                        "pr",
                        "comment",
                        &n.to_string(),
                        "--repo",
                        &full,
                        "--body-file",
                        &path,
                    ]),
                    HOST_WRITE_TIMEOUT,
                )?;
                let url = last_line(&posted.stdout_text());
                gh_parse::comment_url_id(&url).ok_or_else(|| {
                    HostError::Rejected("gh output changed: pr comment url".to_string())
                })
            }
        }
    }

    fn retarget(&self, repo: &HostRepo, number: u64, base: &str) -> Result<(), HostError> {
        let full = repo.full();
        let ctx = AllowCtx {
            repo: Some(&full),
            ..run_ctx(repo, None)
        };
        let n = number.to_string();
        let argv = strings(&["pr", "edit", &n, "--repo", &full, "--base", base]);
        self.gh_ok(repo, &ctx, argv, HOST_WRITE_TIMEOUT).map(|_| ())
    }

    fn permission(&self, repo: &HostRepo, user: &str) -> Result<RepoPermission, HostError> {
        let full = repo.full();
        let ctx = AllowCtx {
            repo: Some(&full),
            ..run_ctx(repo, None)
        };
        let path = format!("repos/{full}/collaborators/{user}/permission");
        match self.gh_ok(repo, &ctx, strings(&["api", &path]), HOST_READ_TIMEOUT) {
            Ok(out) => gh_parse::permission(&out.stdout_text()),
            // Decision 29: not a collaborator.
            Err(HostError::NotFound(_)) => Ok(RepoPermission::None),
            Err(e) => Err(e),
        }
    }

    fn delete_branch(&self, req: &DeleteBranchReq) -> Result<(), HostError> {
        self.delete_stage(req)
    }
}

pub(super) fn run_ctx<'a>(repo: &'a HostRepo, run_id: Option<&'a str>) -> AllowCtx<'a> {
    AllowCtx {
        run_id,
        remote: &repo.remote,
        base_branch: None,
        repo: None,
    }
}

pub(super) fn strings(args: &[&str]) -> Vec<String> {
    args.iter().map(|a| a.to_string()).collect()
}

/// Writes `bytes` to `path` with mode 0600 (decision 8), replacing what was there.
fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

/// A conversation reply's `--body-file`, private to the daemon and removed on drop.
struct BodyFile(PathBuf);

impl BodyFile {
    fn write(text: &str) -> std::io::Result<BodyFile> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let name = format!(
            "anthrex-reply-{}-{}.md",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        let path = std::env::temp_dir().join(name);
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)?;
        let file_path = BodyFile(path);
        file.write_all(text.as_bytes())?;
        Ok(file_path)
    }
}

impl Drop for BodyFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}
