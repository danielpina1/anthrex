//! Task M9.2.5 (and its fix round 1): asked to merge, approve or enable auto-merge in
//! any of `gh`'s spellings, `FakeGh` records the argv and panics; and the allow-list,
//! the first guard, refuses every one of those argv before `FakeGh` could see it.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;
use std::time::Duration;

use super::tests::{FULL, RUN, strings};
use super::*;
use crate::host::allow::{self, AllowCtx};
use crate::host::{Capture, HostError, Program, RunOutput, Runner};

fn run_in(gh: &FakeGh, dir: &Path, args: &[&str]) -> RunOutput {
    gh.run(
        Program::Gh,
        dir,
        &strings(args),
        &[("GH_HOST".to_string(), "github.com".to_string())],
        Duration::from_secs(5),
        Capture::Bytes(1 << 20),
    )
    .unwrap()
}

fn panic_text(gh: &FakeGh, dir: &Path, args: &[&str]) -> String {
    let caught = catch_unwind(AssertUnwindSafe(|| run_in(gh, dir, args)));
    let payload = match caught {
        Ok(out) => panic!("FakeGh answered {args:?} instead of panicking: {out:?}"),
        Err(payload) => payload,
    };
    match payload.downcast::<String>() {
        Ok(text) => *text,
        Err(other) => other
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .unwrap_or_default(),
    }
}

fn landing_text(verb: &str, args: &[&str]) -> String {
    format!(
        "FakeHost: anthrex asked to {verb} gh {}; anthrex never lands anything",
        args.join(" ")
    )
}

/// The first guard: `GhHost` would never have run any of these.
fn assert_refused(args: &[&str]) {
    let ctx = AllowCtx {
        run_id: Some(RUN),
        remote: "origin",
        base_branch: Some("main"),
        repo: Some(FULL),
        pulls: &[],
    };
    match allow::check(Program::Gh, &strings(args), &ctx) {
        Err(HostError::Forbidden(_)) => {}
        other => panic!("the allow-list did not refuse {args:?}: {other:?}"),
    }
}

const AUTO: &str = "query=mutation { enablePullRequestAutoMerge(input: {pullRequestId: \"PR_1\", mergeMethod: SQUASH}) { clientMutationId } }";
const MERGE_MUTATION: &str =
    "mutation { mergePullRequest(input: {pullRequestId: \"PR_1\"}) { clientMutationId } }";
const REVIEW: &str =
    "query=mutation { addPullRequestReview(input: {event: APPROVE}) { clientMutationId } }";

#[test]
fn fake_host_panics_on_merge_approve_and_auto_merge() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("fake");
    let ctl = FakeGithubCtl::open(&dir);
    let gh = FakeGh::new(&dir);
    let cases: [(&[&str], &str); 5] = [
        (&["pr", "merge", "1", "--repo", FULL, "--merge"], "merge"),
        (
            &["pr", "merge", "1", "--repo", FULL, "--auto", "--squash"],
            "enable auto-merge",
        ),
        (
            &["pr", "review", "1", "--repo", FULL, "--approve"],
            "approve",
        ),
        (
            &[
                "api",
                "-X",
                "PUT",
                "repos/anthrex-test/widgets/pulls/1/merge",
            ],
            "merge",
        ),
        (&["api", "graphql", "-f", AUTO], "enable auto-merge"),
    ];
    for (args, verb) in cases {
        assert_eq!(
            panic_text(&gh, tmp.path(), args),
            landing_text(verb, args),
            "{args:?}"
        );
        assert_refused(args);
    }
    let lines = std::fs::read_to_string(dir.join("forbidden.jsonl")).unwrap();
    assert_eq!(lines.lines().count(), 5, "{lines}");
    let expected: Vec<Vec<String>> = cases.iter().map(|(a, _)| strings(a)).collect();
    assert_eq!(ctl.forbidden(), expected);
    // Every ask is a call too, recorded before the panic (fix round 1, m3).
    assert_eq!(ctl.calls(), expected);
}

#[test]
fn fake_host_panics_on_every_spelling_of_a_landing() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("fake");
    let gh = FakeGh::new(&dir);
    // Read relative to the directory `gh` runs in, never the test's own.
    std::fs::write(tmp.path().join("mutation.graphql"), MERGE_MUTATION).unwrap();
    std::fs::write(
        tmp.path().join("body.json"),
        format!("{{\"query\": \"{}\"}}", MERGE_MUTATION.replace('"', "\\\"")),
    )
    .unwrap();
    let glued_review = format!("-f{REVIEW}");
    let glued_auto = format!("-F{AUTO}");
    let long_raw = format!("--raw-field={REVIEW}");
    let more: &[(&[&str], &str)] = &[
        (
            &[
                "api",
                "repos/anthrex-test/widgets/pulls/1/reviews",
                "-f",
                "event=APPROVE",
            ],
            "approve",
        ),
        (
            &[
                "api",
                "-X",
                "POST",
                "repos/anthrex-test/widgets/pulls/1/reviews/7/events",
                "-f",
                "event=APPROVE",
            ],
            "approve",
        ),
        (
            &[
                "api",
                "--method=PUT",
                "repos/anthrex-test/widgets/pulls/1/merge",
            ],
            "merge",
        ),
        (
            &["api", "-XPUT", "/repos/anthrex-test/widgets/pulls/1/merge"],
            "merge",
        ),
        (
            &[
                "api",
                "-X=PUT",
                "repos/anthrex-test/widgets/pulls/1/update-branch",
            ],
            "merge",
        ),
        (
            &[
                "api",
                "-iXPUT",
                "repos/anthrex-test/widgets/pulls/1/update-branch",
            ],
            "merge",
        ),
        (
            &[
                "api",
                "--method",
                "put",
                "repos/anthrex-test/widgets/contents/x.txt",
            ],
            "merge",
        ),
        (
            &[
                "api",
                "repos/anthrex-test/widgets/merges",
                "-f",
                "base=main",
            ],
            "merge",
        ),
        // A merge path after a flag whose value could pass for a path.
        (
            &[
                "api",
                "-H",
                "Accept: application/json",
                "https://github.com/repos/anthrex-test/widgets/pulls/1/merge",
            ],
            "merge",
        ),
        (
            &["api", "graphql", "-F", "query=@mutation.graphql"],
            "merge",
        ),
        (&["api", "-Fquery=@mutation.graphql", "graphql"], "merge"),
        (
            &["api", "--field=query=@mutation.graphql", "/graphql"],
            "merge",
        ),
        (&["api", "--input", "body.json", "graphql"], "merge"),
        (
            &[
                "api",
                "--input=body.json",
                "https://ghe.example.invalid/api/graphql",
            ],
            "merge",
        ),
        (&["api", "graphql", "-f", REVIEW], "approve"),
        (&["api", &glued_review, "graphql"], "approve"),
        (
            &["api", &long_raw, "https://github.example.invalid/graphql"],
            "approve",
        ),
        (&["api", &glued_auto, "/graphql"], "enable auto-merge"),
        (&["pr", "review", "1", "--comment", "-b", "ok"], "approve"),
        (&["pr", "-R", FULL, "review", "1", "--approve"], "approve"),
        (
            &[
                "pr",
                "--repo=anthrex-test/widgets",
                "review",
                "1",
                "--approve",
            ],
            "approve",
        ),
        (&["pr", "--repo", FULL, "merge", "1"], "merge"),
        (
            &["pr", "-Ranthrex-test/widgets", "merge", "1", "--auto"],
            "enable auto-merge",
        ),
        (
            &["pr", "-R", FULL, "merge", "1", "--auto=true"],
            "enable auto-merge",
        ),
        (
            &["pr", "merge", "--auto", "--disable-auto"],
            "enable auto-merge",
        ),
    ];
    for (args, verb) in more {
        assert_eq!(
            panic_text(&gh, tmp.path(), args),
            landing_text(verb, args),
            "{args:?}"
        );
        assert_refused(args);
    }
    let ctl = FakeGithubCtl::open(&dir);
    assert_eq!(ctl.forbidden().len(), more.len());
    assert_eq!(ctl.calls().len(), more.len());

    // Not landings: a reply whose body mentions merging, and flag values that read
    // like a merge path.
    let not: [&[&str]; 4] = [
        &[
            "api",
            "-X",
            "POST",
            "repos/anthrex-test/widgets/pulls/1/comments/5/replies",
            "-f",
            "body=please /merge",
        ],
        &[
            "api",
            "--template",
            "merge",
            "repos/anthrex-test/widgets/pulls/1/comments",
            "--paginate",
        ],
        &[
            "api",
            "-p",
            "merge",
            "repos/anthrex-test/widgets/issues/1/comments",
            "--paginate",
        ],
        &[
            "api",
            "--cache",
            "merge",
            "-q",
            "merges",
            "repos/anthrex-test/widgets/collaborators/a/permission",
        ],
    ];
    for args in not {
        assert!(!run_in(&gh, tmp.path(), args).success, "{args:?}");
    }
    assert_eq!(ctl.forbidden().len(), more.len());
}

/// Fix round 1, m1: the guard runs before anything is recorded, so a landing ask
/// panics even when its argv cannot be written down; it is never answered as an error.
#[test]
fn fake_host_panics_on_a_landing_even_when_it_cannot_record_it() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("fake");
    std::fs::create_dir_all(dir.join("calls.jsonl")).unwrap();
    let gh = FakeGh::new(&dir);
    let args = ["pr", "merge", "1", "--repo", FULL];
    let text = panic_text(&gh, tmp.path(), &args);
    let expected = landing_text("merge", &args);
    let prefix = expected.strip_suffix(" anything").unwrap();
    assert!(text.starts_with(prefix), "{text}");
    assert!(text.contains("calls.jsonl"), "{text}");
    assert_eq!(FakeGithubCtl::open(&dir).forbidden(), vec![strings(&args)]);
}
