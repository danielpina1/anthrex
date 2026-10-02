//! Task M9.2.5: one stage PR's whole life through `GhHost<FakeGh>` (the `calls.jsonl`
//! it leaves is exactly the allow-listed argv), and `FakeGh`'s answers held against the
//! shapes M9.2.1 recorded from the real gh (review focus 7: a fake that agrees with the
//! code and not with the tool is the milestone's known failure). The rig is in
//! `tests.rs`.

use std::collections::BTreeMap;

use serde_json::Value;

use super::tests::{FULL, NAME, OWNER, RUN, Rig, git, head, run_gh, strings};
use super::*;
use crate::host::allow::{self, AllowCtx};
use crate::host::gh::{PR_LIST_FIELDS, PR_VIEW_FIELDS};
use crate::host::{
    CheckStatus, CodeHost, Conclusion, Mergeable, PrRef, PrState, PreflightReq, Program,
    PushOutcome, ReplyReq, ReplyTarget, ReviewState, THREADS_QUERY,
};

#[test]
fn fake_pr_lifecycle_through_gh_host() {
    let rig = Rig::new();
    let repo = rig.host.preflight(&PreflightReq {
        root: rig.work.clone(),
        remote: "origin".to_string(),
        base_branch: "main".to_string(),
        base_sha: rig.base.clone(),
        nonce: "0a1b2c3d".to_string(),
    });
    assert_eq!(repo, Ok(rig.repo()));

    let sha = rig.commit(
        &rig.base,
        &[("src/lib.rs", Some("pub fn one() {}\n"))],
        "stage 1",
    );
    assert_eq!(rig.push(1, &sha), PushOutcome::Pushed);
    assert_eq!(rig.remote(&head(1)).as_deref(), Some(sha.as_str()));
    assert_eq!(rig.push(1, &sha), PushOutcome::UpToDate);
    // The dry run of preflight created nothing.
    assert_eq!(rig.remote("anthrex/preflight-0a1b2c3d"), None);

    let pr = rig.open(1, "main", "Stage 1: one");
    assert_eq!(
        pr,
        PrRef {
            number: 1,
            url: "https://github.com/anthrex-test/widgets/pull/1".to_string(),
            state: PrState::Open,
            existed: false,
            base: None,
        }
    );
    let again = rig.open(1, "main", "Stage 1: one");
    assert!(again.existed);
    assert_eq!(again.number, 1);
    assert_eq!(again.base.as_deref(), Some("main"));

    rig.ctl
        .set_ci(vec![CiRule::new("build", Conclusion::Success)]);
    let view = rig.host.view_pr(&rig.repo(), 1).unwrap();
    assert_eq!(view.head_oid, sha);
    assert_eq!(view.base_ref, "main");
    assert_eq!(view.state, PrState::Open);
    assert_eq!(view.mergeable, Mergeable::Mergeable);
    assert_eq!(view.checks.len(), 1);
    assert_eq!(view.checks[0].name, "build");
    assert_eq!(view.checks[0].status, CheckStatus::Completed);
    assert_eq!(view.checks[0].conclusion, Some(Conclusion::Success));
    let ci_run = view.checks[0].ci_run.expect("an Actions run id");
    assert!(
        view.checks[0]
            .url
            .contains(&format!("/actions/runs/{ci_run}/job/"))
    );

    let first = rig
        .ctl
        .review_comment(1, "alice", "src/lib.rs", 1, "Please name it two.");
    assert!(
        first > u64::from(u32::MAX),
        "ids pass 32 bits as GitHub's do"
    );
    let view = rig.host.view_pr(&rig.repo(), 1).unwrap();
    assert_eq!(view.threads.len(), 1);
    assert_eq!(view.threads[0].comments[0].id, first);
    assert_eq!(view.threads[0].path.as_deref(), Some("src/lib.rs"));
    assert_eq!(view.threads[0].line, Some(1));
    assert!(
        view.threads[0].comments[0]
            .diff_hunk
            .contains("pub fn one() {}")
    );
    assert!(!view.threads[0].comments[0].author.bot);

    let marker = format!("<!-- anthrex:reply {RUN} 1:t{first} {} -->", &sha[..7]);
    let reply = ReplyReq {
        repo: rig.repo(),
        number: 1,
        target: ReplyTarget::Thread { comment_id: first },
        body: "Renamed.".to_string(),
        marker: marker.clone(),
    };
    let replied = rig.host.reply(&reply).unwrap();
    assert!(replied > first);
    let prs = rig.ctl.prs();
    assert_eq!(prs[0].replies.len(), 1);
    assert_eq!(prs[0].replies[0].id, replied);
    assert_eq!(prs[0].replies[0].thread, Some(first));
    assert_eq!(prs[0].replies[0].body, format!("Renamed.\n\n{marker}"));
    assert_eq!(prs[0].threads[0].comments.len(), 2);
    assert_eq!(prs[0].threads[0].comments[1].user, FAKE_LOGIN);

    git(
        &rig.work,
        &[
            "push",
            "-q",
            "origin",
            &format!("{}:refs/heads/dev", rig.base),
        ],
    );
    rig.host.retarget(&rig.repo(), 1, "dev").unwrap();
    assert_eq!(rig.ctl.prs()[0].base, "dev");

    let body_file = rig.work.join("../pr-1.md").to_string_lossy().into_owned();
    let replies = format!("repos/{FULL}/pulls/1/comments/{first}/replies");
    let expected: Vec<Vec<String>> = vec![
        strings(&["--version"]),
        strings(&["auth", "status", "--hostname", "github.com"]),
        strings(&["repo", "view", FULL, "--json", "nameWithOwner"]),
        strings(&[
            "pr",
            "list",
            "--repo",
            FULL,
            "--head",
            &head(1),
            "--state",
            "all",
            "--json",
            PR_LIST_FIELDS,
        ]),
        strings(&[
            "pr",
            "create",
            "--repo",
            FULL,
            "--base",
            "main",
            "--head",
            &head(1),
            "--title",
            "Stage 1: one",
            "--body-file",
            &body_file,
        ]),
        strings(&[
            "pr",
            "list",
            "--repo",
            FULL,
            "--head",
            &head(1),
            "--state",
            "all",
            "--json",
            PR_LIST_FIELDS,
        ]),
        strings(&["pr", "view", "1", "--repo", FULL, "--json", PR_VIEW_FIELDS]),
        strings(&[
            "api",
            "graphql",
            "-f",
            &format!("query={THREADS_QUERY}"),
            "-f",
            &format!("owner={OWNER}"),
            "-f",
            &format!("name={NAME}"),
            "-F",
            "number=1",
        ]),
        strings(&["pr", "view", "1", "--repo", FULL, "--json", PR_VIEW_FIELDS]),
        strings(&[
            "api",
            "graphql",
            "-f",
            &format!("query={THREADS_QUERY}"),
            "-f",
            &format!("owner={OWNER}"),
            "-f",
            &format!("name={NAME}"),
            "-F",
            "number=1",
        ]),
        strings(&["api", "user"]),
        strings(&[
            "api",
            &format!("repos/{FULL}/pulls/1/comments"),
            "--paginate",
        ]),
        strings(&[
            "api",
            "-X",
            "POST",
            &replies,
            "-f",
            &format!("body=Renamed.\n\n{marker}"),
        ]),
        strings(&["pr", "edit", "1", "--repo", FULL, "--base", "dev"]),
    ];
    let calls = rig.ctl.calls();
    assert_eq!(calls, expected);
    let ctx = AllowCtx {
        run_id: Some(RUN),
        remote: "origin",
        base_branch: Some("main"),
        repo: Some(FULL),
    };
    for call in &calls {
        allow::check(Program::Gh, call, &ctx).unwrap();
    }

    // The marker is found: a second reply posts nothing.
    assert_eq!(rig.host.reply(&reply), Ok(replied));
    assert_eq!(rig.ctl.prs()[0].replies.len(), 1);
    assert!(rig.ctl.forbidden().is_empty());
}

/// A value's shape: an object's keys with their shapes, an array's first element, a
/// scalar's kind.
fn shape(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            Value::Object(map.iter().map(|(k, v)| (k.clone(), shape(v))).collect())
        }
        Value::Array(items) => Value::Array(items.iter().take(1).map(shape).collect()),
        Value::Null => Value::from("null"),
        Value::Bool(_) => Value::from("bool"),
        Value::Number(_) => Value::from("number"),
        Value::String(_) => Value::from("string"),
    }
}

fn keys(value: &Value) -> Vec<String> {
    let mut keys: Vec<String> = value.as_object().unwrap().keys().cloned().collect();
    keys.sort();
    keys
}

fn fixture(text: &str) -> Value {
    serde_json::from_str(text).unwrap()
}

fn stdout(rig: &Rig, args: &[&str]) -> String {
    let out = run_gh(&rig.gh(), args);
    assert!(out.success, "{args:?}: {}", out.stderr);
    String::from_utf8(out.stdout).unwrap()
}

/// Every key of each of `ours`' rows is a key some recorded row has, with the same
/// shape where both have it.
fn within(ours: &[Value], recorded: &[Value]) {
    let mut known: BTreeMap<String, Value> = BTreeMap::new();
    for row in recorded {
        for (k, v) in row.as_object().unwrap() {
            known.entry(k.clone()).or_insert_with(|| shape(v));
        }
    }
    for row in ours {
        for (k, v) in row.as_object().unwrap() {
            let seen = known
                .get(k)
                .unwrap_or_else(|| panic!("{k} is in no recorded row"));
            if *seen != "null" && shape(v) != "null" {
                assert_eq!(&shape(v), seen, "{k}");
            }
        }
    }
}

#[test]
fn fake_answers_keep_the_recorded_shapes() {
    let rig = Rig::new();
    rig.ctl
        .set_ci(vec![CiRule::new("build", Conclusion::Failure)]);
    let sha = rig.commit(
        &rig.base,
        &[("src/lib.rs", Some("pub fn one() {}\n"))],
        "stage 1",
    );
    rig.push(1, &sha);
    rig.open(1, "main", "Stage 1");
    let first = rig
        .ctl
        .review_comment(1, "alice", "src/lib.rs", 1, "Rename it.");
    let second = rig.ctl.reply_in_thread(1, first, "bob", "Agreed.");
    // GitHub takes a reply only to a thread's first comment (M9.2.1 check 4).
    for to in [second, first + 1_000] {
        let path = format!("repos/{FULL}/pulls/1/comments/{to}/replies");
        let out = run_gh(&rig.gh(), &["api", "-X", "POST", &path, "-f", "body=x"]);
        assert!(!out.success);
        assert_eq!(out.stderr, "gh: Not Found (HTTP 404)\n");
    }
    rig.ctl.comment(1, "github-actions[bot]", "Thanks!");
    rig.ctl
        .review(1, "alice", ReviewState::ChangesRequested, "Please rename.");
    rig.ctl
        .set_permission("alice", crate::host::RepoPermission::Write);

    let view = fixture(&stdout(
        &rig,
        &["pr", "view", "1", "--repo", FULL, "--json", PR_VIEW_FIELDS],
    ));
    let recorded = fixture(include_str!("../fixtures/pr_view_open_red.json"));
    let mut wanted: Vec<String> = PR_VIEW_FIELDS.split(',').map(str::to_string).collect();
    wanted.sort();
    assert_eq!(keys(&view), wanted, "only the fields asked for");
    for field in &wanted {
        assert_eq!(shape(&view[field]), shape(&recorded[field]), "{field}");
    }

    let listed = fixture(&stdout(
        &rig,
        &[
            "pr",
            "list",
            "--repo",
            FULL,
            "--head",
            &head(1),
            "--state",
            "all",
            "--json",
            PR_LIST_FIELDS,
        ],
    ));
    let recorded = fixture(include_str!("../fixtures/pr_list_head_owner.json"));
    assert_eq!(shape(&listed), shape(&recorded));

    let query = format!("query={THREADS_QUERY}");
    let threads = fixture(&stdout(
        &rig,
        &[
            "api",
            "graphql",
            "-f",
            &query,
            "-f",
            &format!("owner={OWNER}"),
            "-f",
            &format!("name={NAME}"),
            "-F",
            "number=1",
        ],
    ));
    let recorded = fixture(include_str!(
        "../fixtures/threads_query_response_extended.json"
    ));
    assert_eq!(shape(&threads), shape(&recorded));

    let rows = |text: &str| -> Vec<Value> {
        serde_json::Deserializer::from_str(text)
            .into_iter::<Vec<Value>>()
            .flat_map(|page| page.unwrap())
            .collect()
    };
    let review = rows(&stdout(
        &rig,
        &[
            "api",
            &format!("repos/{FULL}/pulls/1/comments"),
            "--paginate",
        ],
    ));
    assert_eq!(review.len(), 2);
    within(
        &review,
        &rows(include_str!("../fixtures/rest_pull_review_comments.json")),
    );
    let issue = rows(&stdout(
        &rig,
        &[
            "api",
            &format!("repos/{FULL}/issues/1/comments"),
            "--paginate",
        ],
    ));
    within(
        &issue,
        &rows(include_str!("../fixtures/rest_issue_comments.json")),
    );
    assert_eq!(issue[0]["user"]["login"], "github-actions[bot]");
    assert_eq!(issue[0]["user"]["type"], "Bot");

    let permission = fixture(&stdout(
        &rig,
        &[
            "api",
            &format!("repos/{FULL}/collaborators/alice/permission"),
        ],
    ));
    within(
        &[permission],
        &[fixture(include_str!(
            "../fixtures/rest_collaborator_permission.json"
        ))],
    );

    // The CI log keeps the observed line shape, with the job's byte-order mark.
    let check = &view["statusCheckRollup"][0];
    let run = crate::host::gh_parse::actions_run_id(check["detailsUrl"].as_str().unwrap()).unwrap();
    let log = stdout(
        &rig,
        &[
            "run",
            "view",
            &run.to_string(),
            "--repo",
            FULL,
            "--log-failed",
        ],
    );
    let recorded = include_str!("../fixtures/run_view_log_failed.txt");
    let fields = |line: &str| line.split('\t').count();
    assert_eq!(
        log.lines().map(fields).collect::<Vec<_>>(),
        recorded
            .lines()
            .take(log.lines().count())
            .map(fields)
            .collect::<Vec<_>>()
    );

    // A rule concluding `Error` is a `StatusContext`, shaped as the recorded one.
    rig.ctl.set_ci(vec![CiRule::new("lint", Conclusion::Error)]);
    let view_args = ["pr", "view", "1", "--repo", FULL, "--json", PR_VIEW_FIELDS];
    let view = fixture(&stdout(&rig, &view_args));
    let recorded = fixture(include_str!(
        "../fixtures/status_check_rollup_status_context.json"
    ));
    assert_eq!(view["statusCheckRollup"][0]["__typename"], "StatusContext");
    assert_eq!(view["statusCheckRollup"][0]["state"], "ERROR");
    assert_eq!(
        shape(&view["statusCheckRollup"][0]),
        shape(&recorded["statusCheckRollup"][0])
    );

    // A merged PR's view, shaped as the recorded merged one.
    rig.ctl.merge(1, MergeMethodArg::Merge, false);
    let view = fixture(&stdout(&rig, &view_args));
    let recorded = fixture(include_str!("../fixtures/pr_view_merged.json"));
    for field in ["state", "mergeable"] {
        assert_eq!(view[field], recorded[field], "{field}");
    }
    for field in ["mergedAt", "mergeCommit"] {
        assert_eq!(shape(&view[field]), shape(&recorded[field]), "{field}");
    }
    assert_eq!(view["mergeCommit"]["oid"], rig.remote("main").unwrap());
    assert!(rig.ctl.forbidden().is_empty());
}
