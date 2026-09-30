//! Milestone 9.0.5 task 2: a round's latest action (`activity`, decision 3) and a
//! worker's last message (`last_text`, decision 4), both counters (decision 6).

use proto::{ACTIVITY_MAX, AgentRole};

use super::fixture::*;
use super::turns::working;
use crate::run::engine::{AgentSignal, Effect};
use crate::run::model::{AgentRound, Run};

fn tool(name: &str, target: &str) -> AgentSignal {
    AgentSignal::ToolUse {
        name: name.into(),
        target: Some(target.into()),
    }
}

fn said(text: &str) -> AgentSignal {
    AgentSignal::Said { text: text.into() }
}

fn latest(fx: &Fixture, role: AgentRole) -> &AgentRound {
    fx.task("t1")
        .rounds
        .iter()
        .rev()
        .find(|r| r.role == role)
        .expect("a round of the role")
}

#[test]
fn tool_use_sets_activity() {
    let (mut fx, window) = working();
    fx.signal(window, tool("Bash", "cargo test"));
    assert_eq!(
        latest(&fx, AgentRole::Worker).activity.as_deref(),
        Some("Bash cargo test")
    );
    // The anthrex server's prefix is dropped; a call with no summary is its name.
    fx.signal(window, tool("mcp__anthrex__task_done", ""));
    assert_eq!(
        latest(&fx, AgentRole::Worker).activity.as_deref(),
        Some("task_done")
    );
    // A sub-agent's call changes nothing; a tool call is not a message.
    fx.signal(
        window,
        AgentSignal::ToolUse {
            name: "Bash".into(),
            target: None,
        },
    );
    let round = latest(&fx, AgentRole::Worker);
    assert_eq!(round.activity.as_deref(), Some("task_done"));
    assert_eq!(round.last_text, None);
}

#[test]
fn said_sets_activity_to_its_first_line_and_last_text_for_workers() {
    let (mut fx, window) = working();
    let text = "\n   \n  Looking at the tests  \nthen the command";
    fx.signal(window, said(text));
    let round = latest(&fx, AgentRole::Worker);
    assert_eq!(
        round.activity.as_deref(),
        Some("says: Looking at the tests")
    );
    assert_eq!(round.last_text.as_deref(), Some(text));
    // The latest of the two wins.
    fx.signal(window, tool("Bash", "cargo test"));
    let round = latest(&fx, AgentRole::Worker);
    assert_eq!(round.activity.as_deref(), Some("Bash cargo test"));
    assert_eq!(
        round.last_text.as_deref(),
        Some(text),
        "text outlives a call"
    );

    // A reviewer's text is its activity, never a last message.
    let (mut fx, _, rwindow) = super::gates_review::reviewed(PROFILE, "");
    fx.signal(rwindow, said("Reading the diff"));
    let round = latest(&fx, AgentRole::Reviewer);
    assert_eq!(round.activity.as_deref(), Some("says: Reading the diff"));
    assert_eq!(round.last_text, None);
}

#[test]
fn activity_is_one_line_and_capped() {
    let hostile = format!("\u{1b}[2J\u{202e}{}\u{7}\r\nnext", "x".repeat(400));
    let clean = |activity: Option<&str>| {
        let activity = activity.expect("an activity");
        assert!(activity.chars().count() <= ACTIVITY_MAX, "{activity}");
        assert!(activity.ends_with('…'), "{activity}");
        assert!(!activity.contains('\u{1b}'), "{activity:?}");
        assert!(!activity.contains('\u{202e}'), "{activity:?}");
        assert!(!activity.chars().any(char::is_control), "{activity:?}");
    };
    let (mut fx, window) = working();
    fx.signal(window, said(&hostile));
    clean(latest(&fx, AgentRole::Worker).activity.as_deref());
    // The last message keeps its line break and loses the rest.
    let text = latest(&fx, AgentRole::Worker).last_text.clone().unwrap();
    assert!(text.ends_with("\nnext"), "{text:?}");
    assert!(
        !text.contains(['\u{1b}', '\u{202e}', '\u{7}', '\r']),
        "{text:?}"
    );

    fx.signal(window, tool("Bash", &hostile));
    clean(latest(&fx, AgentRole::Worker).activity.as_deref());
}

#[test]
fn activity_and_text_are_counters() {
    let (mut fx, window) = working();
    let digest = fx.run().orch.digest_rev;
    let effects = fx.signal(window, said("Added the stats command and its test."));
    let round = latest(&fx, AgentRole::Worker);
    assert!(round.activity.is_some() && round.last_text.is_some());
    let run_id = fx.run().id.clone();
    assert!(
        effects.contains(&Effect::Persist {
            run_id,
            urgent: false
        }),
        "{effects:#?}"
    );
    assert!(
        effects.contains(&Effect::Publish { structural: false }),
        "{effects:#?}"
    );
    assert_eq!(fx.run().orch.digest_rev, digest);
    // A tool call's activity is a counter too.
    let effects = fx.signal(window, tool("Read", "src/lib.rs"));
    assert!(
        effects.contains(&Effect::Publish { structural: false }),
        "{effects:#?}"
    );
    assert_eq!(fx.run().orch.digest_rev, digest);
}

/// A `run.json` written by milestone 9's code (this fixture's working run, serialized
/// before this task added the fields) loads, and gives back what it stored.
#[test]
fn a_run_json_without_the_new_fields_loads() {
    let text = include_str!("m9_run.json");
    let run: Run = serde_json::from_str(text).expect("an M9 run.json loads");
    let round = &run.tasks[0].rounds[0];
    assert_eq!(round.activity, None);
    assert_eq!(round.last_text, None);
    let mut back = serde_json::to_value(&run).unwrap();
    for task in back["tasks"].as_array_mut().unwrap() {
        for round in task["rounds"].as_array_mut().unwrap() {
            let round = round.as_object_mut().unwrap();
            for key in ["activity", "last_text"] {
                assert!(round.remove(key).is_some(), "{key}");
            }
        }
    }
    let stored: serde_json::Value = serde_json::from_str(text).unwrap();
    assert_eq!(back, stored);
}
