//! Task M9.2.12: decision 3's detection. A proposal carries `[delivery] mode = "pr"`
//! only with a GitHub `origin`, an installed `gh` logged in to its host, and a
//! repository it can see; otherwise no table. `FakeHost` over a local bare repository,
//! and a `gh` path that does not exist: never a real `gh`, never GitHub.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use proto::{DeliveryMode, DeliveryProfile};

use super::*;
use crate::host::fake::{FakeGithubCtl, FakeHost};
use crate::host::select::{self, CodeHostChoice};
use crate::manager::TEST_GH_BIN;

const URL: &str = "https://github.com/fake/app.git";

fn git(dir: &Path, args: &[&str]) -> String {
    let os: Vec<&OsStr> = args.iter().map(OsStr::new).collect();
    let deadline = Instant::now() + Duration::from_secs(30);
    let out = crate::worktree::run_git(OsStr::new("git"), dir, &os, deadline).unwrap();
    assert!(out.success, "git {args:?}: {}", out.stderr);
    out.stdout.trim().to_string()
}

/// A repository whose `origin` is `url`, mapped onto a local bare repository, and the
/// fake GitHub's control (nothing known to it yet).
fn repo(tmp: &Path, url: &str) -> (PathBuf, PathBuf, FakeGithubCtl) {
    let (work, bare) = (tmp.join("work"), tmp.join("remote.git"));
    std::fs::create_dir_all(&work).unwrap();
    std::fs::create_dir_all(&bare).unwrap();
    git(&bare, &["init", "-q", "--bare", "-b", "main"]);
    git(&work, &["init", "-q", "-b", "main"]);
    git(&work, &["config", "remote.origin.url", url]);
    let bare_s = bare.to_string_lossy().into_owned();
    for key in ["insteadOf", "pushInsteadOf"] {
        git(&work, &["config", &format!("url.{bare_s}.{key}"), url]);
    }
    (work, bare, FakeGithubCtl::open(&tmp.join("github")))
}

#[test]
fn detection_proposes_pr_only_with_a_github_remote_and_a_logged_in_gh() {
    let pr = Some(DeliveryProfile {
        mode: DeliveryMode::Pr,
        remote: "origin".into(),
    });
    // A GitHub remote, gh logged in, the repository visible: `pr` on `origin`.
    let tmp = tempfile::tempdir().unwrap();
    let (work, bare, ctl) = repo(tmp.path(), URL);
    ctl.create_repo("fake", "app", &bare, "main");
    ctl.log_in("github.com");
    let host = FakeHost::new(ctl.dir());
    assert_eq!(detect(&work, &host), pr);
    // Detection never pushes, not even a dry run: the bare repository has no ref.
    assert_eq!(git(&bare, &["for-each-ref"]), "");
    let asked: Vec<String> = ctl.calls().into_iter().map(|c| c.join(" ")).collect();
    assert_eq!(
        asked,
        [
            "--version",
            "auth status --hostname github.com",
            "repo view fake/app --json nameWithOwner"
        ]
    );

    // Logged out: no table.
    let tmp = tempfile::tempdir().unwrap();
    let (work, bare, ctl) = repo(tmp.path(), URL);
    ctl.create_repo("fake", "app", &bare, "main");
    assert_eq!(detect(&work, &FakeHost::new(ctl.dir())), None);

    // `gh` not installed (a path that does not exist): no table.
    let missing = select::build(&CodeHostChoice::Gh {
        bin: TEST_GH_BIN.into(),
    });
    assert_eq!(detect(&work, &*missing), None);

    // A remote that is not GitHub, even with gh logged in: no table.
    let tmp = tempfile::tempdir().unwrap();
    let (work, bare, ctl) = repo(tmp.path(), "https://gitlab.com/fake/app.git");
    ctl.create_repo("fake", "app", &bare, "main");
    ctl.log_in("github.com");
    assert_eq!(detect(&work, &FakeHost::new(ctl.dir())), None);
    // (Any host may be GitHub Enterprise: gh is asked whether it is logged in there.)
    let asked: Vec<String> = ctl.calls().into_iter().map(|c| c.join(" ")).collect();
    assert_eq!(asked, ["--version", "auth status --hostname gitlab.com"]);

    // No `origin` at all: no table.
    git(&work, &["config", "--unset", "remote.origin.url"]);
    assert_eq!(detect(&work, &FakeHost::new(ctl.dir())), None);
}

/// The service's detection runs off the async threads, and with no host (a profile
/// service the daemon never wired) proposes nothing.
#[tokio::test]
async fn detected_needs_a_host_and_runs_on_a_blocking_thread() {
    let tmp = tempfile::tempdir().unwrap();
    let (work, bare, ctl) = repo(tmp.path(), URL);
    ctl.create_repo("fake", "app", &bare, "main");
    ctl.log_in("github.com");
    assert_eq!(detected(None, work.clone()).await, None);
    let host: Arc<dyn crate::host::CodeHost> = Arc::new(FakeHost::new(ctl.dir()));
    assert_eq!(
        detected(Some(host), work).await.map(|t| t.mode),
        Some(DeliveryMode::Pr)
    );
}
