//! Task M9.2.5: `FakeHost`, the scripted GitHub. `GhHost<FakeGh>` runs its real argv,
//! its allow-list and `gh_parse` against `FakeGh`'s `gh`-shaped answers; `git` is real,
//! and the only remote is a local bare repository that the test repository's own
//! config maps the GitHub URL onto (`url.<bare>.insteadOf`), so no test here opens a
//! network connection. `tests_ci_merge.rs` and `tests_lifecycle.rs` share the rig.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::*;
use crate::host::{Capture, Program, RunOutput, Runner};
use crate::host::{
    CodeHost, HostError, HostRepo, OpenPrReq, PrRef, PrState, PushOutcome, PushReq, ReplyReq,
    ReplyTarget, RepoPermission,
};
use crate::worktree::run_git;

pub(super) const OWNER: &str = "anthrex-test";
pub(super) const NAME: &str = "widgets";
pub(super) const FULL: &str = "anthrex-test/widgets";
pub(super) const URL: &str = "https://github.com/anthrex-test/widgets.git";
pub(super) const RUN: &str = "r1a2b";

pub(super) fn strings(args: &[&str]) -> Vec<String> {
    args.iter().map(|a| a.to_string()).collect()
}

/// Test setup's git, through `worktree::run_git` like the code under test, with an
/// identity and signing given per command so the user's global config cannot change a
/// commit.
pub(super) fn git(dir: &Path, args: &[&str]) -> String {
    let mut all = vec![
        "-c",
        "user.name=anthrex test",
        "-c",
        "user.email=test@anthrex.invalid",
        "-c",
        "commit.gpgSign=false",
        "-c",
        "core.hooksPath=/dev/null",
    ];
    all.extend_from_slice(args);
    let os: Vec<&OsStr> = all.iter().map(OsStr::new).collect();
    let out = run_git(
        OsStr::new("git"),
        dir,
        &os,
        Instant::now() + Duration::from_secs(30),
    )
    .unwrap();
    assert!(out.success, "git {args:?}: {}", out.stderr);
    out.stdout.trim().to_string()
}

pub(super) struct Rig {
    _tmp: tempfile::TempDir,
    pub bare: PathBuf,
    pub work: PathBuf,
    pub dir: PathBuf,
    pub host: FakeHost,
    pub ctl: FakeGithubCtl,
    pub base: String,
}

impl Rig {
    pub fn new() -> Rig {
        let tmp = tempfile::tempdir().unwrap();
        let bare = tmp.path().join("remote.git");
        let work = tmp.path().join("work");
        let dir = tmp.path().join("fake-github");
        std::fs::create_dir_all(&bare).unwrap();
        std::fs::create_dir_all(&work).unwrap();
        git(&bare, &["init", "-q", "--bare", "-b", "main"]);
        git(&work, &["init", "-q", "-b", "main"]);
        let bare_s = bare.to_string_lossy().into_owned();
        git(&work, &["config", "remote.origin.url", URL]);
        git(&work, &["config", &format!("url.{bare_s}.insteadOf"), URL]);
        git(
            &work,
            &["config", &format!("url.{bare_s}.pushInsteadOf"), URL],
        );
        // Before anything talks to the remote: both directions reach the bare
        // repository, never GitHub.
        assert_eq!(git(&work, &["remote", "get-url", "origin"]), bare_s);
        assert_eq!(
            git(&work, &["remote", "get-url", "--push", "origin"]),
            bare_s
        );
        std::fs::write(work.join("README.md"), "# widgets\n\nline two\n").unwrap();
        git(&work, &["add", "-A"]);
        git(&work, &["commit", "-q", "-m", "base"]);
        let base = git(&work, &["rev-parse", "HEAD"]);
        git(&work, &["push", "-q", "origin", "main"]);
        let ctl = FakeGithubCtl::open(&dir);
        ctl.create_repo(OWNER, NAME, &bare, "main");
        ctl.log_in("github.com");
        Rig {
            host: FakeHost::new(&dir),
            ctl,
            _tmp: tmp,
            bare,
            work,
            dir,
            base,
        }
    }

    pub fn repo(&self) -> HostRepo {
        HostRepo {
            host: "github.com".to_string(),
            owner: OWNER.to_string(),
            name: NAME.to_string(),
            remote: "origin".to_string(),
            root: self.work.clone(),
        }
    }

    /// A local commit on `from` writing `files` (`None` deletes), on a scratch branch.
    pub fn commit(&self, from: &str, files: &[(&str, Option<&str>)], message: &str) -> String {
        git(&self.work, &["checkout", "-q", "--detach", from]);
        for (path, content) in files {
            let at = self.work.join(path);
            match content {
                Some(text) => {
                    std::fs::create_dir_all(at.parent().unwrap()).unwrap();
                    std::fs::write(&at, text).unwrap();
                }
                None => std::fs::remove_file(&at).unwrap(),
            }
        }
        git(&self.work, &["add", "-A"]);
        git(&self.work, &["commit", "-q", "-m", message]);
        git(&self.work, &["rev-parse", "HEAD"])
    }

    pub fn push(&self, stage: u16, sha: &str) -> PushOutcome {
        self.host
            .push(&PushReq {
                repo: self.repo(),
                run_id: RUN.to_string(),
                stage,
                sha: sha.to_string(),
            })
            .unwrap()
    }

    pub fn open(&self, stage: u16, base: &str, title: &str) -> PrRef {
        let body = self.work.join(format!("../pr-{stage}.md"));
        std::fs::write(&body, format!("Body of stage {stage}.\n")).unwrap();
        self.host
            .open_pr(&OpenPrReq {
                repo: self.repo(),
                run_id: RUN.to_string(),
                base: base.to_string(),
                head: head(stage),
                title: title.to_string(),
                body_file: body,
            })
            .unwrap()
    }

    /// The bare repository's `refs/heads/<branch>`, or `None`.
    pub fn remote(&self, branch: &str) -> Option<String> {
        let refname = format!("refs/heads/{branch}");
        let args = ["rev-parse", "--verify", "--quiet", &refname];
        let os: Vec<&OsStr> = args.iter().map(OsStr::new).collect();
        let out = run_git(
            OsStr::new("git"),
            &self.bare,
            &os,
            Instant::now() + Duration::from_secs(30),
        )
        .unwrap();
        out.success.then(|| out.stdout.trim().to_string())
    }

    pub fn gh(&self) -> FakeGh {
        FakeGh::new(&self.dir)
    }
}

pub(super) fn head(stage: u16) -> String {
    format!("anthrex/{RUN}/stage-{stage}")
}

pub(super) fn run_gh(gh: &FakeGh, args: &[&str]) -> RunOutput {
    gh.run(
        Program::Gh,
        Path::new("/"),
        &strings(args),
        &[("GH_HOST".to_string(), "github.com".to_string())],
        Duration::from_secs(5),
        Capture::Bytes(1 << 20),
    )
    .unwrap()
}

#[test]
fn fake_gh_refuses_unknown_commands() {
    let rig = Rig::new();
    let gh = rig.gh();
    let unknown: [&[&str]; 9] = [
        &["pr", "close", "1", "--repo", FULL],
        &["pr", "ready", "1", "--repo", FULL],
        &["issue", "list", "--repo", FULL],
        &["repo", "delete", FULL, "--yes"],
        &[
            "pr",
            "view",
            "1",
            "--repo",
            FULL,
            "--json",
            "state,notAField",
        ],
        &["api", "repos/anthrex-test/widgets/pulls/1"],
        &["api", "graphql", "-f", "query={ viewer { login } }"],
        // A read of a ref; a write to one is a landing (`tests_guard.rs`).
        &["api", "repos/anthrex-test/widgets/git/refs/heads/main"],
        &["pr", "edit", "1", "--repo", FULL, "--title", "other"],
    ];
    for args in unknown {
        let out = run_gh(&gh, args);
        assert!(!out.success, "{args:?}");
        assert_eq!(
            out.stderr.trim_end(),
            format!("FakeGh: unsupported: gh {}", args.join(" ")),
            "{args:?}"
        );
        assert!(out.stdout.is_empty(), "{args:?}");
    }
    assert!(rig.ctl.forbidden().is_empty());
    assert!(!rig.dir.join("forbidden.jsonl").exists());
    assert_eq!(rig.ctl.calls().len(), unknown.len());
}

#[test]
fn fake_github_state_survives_a_reopen() {
    let rig = Rig::new();
    let sha = rig.commit(&rig.base, &[("a.txt", Some("a\n"))], "stage 1");
    rig.push(1, &sha);
    rig.open(1, "main", "Stage 1");
    let said = rig.ctl.comment(1, "alice", "Looks risky.");
    let bot = rig
        .ctl
        .comment(1, "github-actions[bot]", "Thanks for the PR!");
    let replied = rig
        .host
        .reply(&ReplyReq {
            repo: rig.repo(),
            number: 1,
            target: ReplyTarget::Conversation,
            body: "It is covered by a test.".to_string(),
            marker: format!("<!-- anthrex:reply {RUN} 1:c{said} {} -->", &sha[..7]),
        })
        .unwrap();
    rig.ctl.set_permission("alice", RepoPermission::Maintain);

    let reopened = FakeGithubCtl::open(&rig.dir);
    let prs = reopened.prs();
    assert_eq!(prs.len(), 1);
    let pr = &prs[0];
    assert_eq!(
        (pr.number, pr.base.as_str(), pr.head.as_str()),
        (1, "main", head(1).as_str())
    );
    assert_eq!(pr.title, "Stage 1");
    assert_eq!(pr.body, "Body of stage 1.\n");
    assert_eq!(pr.state, PrState::Open);
    let ids: Vec<u64> = pr.comments.iter().map(|c| c.id).collect();
    assert_eq!(ids, vec![said, bot, replied]);
    assert_eq!(pr.comments[0].user, "alice");
    assert_eq!(pr.replies.len(), 1);
    assert_eq!(pr.replies[0].thread, None);
    // Ids keep counting from where the first handle left them.
    let later = reopened.comment(1, "bob", "One more.");
    assert!(later > replied);
    // GitHub's GraphQL shows a bot without `[bot]`, typed `Bot` (ruling R-3).
    let view = FakeHost::new(&rig.dir).view_pr(&rig.repo(), 1).unwrap();
    let bot_seen = view.comments.iter().find(|c| c.id == bot).unwrap();
    assert_eq!(bot_seen.author.login, "github-actions");
    assert!(bot_seen.author.bot);
    assert_eq!(
        FakeHost::new(&rig.dir).permission(&rig.repo(), "alice"),
        Ok(RepoPermission::Maintain)
    );
    assert_eq!(
        FakeHost::new(&rig.dir).permission(&rig.repo(), "mallory"),
        Ok(RepoPermission::None)
    );
}

#[test]
fn fake_rate_limit_fails_the_next_calls_with_githubs_text() {
    let rig = Rig::new();
    let sha = rig.commit(&rig.base, &[("a.txt", Some("a\n"))], "stage 1");
    rig.push(1, &sha);
    rig.open(1, "main", "Stage 1");
    rig.ctl.rate_limit(2);
    // `gh --version` asks GitHub nothing, so it does not use up a limited call.
    assert!(run_gh(&rig.gh(), &["--version"]).success);
    assert_eq!(
        rig.host.view_pr(&rig.repo(), 1),
        Err(HostError::RateLimited(
            "GraphQL: API rate limit exceeded for user ID 1.".to_string()
        ))
    );
    assert_eq!(
        rig.host.permission(&rig.repo(), "alice"),
        Err(HostError::RateLimited(
            "gh: API rate limit exceeded for user ID 1. (HTTP 403)".to_string()
        ))
    );
    let view = rig.host.view_pr(&rig.repo(), 1).unwrap();
    assert_eq!(view.head_oid, sha);
}

/// Decision 14: the test process and the daemon share `github.json` under a blocking
/// `flock`. Two handles changing it at once lose nothing.
#[test]
fn fake_github_changes_are_serialized_by_the_lock() {
    let rig = Rig::new();
    let sha = rig.commit(&rig.base, &[("a.txt", Some("a\n"))], "stage 1");
    rig.push(1, &sha);
    rig.open(1, "main", "Stage 1");
    let ids: Vec<u64> = std::thread::scope(|scope| {
        let workers: Vec<_> = ["alice", "bob"]
            .into_iter()
            .map(|user| {
                let dir = rig.dir.clone();
                scope.spawn(move || {
                    let ctl = FakeGithubCtl::open(&dir);
                    (0..25)
                        .map(|i| ctl.comment(1, user, &format!("{user} {i}")))
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        workers
            .into_iter()
            .flat_map(|w| w.join().unwrap())
            .collect()
    });
    let mut distinct = ids.clone();
    distinct.sort_unstable();
    distinct.dedup();
    assert_eq!(distinct.len(), 50);
    assert_eq!(rig.ctl.prs()[0].comments.len(), 50);
}

/// Fix wave A1 (review A, M1): a re-issued `OpenPr` adopts the open PR it finds for the
/// head and answers the base GitHub reports for it, not the base it asked for, so the
/// engine can retarget a PR whose create timed out after it succeeded.
#[test]
fn an_adopted_pr_reports_the_base_github_has() {
    let rig = Rig::new();
    let one = rig.commit(&rig.base, &[("one.txt", Some("1\n"))], "stage 1");
    let two = rig.commit(&one, &[("two.txt", Some("2\n"))], "stage 2");
    assert_eq!(rig.push(1, &one), PushOutcome::Pushed);
    assert_eq!(rig.push(2, &two), PushOutcome::Pushed);
    let stage_1 = format!("anthrex/{RUN}/stage-1");
    let made = rig.open(2, &stage_1, "Stage 2: two");
    assert_eq!((made.existed, made.base), (false, None));

    // The retry asks for `main` (stage 1 merged meanwhile); GitHub still has stage 1.
    let again = rig.open(2, "main", "Stage 2: two");
    assert_eq!((again.number, again.existed), (made.number, true));
    assert_eq!(again.base.as_deref(), Some(stage_1.as_str()));
}

/// Fix wave A1: a journal line written before `PrRef.base` existed still decodes, and a
/// created PR's line carries no `base` key.
#[test]
fn a_pr_ref_without_a_base_round_trips() {
    let created = PrRef {
        number: 3,
        url: "u".to_string(),
        state: PrState::Open,
        existed: false,
        base: None,
    };
    let line = serde_json::to_string(&created).unwrap();
    assert!(!line.contains("base"), "{line}");
    assert_eq!(serde_json::from_str::<PrRef>(&line).unwrap(), created);
    let adopted = PrRef {
        existed: true,
        base: Some("main".to_string()),
        ..created
    };
    let line = serde_json::to_string(&adopted).unwrap();
    assert_eq!(serde_json::from_str::<PrRef>(&line).unwrap(), adopted);
}
