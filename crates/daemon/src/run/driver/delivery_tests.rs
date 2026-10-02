//! Task M9.2.12: decision 17's preflight from the driver, against `FakeHost` (`GhHost`
//! over `FakeGh`: the real argv, allow-list and parsing; a local bare repository as the
//! remote; no network, no `gh`), and decision 3's resolution of the mode. The start
//! paths are in `delivery_tests_start.rs`.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use proto::{DeliveryMode, DeliveryProfile};

use super::super::host_ops::tests::Sleepy;
use super::*;
use crate::host::fake::{FakeGithubCtl, FakeHost};
use crate::host::scripted::ScriptedRunner;
use crate::host::select::{self, CodeHostChoice};
use crate::host::{GhHost, HostError};
use crate::manager::TEST_GH_BIN;

pub(in crate::run::driver) const URL: &str = "https://github.com/fake/app.git";

pub(in crate::run::driver) fn git(dir: &Path, args: &[&str]) -> String {
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
    let deadline = Instant::now() + Duration::from_secs(30);
    let out = crate::worktree::run_git(OsStr::new("git"), dir, &os, deadline).unwrap();
    assert!(out.success, "git {args:?}: {}", out.stderr);
    out.stdout.trim().to_string()
}

/// A repository whose `origin` is a GitHub URL that its own config maps onto a local
/// bare repository (fetch and push), and the fake GitHub's directory. Nothing is known
/// to the fake GitHub until the test says so.
pub(in crate::run::driver) struct Rig {
    pub tmp: tempfile::TempDir,
    pub work: PathBuf,
    pub bare: PathBuf,
    pub ctl: FakeGithubCtl,
    pub base: String,
}

impl Rig {
    pub(in crate::run::driver) fn new(push_base: bool) -> Rig {
        let tmp = tempfile::tempdir().unwrap();
        let (bare, work) = (tmp.path().join("remote.git"), tmp.path().join("work"));
        std::fs::create_dir_all(&bare).unwrap();
        std::fs::create_dir_all(work.join("crates/a/src")).unwrap();
        git(&bare, &["init", "-q", "--bare", "-b", "main"]);
        git(&work, &["init", "-q", "-b", "main"]);
        git(&work, &["config", "user.name", "t"]);
        git(&work, &["config", "user.email", "t@t"]);
        git(&work, &["config", "remote.origin.url", URL]);
        let bare_s = bare.to_string_lossy().into_owned();
        for key in ["insteadOf", "pushInsteadOf"] {
            git(&work, &["config", &format!("url.{bare_s}.{key}"), URL]);
        }
        // Both directions reach the bare repository, never GitHub.
        assert_eq!(git(&work, &["remote", "get-url", "origin"]), bare_s);
        assert_eq!(
            git(&work, &["remote", "get-url", "--push", "origin"]),
            bare_s
        );
        std::fs::write(work.join("crates/a/src/lib.rs"), "// a\n").unwrap();
        git(&work, &["add", "-A"]);
        git(&work, &["commit", "-q", "-m", "base"]);
        if push_base {
            git(&work, &["push", "-q", "origin", "main"]);
        }
        let base = git(&work, &["rev-parse", "HEAD"]);
        let ctl = FakeGithubCtl::open(&tmp.path().join("github"));
        Rig {
            tmp,
            work,
            bare,
            ctl,
            base,
        }
    }

    /// The fake GitHub knows the repository and `gh` is logged in to github.com.
    pub(in crate::run::driver) fn ready(push_base: bool) -> Rig {
        let rig = Rig::new(push_base);
        rig.ctl.create_repo("fake", "app", &rig.bare, "main");
        rig.ctl.log_in("github.com");
        rig
    }

    pub(in crate::run::driver) fn host(&self) -> Arc<dyn CodeHost> {
        Arc::new(FakeHost::new(self.ctl.dir()))
    }

    pub(in crate::run::driver) fn req(&self) -> PreflightReq {
        PreflightReq {
            root: self.work.clone(),
            remote: "origin".into(),
            base_branch: "main".into(),
            base_sha: self.base.clone(),
            nonce: "0a1b2c3d".into(),
        }
    }
}

/// `host`'s preflight from the driver, its refusal text.
async fn refusal(host: Arc<dyn CodeHost>, req: PreflightReq, bound: Duration) -> String {
    match preflight(host, "git".into(), req, bound).await {
        Ok(frozen) => panic!("preflight passed: {frozen:?}"),
        Err(text) => text,
    }
}

#[tokio::test]
async fn preflight_messages_are_exact() {
    let b = PREFLIGHT_BOUND;
    // 1. No remote, then one that is not GitHub.
    let rig = Rig::ready(true);
    git(&rig.work, &["config", "--unset", "remote.origin.url"]);
    assert_eq!(
        refusal(rig.host(), rig.req(), b).await,
        "remote origin is not set in this repository; use --delivery local"
    );
    git(
        &rig.work,
        &[
            "config",
            "remote.origin.url",
            "https://gitlab.com/fake/app.git",
        ],
    );
    assert_eq!(
        refusal(rig.host(), rig.req(), b).await,
        "remote origin is not a GitHub repository (https://gitlab.com/fake/app.git); use --delivery local"
    );
    // 2. `gh` missing (`ANTHREX_CODE_HOST=gh`, a path that does not exist).
    let rig = Rig::ready(true);
    let missing = select::build(&CodeHostChoice::Gh {
        bin: TEST_GH_BIN.into(),
    });
    assert_eq!(
        refusal(missing, rig.req(), b).await,
        format!(
            "gh is not installed (looked for {TEST_GH_BIN}); install it, or use --delivery local"
        )
    );
    // 3. Logged out.
    let rig = Rig::new(true);
    rig.ctl.create_repo("fake", "app", &rig.bare, "main");
    assert_eq!(
        refusal(rig.host(), rig.req(), b).await,
        "gh is not logged in to github.com; run gh auth login, or use --delivery local"
    );
    // 4. A repository gh cannot see.
    let rig = Rig::new(true);
    rig.ctl.log_in("github.com");
    assert_eq!(
        refusal(rig.host(), rig.req(), b).await,
        "gh cannot see fake/app: GraphQL: Could not resolve to a Repository with the name 'fake/app'. (repository)"
    );
    // 5. A dry-run push the remote refuses (its URL rewritten to no repository).
    let rig = Rig::ready(true);
    let nowhere = rig.tmp.path().join("nowhere.git");
    let nowhere = nowhere.to_string_lossy();
    git(&rig.work, &["config", "remote.origin.pushurl", &nowhere]);
    // The last line of git's stderr, as the Messages table says.
    assert_eq!(
        refusal(rig.host(), rig.req(), b).await,
        "a dry-run push to origin was refused: and the repository exists."
    );
    assert!(
        rig.ctl.calls().iter().all(|c| c[0] != "pr"),
        "no PR command ran"
    );
    // 6. The base branch is not on the remote.
    let rig = Rig::ready(false);
    assert_eq!(
        refusal(rig.host(), rig.req(), b).await,
        "the base branch main does not exist on origin; push it first"
    );
    // A check that times out names itself.
    let rig = Rig::ready(true);
    let slow = GhHost::new(
        ScriptedRunner::new()
            .ok(&format!("{URL}\n"))
            .answer(Err(HostError::TimedOut("gh --version".into()))),
        TEST_GH_BIN,
        "git",
    );
    assert_eq!(
        refusal(Arc::new(slow), rig.req(), b).await,
        "gh --version: gh did not answer within 30 s"
    );
    // And a host that ignores its commands' timeouts is cut at the bound.
    let finished = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let sleepy = GhHost::new(Sleepy(finished), TEST_GH_BIN, "git");
    assert_eq!(
        refusal(Arc::new(sleepy), rig.req(), Duration::from_millis(200)).await,
        "preflight: gh did not answer within 1 s"
    );
}

/// A passing preflight freezes the repository and the remote's seal.
#[tokio::test]
async fn a_passing_preflight_freezes_the_repository_and_the_seal() {
    let rig = Rig::ready(true);
    let frozen = preflight(rig.host(), "git".into(), rig.req(), PREFLIGHT_BOUND)
        .await
        .unwrap();
    assert_eq!(frozen.mode, DeliveryMode::Pr);
    let repo = frozen.repo.clone().unwrap();
    assert_eq!(
        (repo.host.as_str(), repo.owner.as_str(), repo.name.as_str()),
        ("github.com", "fake", "app")
    );
    assert_eq!(repo.root, rig.work);
    assert_eq!(
        frozen.seal,
        Some(seal("git".as_ref(), &rig.work, "origin").unwrap())
    );
    // The dry run created nothing on the remote.
    assert_eq!(
        git(&rig.bare, &["for-each-ref", "--format=%(refname)"]),
        "refs/heads/main"
    );
}

/// Decision 3: `--delivery` over the profile's mode, else `local`; the profile's remote,
/// else `origin`.
#[test]
fn the_mode_is_the_flag_then_the_profile_then_local() {
    let pr = DeliveryProfile {
        mode: DeliveryMode::Pr,
        remote: "upstream".into(),
    };
    let local = |remote: &str| (DeliveryMode::Local, remote.to_string());
    let pr_mode = |remote: &str| (DeliveryMode::Pr, remote.to_string());
    assert_eq!(resolve(None, None), local("origin"));
    assert_eq!(resolve(None, Some(&pr)), pr_mode("upstream"));
    assert_eq!(
        resolve(Some(DeliveryMode::Local), Some(&pr)),
        local("upstream")
    );
    assert_eq!(resolve(Some(DeliveryMode::Pr), None), pr_mode("origin"));
}

#[path = "delivery_tests_start.rs"]
mod start;

use std::ffi::OsStr;
