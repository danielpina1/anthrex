//! Task M9.2.4, fix round 1: `git push --porcelain` refusals other than `[rejected]` and
//! `[remote rejected]` (I2), scp-form redaction (m5), and a view's item cap (m6). Pure.

use serde_json::{Value, json};

use super::gh_parse;
use super::remote;
use super::runner::RunOutput;
use super::tests::{PR_VIEW_OPEN_RED, SHA};
use super::*;

const DST: &str = "refs/heads/anthrex/r1a2b/stage-1";

fn porcelain(flag: &str, summary: &str, success: bool) -> RunOutput {
    RunOutput {
        success,
        stdout: format!("To https://github.com/o/r.git\n{flag}\t{SHA}:{DST}\t{summary}\nDone\n")
            .into_bytes(),
        stderr: "error: failed to push some refs to 'https://github.com/o/r.git'\n".to_string(),
        cut: None,
    }
}

#[test]
fn only_rejected_and_remote_rejected_are_push_outcomes() {
    let outcome =
        |flag, summary, success| gh_parse::push_outcome(&porcelain(flag, summary, success), DST);
    assert_eq!(
        outcome("!", "[rejected] (non-fast-forward)", false),
        Ok(PushOutcome::Rejected {
            reason: "non-fast-forward".to_string()
        })
    );
    assert_eq!(
        outcome("!", "[rejected] (fetch first)", false),
        Ok(PushOutcome::Rejected {
            reason: "fetch first".to_string()
        })
    );
    assert_eq!(
        outcome("!", "[remote rejected] (protected branch hook declined)", false),
        Ok(PushOutcome::Refused {
            reason: "the remote refused the push of anthrex/r1a2b/stage-1: protected branch hook declined".to_string()
        })
    );
    // Anything else says nothing about the remote branch: an error, retried when due.
    for summary in [
        "[remote failure] (remote failed to report status)",
        "[no match]",
        "(delete of current branch prohibited)",
    ] {
        assert_eq!(
            outcome("!", summary, false),
            Err(HostError::Failed(format!(
                "git push of anthrex/r1a2b/stage-1 failed: {summary}"
            ))),
            "{summary}"
        );
    }
    assert_eq!(
        outcome("=", "[up to date]", true),
        Ok(PushOutcome::UpToDate)
    );
    assert!(matches!(
        outcome("+", "1111111...2222222 (forced update)", true),
        Err(HostError::Failed(_))
    ));
}

#[test]
fn redaction_hides_a_token_in_every_url_form() {
    for (url, shown) in [
        ("https://u:tok@github.com/o/r", "https://***@github.com/o/r"),
        ("https://tok@github.com/o/r", "https://***@github.com/o/r"),
        (
            "ssh://u:tok@github.com:22/o/r",
            "ssh://***@github.com:22/o/r",
        ),
        ("u:tok@github.com:o/r", "***@github.com:o/r"),
        ("git@github.com:o/r.git", "git@github.com:o/r.git"),
        ("/srv/git/r.git", "/srv/git/r.git"),
        ("a\tb\u{1b}[2Jc", "a\\tb\\u{1b}[2Jc"),
    ] {
        assert_eq!(remote::redact(url), shown, "{url}");
    }
}

#[test]
fn a_view_keeps_the_newest_items_of_each_class() {
    let n = VIEW_ITEMS_MAX as u64 + 50;
    let user = json!({ "__typename": "User", "login": "alice" });
    let reviews: Vec<Value> = (1..=n)
        .map(|id| json!({ "fullDatabaseId": id.to_string(), "state": "COMMENTED", "body": "", "author": user }))
        .collect();
    let comments: Vec<Value> = (1..=n)
        .map(|id| json!({ "fullDatabaseId": (1000 + id).to_string(), "body": "c", "author": user }))
        .collect();
    let threads: Vec<Value> = (1..=n)
        .map(|id| {
            json!({ "isResolved": false, "path": "a.rs", "line": 1, "comments": { "nodes": [
                { "fullDatabaseId": (5000 + id).to_string(), "body": "t", "diffHunk": "@@", "author": user }
            ] } })
        })
        .collect();
    let answer = json!({ "data": { "repository": { "pullRequest": {
        "reviewThreads": { "nodes": threads },
        "reviews": { "nodes": reviews },
        "comments": { "nodes": comments },
    } } } });
    let view = gh_parse::pr_view(1, PR_VIEW_OPEN_RED, &answer.to_string()).unwrap();
    let max = VIEW_ITEMS_MAX as u64;
    let ids: Vec<u64> = view.reviews.iter().map(|r| r.id).collect();
    assert_eq!(ids, (51..=n).rev().collect::<Vec<_>>());
    assert_eq!(ids.len() as u64, max);
    let ids: Vec<u64> = view.comments.iter().map(|c| c.id).collect();
    assert_eq!(ids, (1051..=1000 + n).rev().collect::<Vec<_>>());
    let ids: Vec<u64> = view.threads.iter().map(|t| t.comments[0].id).collect();
    assert_eq!(ids, (5051..=5000 + n).rev().collect::<Vec<_>>());
}
