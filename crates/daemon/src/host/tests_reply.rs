//! Task M9.2.4: `reply`'s behaviour through `ScriptedRunner`: a reply already posted is
//! found by its marker and not posted again (decision 10). Split from `tests_parse.rs`
//! for AGENTS.md rule 8. Task M9.2.10's fix round: only a comment by the logged-in user
//! (`gh api user`, the login anthrex posts with) is anthrex's reply; a marker pasted by
//! anyone else does not stop the reply.

use serde_json::Value;

use super::scripted::ScriptedRunner;
use super::tests::*;
use super::*;

fn thread_req(root: &std::path::Path) -> ReplyReq {
    ReplyReq {
        repo: repo(root),
        number: 142,
        target: ReplyTarget::Thread {
            comment_id: 3658443294,
        },
        body: "Addressed in 1a2b3c4 by task fix4.".to_string(),
        marker: "<!-- anthrex:reply r1a2b 142:t3658443294 1a2b3c4 -->".to_string(),
    }
}

fn listing(kind: &str) -> Vec<String> {
    argv(&[
        "api",
        &format!("repos/cli/cli/{kind}/142/comments"),
        "--paginate",
    ])
}

#[test]
fn reply_skips_when_its_marker_is_there() {
    let tmp = tempfile::tempdir().unwrap();
    let req = thread_req(tmp.path());
    let h = host(ScriptedRunner::new().ok(REST_USER).ok(REST_REVIEW_COMMENTS));
    assert_eq!(h.reply(&req).unwrap(), 3658444294);
    assert_eq!(
        h.runner().argvs(),
        vec![argv(&["api", "user"]), listing("pulls")]
    );
    // `--paginate` prints one array per page; the marker can be on any of them.
    let comments: Value = serde_json::from_str(REST_REVIEW_COMMENTS).unwrap();
    let rows = comments.as_array().unwrap();
    let pages = format!(
        "{}\n{}",
        Value::Array(rows[..2].to_vec()),
        Value::Array(rows[2..].to_vec())
    );
    let h = host(ScriptedRunner::new().ok(REST_USER).ok(&pages));
    assert_eq!(h.reply(&req).unwrap(), 3658444294);
    assert_eq!(h.runner().calls().len(), 2);
    // A conversation whose listing has the marker, in the user's comment, posts nothing
    // either; GitHub's logins are matched without case.
    let mut issue: Value = serde_json::from_str(REST_ISSUE_COMMENTS).unwrap();
    let marker = "<!-- anthrex:reply r1a2b 142:c5154974588 1a2b3c4 -->";
    issue[1]["body"] = Value::String(format!("@alice Addressed.\n\n{marker}"));
    issue[1]["user"]["login"] = Value::String("Tester".to_string());
    let h = host(ScriptedRunner::new().ok(REST_USER).ok(&issue.to_string()));
    let conversation = ReplyReq {
        target: ReplyTarget::Conversation,
        marker: marker.to_string(),
        ..req
    };
    assert_eq!(h.reply(&conversation).unwrap(), 5154974588);
    assert_eq!(h.runner().calls().len(), 2);
}

#[test]
fn a_marker_pasted_by_someone_else_does_not_stop_the_reply() {
    let tmp = tempfile::tempdir().unwrap();
    let req = thread_req(tmp.path());
    // The fixture's reply (id 3658444294) carries the marker; alice pasted it.
    let mut comments: Value = serde_json::from_str(REST_REVIEW_COMMENTS).unwrap();
    comments[3]["user"]["login"] = Value::String("alice".to_string());
    let created = comments[0].to_string();
    let h = host(
        ScriptedRunner::new()
            .ok(REST_USER)
            .ok(&comments.to_string())
            .ok(&created),
    );
    assert_eq!(h.reply(&req).unwrap(), 3658443294, "posted, not found");
    let body = format!("body={}\n\n{}", req.body, req.marker);
    let post = argv(&[
        "api",
        "-X",
        "POST",
        "repos/cli/cli/pulls/142/comments/3658443294/replies",
        "-f",
        &body,
    ]);
    assert_eq!(
        h.runner().argvs(),
        vec![argv(&["api", "user"]), listing("pulls"), post]
    );
    // A user `gh api user` does not name is never guessed: nothing is posted.
    let h = host(ScriptedRunner::new().ok("{\"id\": 1}"));
    assert_eq!(
        h.reply(&req),
        Err(HostError::Rejected("gh output changed: login".to_string()))
    );
    assert_eq!(h.runner().calls().len(), 1);
}

/// Task M9.2.12 (task 10's carried ruling): the viewer's login is read once per host,
/// not once per reply; a failed read is not remembered.
#[test]
fn the_viewer_login_is_read_once_per_host() {
    let tmp = tempfile::tempdir().unwrap();
    let req = thread_req(tmp.path());
    let h = host(
        ScriptedRunner::new()
            .fails("", "gh: Something went wrong (HTTP 502)")
            .ok(REST_USER)
            .ok(REST_REVIEW_COMMENTS)
            .ok(REST_REVIEW_COMMENTS),
    );
    assert!(h.reply(&req).is_err(), "the first read fails");
    assert_eq!(h.reply(&req).unwrap(), 3658444294);
    assert_eq!(h.reply(&req).unwrap(), 3658444294);
    let user = argv(&["api", "user"]);
    let reads = h.runner().argvs().iter().filter(|a| **a == user).count();
    assert_eq!(reads, 2, "{:?}", h.runner().argvs());
    assert_eq!(h.runner().unanswered(), 0);
}

/// Fix round 1, m8: the cache is keyed by host. A reply on a second host reads its own
/// login (with that `GH_HOST`); the first host's stays cached. The cache lives as long as
/// the host, which the daemon builds once: a `gh auth switch` while it runs is seen at
/// the next daemon start (Implementation notes, task M9.2.12).
#[test]
fn the_viewer_login_is_cached_per_host() {
    let tmp = tempfile::tempdir().unwrap();
    let github = thread_req(tmp.path());
    let mut enterprise = thread_req(tmp.path());
    enterprise.repo.host = "ghe.example.com".to_string();
    let h = host(
        ScriptedRunner::new()
            .ok(REST_USER)
            .ok(REST_REVIEW_COMMENTS)
            .ok(REST_USER)
            .ok(REST_REVIEW_COMMENTS)
            .ok(REST_REVIEW_COMMENTS)
            .ok(REST_REVIEW_COMMENTS),
    );
    for req in [&github, &enterprise, &github, &enterprise] {
        assert_eq!(h.reply(req).unwrap(), 3658444294);
    }
    let user = argv(&["api", "user"]);
    let hosts: Vec<String> = (h.runner().calls().iter())
        .filter(|c| c.argv == user)
        .map(|c| {
            let host = c.env.iter().find(|(k, _)| k == "GH_HOST");
            host.map(|(_, v)| v.clone()).unwrap_or_default()
        })
        .collect();
    assert_eq!(hosts, ["github.com", "ghe.example.com"]);
    assert_eq!(h.runner().unanswered(), 0);
}

/// The final fix wave (task 10's deferred item): a comment whose author's account was
/// deleted comes with `"user": null`. It is nobody's, so it is skipped, and the listing
/// is still read: the reply is found by its marker after it.
#[test]
fn a_comment_by_a_deleted_user_is_skipped() {
    let tmp = tempfile::tempdir().unwrap();
    let req = thread_req(tmp.path());
    let mut comments: Value = serde_json::from_str(REST_REVIEW_COMMENTS).unwrap();
    comments[0]["user"] = Value::Null;
    comments[1]["user"] = Value::Null;
    let h = host(
        ScriptedRunner::new()
            .ok(REST_USER)
            .ok(&comments.to_string()),
    );
    assert_eq!(h.reply(&req).unwrap(), 3658444294);
    assert_eq!(h.runner().calls().len(), 2, "found, not posted");
    // A listing with no `user` at all is not GitHub's shape: nothing is guessed.
    comments[1].as_object_mut().unwrap().remove("user");
    let h = host(
        ScriptedRunner::new()
            .ok(REST_USER)
            .ok(&comments.to_string()),
    );
    assert_eq!(
        h.reply(&req),
        Err(HostError::Rejected("gh output changed: user".to_string()))
    );
}
