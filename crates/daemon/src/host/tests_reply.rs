//! Task M9.2.4: `reply`'s behaviour through `ScriptedRunner`: a reply already posted is
//! found by its marker and not posted again (decision 10). Split from `tests_parse.rs`
//! for AGENTS.md rule 8.

use serde_json::Value;

use super::scripted::ScriptedRunner;
use super::tests::*;
use super::*;

#[test]
fn reply_skips_when_its_marker_is_there() {
    let tmp = tempfile::tempdir().unwrap();
    let req = ReplyReq {
        repo: repo(tmp.path()),
        number: 142,
        target: ReplyTarget::Thread {
            comment_id: 3658443294,
        },
        body: "Addressed in 1a2b3c4 by task fix4.".to_string(),
        marker: "<!-- anthrex:reply r1a2b 142:t3658443294 1a2b3c4 -->".to_string(),
    };
    let h = host(ScriptedRunner::new().ok(REST_REVIEW_COMMENTS));
    assert_eq!(h.reply(&req).unwrap(), 3658444294);
    assert_eq!(
        h.runner().argvs(),
        vec![argv(&[
            "api",
            "repos/cli/cli/pulls/142/comments",
            "--paginate"
        ])]
    );
    // `--paginate` prints one array per page; the marker can be on any of them.
    let comments: Value = serde_json::from_str(REST_REVIEW_COMMENTS).unwrap();
    let rows = comments.as_array().unwrap();
    let pages = format!(
        "{}\n{}",
        Value::Array(rows[..2].to_vec()),
        Value::Array(rows[2..].to_vec())
    );
    let h = host(ScriptedRunner::new().ok(&pages));
    assert_eq!(h.reply(&req).unwrap(), 3658444294);
    assert_eq!(h.runner().calls().len(), 1);
    // A conversation whose listing has the marker posts nothing either.
    let mut issue: Value = serde_json::from_str(REST_ISSUE_COMMENTS).unwrap();
    let marker = "<!-- anthrex:reply r1a2b 142:c5154974588 1a2b3c4 -->";
    issue[1]["body"] = Value::String(format!("@alice Addressed.\n\n{marker}"));
    let h = host(ScriptedRunner::new().ok(&issue.to_string()));
    let conversation = ReplyReq {
        target: ReplyTarget::Conversation,
        marker: marker.to_string(),
        ..req
    };
    assert_eq!(h.reply(&conversation).unwrap(), 5154974588);
    assert_eq!(h.runner().calls().len(), 1);
}
