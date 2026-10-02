//! Task M9.2.12 fix round 1, I1: the remote's seal is read through `GhHost::git`, so its
//! two `git remote get-url` commands pass decision 7's allow-list like every host
//! command. Exactly two shapes are accepted, in the read context, for the run's remote;
//! every near miss is refused before any process starts.

use std::ffi::OsStr;
use std::path::Path;
use std::time::{Duration, Instant};

use super::allow::{AllowCtx, check};
use super::scripted::ScriptedRunner;
use super::tests::{NOHOOK, WRITE, with};
use super::*;

const FETCH: [&str; 3] = ["remote", "get-url", "--all"];
const PUSH: [&str; 4] = ["remote", "get-url", "--push", "--all"];

fn ctx(remote: &str) -> AllowCtx<'_> {
    AllowCtx {
        run_id: None,
        remote,
        base_branch: None,
        repo: None,
    }
}

fn seal_argv(flags: &[&str], shape: &[&str], remote: &str) -> Vec<String> {
    let mut all = with(flags, shape);
    all.push(remote.to_string());
    all
}

#[test]
fn the_seal_reads_pass_the_allow_list_exactly() {
    for shape in [&FETCH[..], &PUSH[..]] {
        let ok = seal_argv(&NOHOOK, shape, "origin");
        assert_eq!(check(Program::Git, &ok, &ctx("origin")), Ok(()), "{ok:?}");
    }
    let refused = |args: Vec<String>, remote: &str| match check(Program::Git, &args, &ctx(remote)) {
        Err(HostError::Forbidden(text)) => {
            assert!(text.starts_with("anthrex never runs: "), "{text}")
        }
        other => panic!("{args:?} was not refused: {other:?}"),
    };
    // No `--all`, the flags in another order, an extra argument, two remotes.
    refused(with(&NOHOOK, &["remote", "get-url", "origin"]), "origin");
    refused(
        with(&NOHOOK, &["remote", "get-url", "--push", "origin"]),
        "origin",
    );
    refused(
        with(&NOHOOK, &["remote", "get-url", "--all", "--push", "origin"]),
        "origin",
    );
    refused(
        with(&NOHOOK, &["remote", "get-url", "--all", "origin", "x"]),
        "origin",
    );
    refused(
        with(&NOHOOK, &["remote", "get-url", "--all", "--all", "origin"]),
        "origin",
    );
    refused(
        with(
            &NOHOOK,
            &["remote", "get-url", "--push", "--all", "-v", "origin"],
        ),
        "origin",
    );
    // A remote that is not the run's, or one shaped like an option.
    refused(seal_argv(&NOHOOK, &FETCH, "upstream"), "origin");
    refused(
        seal_argv(&NOHOOK, &FETCH, "--upload-pack=x"),
        "--upload-pack=x",
    );
    refused(seal_argv(&NOHOOK, &PUSH, "-x"), "-x");
    refused(seal_argv(&NOHOOK, &FETCH, ""), "");
    // The write context, no hooks flag at all, or another `remote` subcommand.
    refused(seal_argv(&WRITE, &FETCH, "origin"), "origin");
    refused(seal_argv(&WRITE, &PUSH, "origin"), "origin");
    refused(seal_argv(&[], &FETCH, "origin"), "origin");
    refused(
        with(&NOHOOK, &["remote", "set-url", "--all", "origin", "x"]),
        "origin",
    );
    refused(
        with(&NOHOOK, &["remote", "set-url", "--push", "origin", "x"]),
        "origin",
    );
    refused(with(&NOHOOK, &["remote", "-v"]), "origin");
    // `--all` stays refused everywhere else.
    refused(with(&NOHOOK, &["fetch", "--all"]), "origin");
}

/// Through `GhHost` the runner sees exactly the two allowed commands, in the repository,
/// as a read; a refused remote reaches no process.
#[test]
fn remote_seal_runs_the_two_reads_through_the_host() {
    let root = Path::new("/nonexistent/anthrex-test/work");
    let host = GhHost::new(
        ScriptedRunner::new()
            .ok("https://github.com/o/r.git\n")
            .ok("https://github.com/o/r.git\n"),
        crate::manager::TEST_GH_BIN,
        "git",
    );
    let seal = host.remote_seal(root, "origin").unwrap();
    assert_eq!(seal.len(), 64);
    assert!(seal.bytes().all(|b| b.is_ascii_hexdigit()), "{seal}");
    let calls = host.runner().calls();
    let shown: Vec<_> = calls.iter().map(|c| (c.program, c.argv.clone())).collect();
    assert_eq!(
        shown,
        [
            (Program::Git, seal_argv(&NOHOOK, &FETCH, "origin")),
            (Program::Git, seal_argv(&NOHOOK, &PUSH, "origin")),
        ]
    );
    assert!(
        calls
            .iter()
            .all(|c| c.dir == root && c.timeout == HOST_READ_TIMEOUT)
    );

    let refused = GhHost::new(ScriptedRunner::new(), "gh", "git");
    assert!(matches!(
        refused.remote_seal(root, "-x"),
        Err(HostError::Forbidden(_))
    ));
    assert!(refused.runner().calls().is_empty());
    // A failed read names the remote, never a URL.
    let failing = GhHost::new(
        ScriptedRunner::new().fails("", "error: No such remote 'origin'\n"),
        "gh",
        "git",
    );
    assert_eq!(
        failing.remote_seal(root, "origin"),
        Err(HostError::Failed(
            "cannot read remote origin's URLs: error: No such remote 'origin'".into()
        ))
    );
}

fn git(dir: &Path, args: &[&str]) {
    let os: Vec<&OsStr> = args.iter().map(OsStr::new).collect();
    let deadline = Instant::now() + Duration::from_secs(30);
    let out = crate::worktree::run_git(OsStr::new("git"), dir, &os, deadline).unwrap();
    assert!(out.success, "git {args:?}: {}", out.stderr);
}

/// Over a real repository: the seal is stable, and moves with the URL, the push URL and
/// an `insteadOf` rewrite alike.
#[test]
fn the_seal_follows_every_url_git_would_use() {
    let tmp = tempfile::tempdir().unwrap();
    let work = tmp.path();
    git(work, &["init", "-q", "-b", "main"]);
    git(
        work,
        &["config", "remote.origin.url", "https://github.com/o/r.git"],
    );
    let host = select::build(&select::CodeHostChoice::Gh {
        bin: crate::manager::TEST_GH_BIN.into(),
    });
    let first = host.remote_seal(work, "origin").unwrap();
    assert_eq!(host.remote_seal(work, "origin").unwrap(), first);
    let mut seen = vec![first];
    for change in [
        ["config", "url./elsewhere.insteadOf", "https://github.com/"],
        ["config", "url./other.pushInsteadOf", "https://github.com/"],
        ["config", "remote.origin.pushurl", "/somewhere/else.git"],
    ] {
        git(work, &change);
        let now = host.remote_seal(work, "origin").unwrap();
        assert!(!seen.contains(&now), "{change:?} left the seal as it was");
        seen.push(now);
    }
}
