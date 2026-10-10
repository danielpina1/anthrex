//! `profile_cmd.rs`'s unit tests: `profile status`'s text, the row edit's line, the
//! sanitised daemon text and `edit --anyway`'s request. Split out by responsibility
//! (`AGENTS.md` rule 8).

use super::*;
use proto::{DroppedGoal, ProfileSource, QueuedGoalInfo, RowEdit, SetupState};

fn status() -> ProfileStatus {
    ProfileStatus {
        project: PathBuf::from("/work/app"),
        repo_dir: PathBuf::from("/data/repos/app"),
        source: ProfileSource::Stored,
        confirmed_at: Some(900),
        stale: Vec::new(),
        unparseable: None,
        proposal: None,
        scout: None,
        verify_confined: true,
        queued: Vec::new(),
        checking: None,
        verified_at: Some(940),
        unreadable_text: None,
        dropped_goals: Vec::new(),
    }
}

fn queued(goal: &str) -> QueuedGoalInfo {
    QueuedGoalInfo {
        id: "q1".into(),
        project: PathBuf::from("/work/app"),
        goal: goal.into(),
        queued_at: 950,
        yes: false,
        trust_project: false,
        unconfined_checks: false,
        setup: SetupState::NeedsReview,
    }
}

#[test]
fn status_text_has_the_status_line_second() {
    let mut s = status();
    s.queued = vec![queued("add a")];
    s.dropped_goals = vec![DroppedGoal {
        goal: "add b".into(),
        reason: "the profile was rejected".into(),
        at: 960,
    }];
    let text = status_text(&s, 1000);
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines[0], "profile: /work/app");
    assert_eq!(lines[1], "  Ready · verified 1m ago");
    assert!(lines[2].starts_with("  stored: yes, confirmed "), "{text}");
    assert!(text.contains("  detection: none\n"), "{text}");
    assert!(text.contains("  verification: confined, as runs are\n"));
    let tail = &lines[lines.len() - 2..];
    assert_eq!(tail[0], "  waiting: add a");
    assert_eq!(
        tail[1],
        "  dropped goal \"add b\": the profile was rejected"
    );
}

#[test]
fn status_text_draws_daemon_text_safely() {
    let mut s = status();
    s.project = PathBuf::from("/work/\u{1b}[31mapp");
    s.stale = vec!["a\u{1b}[2Jb".into(), "x\ny".into()];
    s.queued = vec![queued("go\n  stored: forged\u{1b}]0;t\u{7}\u{202e}")];
    s.dropped_goals = vec![DroppedGoal {
        goal: "g\r\n  waiting: forged".into(),
        reason: "r\u{1b}[0m\nline".into(),
        at: 1,
    }];
    s.unparseable = Some("bad\u{1b}[1m toml".into());
    let text = status_text(&s, 1000);
    assert!(
        !text.chars().any(|c| c.is_control() && c != '\n'),
        "{text:?}"
    );
    assert!(!text.contains('\u{202e}'), "{text:?}");
    // A goal or reason cannot start a line of its own.
    for line in text.lines() {
        assert!(
            !line.starts_with("  stored: forged") && !line.starts_with("  waiting: forged"),
            "{text:?}"
        );
    }
    assert_eq!(
        text.lines().filter(|l| l.starts_with("  waiting:")).count(),
        1
    );
}

/// A status whose proposal (of `origin`) holds a row edit of `check` in `state`.
fn with_edit(origin: ProposalOrigin, value: Option<&str>, state: RowEditState) -> ProfileStatus {
    let mut s = status();
    s.proposal = Some(ProposalRecord {
        project: PathBuf::from("/work/app"),
        state: ProposalState::Ready,
        origin,
        started_at: 900,
        updated_at: 900,
        base_sha: String::new(),
        scout_id: None,
        window_id: None,
        profile: None,
        verification: None,
        dropped: vec![],
        proposed: None,
        trusted_project: vec![],
        unconfined_checks: false,
        auto_confirm: false,
        edit: Some(RowEdit {
            key: "check".into(),
            value: value.map(str::to_string),
            state,
        }),
    });
    s
}

fn failed(reason: &str) -> RowEditState {
    RowEditState::Failed {
        reason: reason.into(),
        tail: String::new(),
        secs: 3,
    }
}

/// Final review C-I2: a row edit is one line after the detection line: its key, its
/// value, its state and, after a ✗, its reason and what to do.
#[test]
fn status_text_prints_the_row_edit() {
    let edit = || ProposalOrigin::Edit {
        keys: vec!["check".into()],
    };
    let line = |s: &ProfileStatus| {
        let text = status_text(s, 1000);
        let lines: Vec<String> = text.lines().map(str::to_string).collect();
        let at = lines
            .iter()
            .position(|l| l.starts_with("  detection:"))
            .unwrap();
        lines[at + 1].clone()
    };
    let checking = with_edit(edit(), Some("\"make test\""), RowEditState::Verifying);
    assert_eq!(line(&checking), "  edit: check = make test, checking");
    let held = with_edit(edit(), Some("false"), failed("exit 1 after 0s"));
    assert_eq!(
        line(&held),
        "  edit: check = false failed its check: exit 1 after 0s \
         (save it anyway with --anyway, or anthrex profile reject)"
    );
    let review = with_edit(ProposalOrigin::Detect, Some("false"), failed("exit 1"));
    assert_eq!(
        line(&review),
        "  edit: check = false failed its check: exit 1 \
         (not in the proposal; anthrex profile use stores the proposal without it)"
    );
    let unset = with_edit(edit(), None, RowEditState::Verifying);
    assert_eq!(line(&unset), "  edit: unset check, checking");
    // No row edit: no line.
    assert!(!status_text(&status(), 1000).contains("  edit:"));
}

/// Final review C-I2 and task 11 review M1: every daemon-supplied field is one line
/// of its own; a newline in any of them never starts a forged line.
#[test]
fn status_text_keeps_every_field_on_its_line() {
    let forged = "x\n  forged: line\r\n  forged: too";
    let mut s = with_edit(
        ProposalOrigin::Edit {
            keys: vec!["check".into()],
        },
        Some(forged),
        failed(forged),
    );
    s.project = PathBuf::from(forged);
    s.repo_dir = PathBuf::from(forged);
    s.stale = vec![forged.into(), "b".into()];
    if let Some(p) = s.proposal.as_mut() {
        p.scout_id = Some(forged.into());
        p.window_id = Some(3);
        p.state = ProposalState::Scouting;
    }
    s.queued = vec![queued(forged)];
    s.dropped_goals = vec![DroppedGoal {
        goal: forged.into(),
        reason: forged.into(),
        at: 1,
    }];
    let text = status_text(&s, 1000);
    assert!(
        text.lines().all(|l| !l.trim_start().starts_with("forged")),
        "{text}"
    );
    s.unparseable = Some(forged.into());
    let text = status_text(&s, 1000);
    assert!(
        text.lines().all(|l| !l.trim_start().starts_with("forged")),
        "{text}"
    );
}

/// Final review M10: `show`'s TOML (its comments carry scout-proposed commands) and
/// the daemon's `Done` and `Refused` texts are printed sanitised.
#[test]
fn printed_daemon_text_is_sanitised() {
    let evil = "a\u{1b}[2Jb\u{1b}]0;t\u{7}\u{202e}c\n# next";
    let clean = |text: &str| {
        assert!(
            !text.chars().any(|c| c.is_control() && c != '\n') && !text.contains('\u{202e}'),
            "{text:?}"
        );
    };
    let done = done_text(ProfileReply::Done {
        message: evil.into(),
    });
    clean(&done.unwrap());
    let refused = done_text(ProfileReply::Refused {
        message: evil.into(),
    });
    clean(&refused.unwrap_err().to_string());
    let shown = ProfileReply::Shown {
        source: ProfileSource::Stored,
        toml: format!("check = \"x\"\n# verification: {evil}\n"),
        meta: None,
        verification: None,
        dropped: vec![],
    };
    let text = show_text(shown, false).unwrap();
    clean(&text);
    assert!(
        text.contains("\n# next"),
        "the TOML keeps its lines: {text:?}"
    );
    clean(&error_text(&anyhow::anyhow!(evil.to_string())));
}

#[test]
fn edit_anyway_sends_the_request_with_yes() {
    let request = edit_request(
        PathBuf::from("/work/app"),
        "check".into(),
        Some("false".into()),
        false,
        true,
        false,
    );
    assert_eq!(
        request,
        ProfileRequest::Edit {
            dir: PathBuf::from("/work/app"),
            key: "check".into(),
            value: Some("false".into()),
            yes: true,
            unconfined_checks: false,
            anyway: true,
            on_proposal: false,
        }
    );
}
