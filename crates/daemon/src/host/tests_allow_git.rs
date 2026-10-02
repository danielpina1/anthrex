//! Task M9.2.4: decision 7's allow-list, for `git`: no force, no foreign destination, no
//! option-like remote and none of the old push shapes. Split from `tests_allow.rs` for
//! AGENTS.md rule 8 (move-only).

use super::allow::{AllowCtx, check};
use super::tests::{NOHOOK, SHA, WRITE, argv, with};
use super::tests_allow::{RUN, accepted, ctx, refused};
use super::*;

#[test]
fn allow_list_refuses_force_and_foreign_destinations() {
    let stage = format!("refs/heads/anthrex/{RUN}/stage-1");
    let spec = format!("{SHA}:{stage}");
    let push = |extra: &[&str], spec: &str| {
        let mut parts = vec![
            "push",
            "--porcelain",
            "--no-follow-tags",
            "--recurse-submodules=no",
        ];
        parts.extend_from_slice(extra);
        parts.extend(["origin", spec]);
        with(&WRITE, &parts)
    };
    accepted(Program::Git, &push(&[], &spec));
    for flag in [
        "--force",
        "-f",
        "--force-with-lease",
        "--force-with-lease=refs/heads/x",
        "--force-if-includes",
        "--mirror",
        "--all",
        "--tags",
        "--prune",
    ] {
        refused(Program::Git, &push(&[flag], &spec));
    }
    for bad in [
        format!("+{SHA}:{stage}"),
        format!("{SHA}:refs/heads/main"),
        format!("{SHA}:main"),
        format!("{SHA}:refs/heads/anthrex/r9999/stage-1"),
        format!("{SHA}:refs/heads/anthrex/{RUN}/integration"),
        format!("{SHA}:refs/heads/anthrex/{RUN}/stage-01"),
        format!("{SHA}:refs/heads/anthrex/{RUN}/stage-1/x"),
        format!("{SHA}:refs/tags/v1"),
        format!("HEAD:{stage}"),
        format!("main:{stage}"),
        format!("{}:{stage}", &SHA[..12]),
        SHA.to_string(),
        format!(":{stage}"),
        "refs/heads/*:refs/heads/*".to_string(),
    ] {
        refused(Program::Git, &push(&[], &bad));
    }
    // Two refspecs, or the push without anthrex's write flags.
    refused(
        Program::Git,
        &with(
            &WRITE,
            &[
                "push",
                "--porcelain",
                "--no-follow-tags",
                "--recurse-submodules=no",
                "origin",
                &spec,
                &spec,
            ],
        ),
    );
    refused(
        Program::Git,
        &argv(&[
            "push",
            "--porcelain",
            "--no-follow-tags",
            "--recurse-submodules=no",
            "origin",
            &spec,
        ]),
    );
    refused(
        Program::Git,
        &with(
            &NOHOOK,
            &[
                "push",
                "--porcelain",
                "--no-follow-tags",
                "--recurse-submodules=no",
                "origin",
                &spec,
            ],
        ),
    );
    // Another remote.
    refused(
        Program::Git,
        &with(
            &WRITE,
            &[
                "push",
                "--porcelain",
                "--no-follow-tags",
                "--recurse-submodules=no",
                "upstream",
                &spec,
            ],
        ),
    );
    // Deletes: only this run's stage branches.
    accepted(
        Program::Git,
        &with(
            &WRITE,
            &[
                "push",
                "--porcelain",
                "--no-follow-tags",
                "--recurse-submodules=no",
                "origin",
                "--delete",
                &stage,
            ],
        ),
    );
    for dst in [
        "refs/heads/main",
        "main",
        "refs/heads/anthrex/r9999/stage-1",
        "refs/heads/anthrex/r1a2b/integration",
    ] {
        refused(
            Program::Git,
            &with(
                &WRITE,
                &[
                    "push",
                    "--porcelain",
                    "--no-follow-tags",
                    "--recurse-submodules=no",
                    "origin",
                    "--delete",
                    dst,
                ],
            ),
        );
    }
    refused(
        Program::Git,
        &with(
            &WRITE,
            &[
                "push",
                "--porcelain",
                "--no-follow-tags",
                "--recurse-submodules=no",
                "origin",
                &format!(":{stage}"),
            ],
        ),
    );
    // The dry run: only with --dry-run, only to anthrex/preflight-<8 hex>.
    let dry = format!("{SHA}:refs/heads/anthrex/preflight-0a1b2c3d");
    accepted(
        Program::Git,
        &with(
            &WRITE,
            &[
                "push",
                "--dry-run",
                "--porcelain",
                "--no-follow-tags",
                "--recurse-submodules=no",
                "origin",
                &dry,
            ],
        ),
    );
    refused(Program::Git, &push(&[], &dry));
    refused(
        Program::Git,
        &with(
            &WRITE,
            &[
                "push",
                "--dry-run",
                "--porcelain",
                "--no-follow-tags",
                "--recurse-submodules=no",
                "origin",
                &format!("{SHA}:refs/heads/main"),
            ],
        ),
    );
    // Fetch: only into this run's private refs.
    let fetch = |spec: &str| {
        with(
            &WRITE,
            &[
                "fetch",
                "--no-tags",
                "--no-prune",
                "--no-prune-tags",
                "--no-recurse-submodules",
                "--no-auto-maintenance",
                "--no-write-fetch-head",
                "--refmap=",
                "origin",
                spec,
            ],
        )
    };
    accepted(
        Program::Git,
        &fetch(&format!("+refs/heads/main:refs/anthrex/{RUN}/remote/base")),
    );
    for bad in [
        "+refs/heads/main:refs/heads/main".to_string(),
        "+refs/heads/main:refs/remotes/origin/main".to_string(),
        "+refs/heads/main:refs/anthrex/r9999/remote/base".to_string(),
        format!("+refs/heads/*:refs/anthrex/{RUN}/remote/*"),
        format!("refs/heads/main:refs/anthrex/{RUN}/remote/base"),
        format!("+refs/heads/main:refs/heads/anthrex/{RUN}/stage-1"),
    ] {
        refused(Program::Git, &fetch(&bad));
    }
    refused(
        Program::Git,
        &with(
            &WRITE,
            &[
                "fetch",
                "--no-tags",
                "--no-prune",
                "--no-prune-tags",
                "--no-recurse-submodules",
                "--no-auto-maintenance",
                "--no-write-fetch-head",
                "--refmap=",
                "--prune",
                "origin",
                &format!("+refs/heads/main:refs/anthrex/{RUN}/remote/base"),
            ],
        ),
    );
}

#[test]
fn allow_list_refuses_a_remote_named_like_an_option_and_the_old_push_shapes() {
    // Fix round 1 (m1): a remote that reads as an option is refused even when the
    // context names it.
    let option = AllowCtx {
        remote: "--receive-pack=evil",
        ..ctx()
    };
    let push = with(
        &WRITE,
        &[
            "push",
            "--porcelain",
            "--no-follow-tags",
            "--recurse-submodules=no",
            "--receive-pack=evil",
            &format!("{SHA}:refs/heads/anthrex/{RUN}/stage-1"),
        ],
    );
    assert!(matches!(
        check(Program::Git, &push, &option),
        Err(HostError::Forbidden(_))
    ));
    let config = with(&NOHOOK, &["config", "--get", "remote.-x.url"]);
    let dash = AllowCtx {
        remote: "-x",
        ..ctx()
    };
    assert!(matches!(
        check(Program::Git, &config, &dash),
        Err(HostError::Forbidden(_))
    ));
    // Ruling I1: a push or fetch without its no-tags and no-submodules flags; the
    // dry run and the delete too (deferred from task 4); and a fetch without
    // `--no-prune --no-prune-tags`, which a user's `fetch.prune` would otherwise turn on.
    let spec = format!("{SHA}:refs/heads/anthrex/{RUN}/stage-1");
    let dry = format!("{SHA}:refs/heads/anthrex/preflight-0a1b2c3d");
    let stage = format!("refs/heads/anthrex/{RUN}/stage-1");
    for args in [
        with(&WRITE, &["push", "--porcelain", "origin", &spec]),
        with(
            &WRITE,
            &["push", "--porcelain", "--no-follow-tags", "origin", &spec],
        ),
        with(
            &WRITE,
            &["push", "--dry-run", "--porcelain", "origin", &dry],
        ),
        with(
            &WRITE,
            &[
                "push",
                "--dry-run",
                "--porcelain",
                "--no-follow-tags",
                "origin",
                &dry,
            ],
        ),
        with(
            &WRITE,
            &[
                "push",
                "--dry-run",
                "--porcelain",
                "--recurse-submodules=no",
                "origin",
                &dry,
            ],
        ),
        with(
            &WRITE,
            &["push", "--porcelain", "origin", "--delete", &stage],
        ),
        with(
            &WRITE,
            &[
                "push",
                "--porcelain",
                "--no-follow-tags",
                "origin",
                "--delete",
                &stage,
            ],
        ),
        with(
            &WRITE,
            &[
                "push",
                "--porcelain",
                "--recurse-submodules=no",
                "origin",
                "--delete",
                &stage,
            ],
        ),
        with(
            &WRITE,
            &[
                "fetch",
                "--no-tags",
                "--no-recurse-submodules",
                "--no-auto-maintenance",
                "--no-write-fetch-head",
                "--refmap=",
                "origin",
                &format!("+refs/heads/main:refs/anthrex/{RUN}/remote/base"),
            ],
        ),
        with(
            &WRITE,
            &[
                "fetch",
                "--no-tags",
                "--no-write-fetch-head",
                "--refmap=",
                "origin",
                &format!("+refs/heads/main:refs/anthrex/{RUN}/remote/base"),
            ],
        ),
    ] {
        refused(Program::Git, &args);
    }
}
