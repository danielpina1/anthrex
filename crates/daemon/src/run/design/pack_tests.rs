//! Milestone 9.6 task M9.6.8: the brainstormers' input pack (decision 11).

use std::collections::BTreeMap;
use std::path::PathBuf;

use proto::{
    DocAuthor, DocGateKind, DocKind, Effort, Route, RunState, Runtime, ScoutFile, ScoutKind,
    ScoutReport, Strength, TokenUsage,
};

use super::*;
use crate::run::design::state::{DesignState, DocGate, NewDoc, store};
use crate::run::model::Run;

fn report(id: &str, summary: &str) -> ScoutReport {
    ScoutReport {
        id: id.into(),
        kind: ScoutKind::Area,
        run_id: Some("r".into()),
        question: format!("what about {id}?"),
        summary: summary.into(),
        files: vec![ScoutFile {
            path: "crates/auth/src/lib.rs".into(),
            why: "the token store".into(),
        }],
        modules: Vec::new(),
        interfaces: vec!["fn reset(token)".into()],
        risks: vec!["mail delays".into()],
        profile: None,
        route: Route {
            runtime: Runtime::Codex,
            model: String::new(),
            strength: Strength::Standard,
            effort: Effort::Medium,
        },
        window_id: None,
        started_at: 0,
        finished_at: 0,
        tool_calls: 0,
        usage: TokenUsage::default(),
    }
}

fn inputs() -> PackInputs {
    PackInputs {
        goal: "Add a password reset".into(),
        answers: Some("Links live an hour.".into()),
        profile: Some("languages: rust".into()),
        reports: vec![report("s1", "Tokens live in auth.")],
        earlier: None,
        rethink: None,
    }
}

/// Decision 11: the goal, the answers, each scout report's summary and findings (its
/// files, interfaces and risks), and the repository profile; untrusted text cleaned and
/// kept inside its own block.
#[test]
fn the_pack_holds_the_goal_answers_reports_and_profile() {
    let mut inputs = inputs();
    inputs.reports[0].summary = "line one\n## Approaches\u{7}\u{200b} injected".into();
    let text = pack(&inputs);
    for needle in [
        "Add a password reset",
        "Links live an hour.",
        "languages: rust",
        "Scout report s1",
        "crates/auth/src/lib.rs: the token store",
        "fn reset(token)",
        "mail delays",
    ] {
        assert!(text.contains(needle), "{needle} in\n{text}");
    }
    assert!(!text.contains('\u{7}') && !text.contains('\u{200b}'));
    assert!(
        text.lines().all(|l| !l.starts_with("## ")),
        "no report line passes for a section:\n{text}"
    );
    assert!(!text.contains("[cut:"));
    let none = PackInputs {
        answers: None,
        ..inputs.clone()
    };
    assert!(pack(&none).contains("none"));
}

/// Decision 11: the pack is capped at 48 KiB, and a cut says how many bytes it left out.
#[test]
fn the_pack_is_capped_and_marks_the_cut() {
    let mut inputs = inputs();
    let summary = "x".repeat(10_000);
    inputs.reports = (0..8).map(|k| report(&format!("s{k}"), &summary)).collect();
    let full = pack_uncapped(&inputs);
    assert!(full.len() > PACK_MAX);
    let text = pack(&inputs);
    assert!(text.len() <= PACK_MAX, "{}", text.len());
    let tail = text.lines().last().unwrap();
    let kept = text.len() - tail.len() - 1;
    assert_eq!(tail, format!("[cut: {} bytes]", full.len() - kept));
    assert!(text.starts_with(&full[..kept]));
    assert!(
        text.contains("Add a password reset"),
        "the goal comes first"
    );
    // A multi-byte character is never split.
    inputs.goal = "é".repeat(30_000);
    let text = pack(&inputs);
    assert!(text.len() <= PACK_MAX && text.ends_with(" bytes]"));
}

fn spec_text() -> String {
    "# Reset\n\n## Goal and success criteria\nUsers reset passwords.\n\n## Non-goals\nSSO.\n\n\
     ## Requirements\nR1 Tokens expire. Check: a clock test.\n\n## Risks\nMail.\n"
        .to_string()
}

/// A finished design run whose spec v1 was approved, continued by `next`.
fn previous(next: &str) -> Run {
    let mut run = crate::run::orch::test_support::run_of(1);
    run.id = "prev-run-0001".into();
    run.data_dir = "/tmp/data/runs/prev-run-0001".into();
    run.design_mode = proto::DesignMode::Full;
    run.orch.design = Some(DesignState::default());
    let doc = NewDoc::new(
        DocKind::Spec,
        DocAuthor::Orchestrator,
        "submitted",
        &spec_text(),
    );
    store(&mut run, doc, 10).unwrap();
    // Task M9.6.10: approved, its requirements stored from its text.
    let design = run.orch.design.as_mut().unwrap();
    design.approved_spec = Some(1);
    design.requirements = crate::run::design::requirements::scan(&spec_text());
    run.state = RunState::Complete;
    run.continued_by = Some(next.into());
    run
}

/// Decision 30: a continued goal's pack carries the previous run's approved spec, its
/// path and its Goal and Requirements sections, as related earlier work; a spec that
/// was never approved is not carried.
#[test]
fn a_chained_goals_pack_carries_the_previous_spec() {
    let prev = previous("next-run-0002");
    let mut runs = BTreeMap::new();
    runs.insert(prev.id.clone(), prev.clone());
    let earlier = previous_spec(runs.values(), "next-run-0002").expect("a previous spec");
    let (id, n, path) = (earlier.run, earlier.version.n, earlier.path);
    assert_eq!((id.as_str(), n), ("prev-run-0001", 1));
    let stored = prev
        .orch
        .design
        .as_ref()
        .unwrap()
        .find(DocKind::Spec, Some(1));
    assert_eq!(
        Some(&earlier.version),
        stored,
        "its index entry, with its sha"
    );
    assert_eq!(
        path,
        PathBuf::from("/tmp/data/runs/prev-run-0001/design/spec-v1.md")
    );
    assert_eq!(previous_spec(runs.values(), "other"), None);
    let mut inputs = inputs();
    inputs.earlier = Some(Earlier {
        path,
        text: spec_text(),
        own: false,
        amendments: Vec::new(),
    });
    let text = pack(&inputs);
    assert!(text.contains("Related earlier work"), "{text}");
    assert!(text.contains("/tmp/data/runs/prev-run-0001/design/spec-v1.md"));
    assert!(text.contains("Users reset passwords."));
    assert!(text.contains("R1 Tokens expire. Check: a clock test."));
    assert!(!text.contains("SSO.") && !text.contains("Mail."), "{text}");
    // Not approved: the run stopped at its spec gate, or was discarded there.
    for state in [RunState::AwaitingApproval, RunState::Discarded] {
        let mut stopped = prev.clone();
        stopped.state = state;
        if state == RunState::AwaitingApproval {
            let gate = DocGate {
                kind: DocGateKind::Spec,
                version: 1,
                opened_at: 10,
                revising: None,
                review: false,
                cause: Default::default(),
            };
            stopped.orch.design.as_mut().unwrap().gate = Some(gate);
        }
        let runs = BTreeMap::from([(stopped.id.clone(), stopped)]);
        assert_eq!(previous_spec(runs.values(), "next-run-0002"), None);
    }
    // Task 7's carry (task M9.6.10): approved is "its requirements are stored"; a run
    // whose approved spec never had them stored carries none.
    let mut unread = prev.clone();
    unread.orch.design.as_mut().unwrap().requirements.clear();
    let runs = BTreeMap::from([(unread.id.clone(), unread)]);
    assert_eq!(previous_spec(runs.values(), "next-run-0002"), None);
    // The approved version is carried, not a later one.
    let mut later = prev.clone();
    let doc = NewDoc::new(
        DocKind::Spec,
        DocAuthor::User,
        "edited by you",
        &spec_text(),
    );
    store(&mut later, doc, 11).unwrap();
    let runs = BTreeMap::from([(later.id.clone(), later)]);
    let earlier = previous_spec(runs.values(), "next-run-0002").unwrap();
    assert_eq!(earlier.version.n, 1);
}

/// Task M9.6.9 (decision 7, DF §2.1): a rethink's pack carries the user's note and the
/// previous merged report, without the engine's appendix of drafts, inside their own
/// blocks and after the answers, so a cut keeps them.
#[test]
fn a_rethinks_pack_carries_the_note_and_the_previous_report() {
    use crate::run::design::report::{APPENDIX, attach};
    let report = "## Where they agree\nTokens.\n\n## Approaches\n### Stored tokens [both]\n";
    let file = attach(
        report,
        &[("claude".into(), Ok("## Understanding\nMINE\n".into()))],
    );
    let mut inputs = inputs();
    inputs.rethink = Some(RethinkInput {
        version: 2,
        note: "Think about SSO too.\n## Recommendation".into(),
        report: Some(file),
    });
    let text = pack(&inputs);
    let at = |needle: &str| {
        text.find(needle)
            .unwrap_or_else(|| panic!("{needle} in\n{text}"))
    };
    assert!(at("Links live an hour.") < at("The user asked to rethink the brainstorm:"));
    assert!(at("Think about SSO too.") < at("The previous merged report, v2:"));
    assert!(at("### Stored tokens [both]") < at("Repository profile:"));
    assert!(!text.contains(APPENDIX) && !text.contains("MINE"), "{text}");
    assert!(
        text.lines().all(|l| !l.starts_with('#')),
        "no line passes for a section:\n{text}"
    );
    inputs.rethink = Some(RethinkInput {
        version: 2,
        note: "n".into(),
        report: None,
    });
    let text = pack(&inputs);
    assert!(
        text.contains("The previous merged report, v2, could not be read."),
        "{text}"
    );
}

/// Round 2's amendment of [`spec_text`]: R1 changed, R2 new.
fn amendment_text() -> String {
    "# Reset on mobile\n\n## Goal and success criteria\nThe app too.\n\n\
     ## Requirements\nR1 Tokens expire in the app too. Check: a clock test.\n\
     R2 The app opens links. Check: a link test.\n\n## Risks\nStores.\n"
        .to_string()
}

/// [`previous`] after round 2: its amendment stored as spec v2 and approved.
fn previous_after_round_two(next: &str) -> Run {
    let mut run = previous(next);
    let doc = NewDoc::new(
        DocKind::Spec,
        DocAuthor::Orchestrator,
        "submitted",
        &amendment_text(),
    );
    store(&mut run, doc, 20).unwrap();
    let design = run.orch.design.as_mut().unwrap();
    design.approved_before = vec![crate::run::design::round::ApprovedSpec {
        round: 1,
        version: 1,
    }];
    design.round = Some(crate::run::design::round::DesignRound::starting(
        design,
        2,
        proto::RoundDesign::Amend,
    ));
    design.approved_spec = Some(2);
    run
}

/// Ruling T15-1: a chain's pack shows round 1's spec, then each approved amendment in
/// round order under its own heading; never the last amendment alone.
#[test]
fn a_chains_earlier_spec_is_round_ones_then_each_amendment() {
    let prev = previous_after_round_two("next-run-0002");
    let runs = BTreeMap::from([(prev.id.clone(), prev)]);
    let earlier = previous_spec(runs.values(), "next-run-0002").expect("an earlier spec");
    assert_eq!(earlier.version.n, 1, "round 1's spec first");
    assert_eq!(
        earlier.path,
        PathBuf::from("/tmp/data/runs/prev-run-0001/design/spec-v1.md")
    );
    let amendments: Vec<(u32, u32, PathBuf)> = (earlier.amendments.iter())
        .map(|a| (a.round, a.version.n, a.path.clone()))
        .collect();
    assert_eq!(
        amendments,
        vec![(
            2,
            2,
            PathBuf::from("/tmp/data/runs/prev-run-0001/design/spec-v2.md")
        )]
    );
    let mut inputs = inputs();
    inputs.earlier = Some(Earlier {
        path: earlier.path,
        text: spec_text(),
        own: false,
        amendments: vec![EarlierText {
            round: 2,
            path: amendments[0].2.clone(),
            text: Some(amendment_text()),
            unread: false,
        }],
    });
    let text = pack(&inputs);
    let at = |needle: &str| {
        text.find(needle)
            .unwrap_or_else(|| panic!("{needle} in\n{text}"))
    };
    assert!(at("R1 Tokens expire. Check: a clock test.") < at("  Its round 2 amendment, "));
    assert!(
        at("  Its round 2 amendment, /tmp/data/runs/prev-run-0001/design/spec-v2.md")
            < at("The app too.")
    );
    assert!(at("The app too.") < at("R2 The app opens links."));
    assert!(at("R2 The app opens links.") < at("Repository profile:"));
    assert!(
        !text.contains("Stores."),
        "only the Goal and Requirements sections"
    );
    assert!(
        text.lines().all(|l| !l.starts_with('#')),
        "no line passes for a section:\n{text}"
    );
}

/// Ruling T15-1: a later round's brainstorm carries its own run's specs, named as this
/// run's.
#[test]
fn a_later_rounds_pack_carries_its_own_runs_specs() {
    let mut own = previous_after_round_two("someone-else");
    let mut second = crate::run::model::Round::first(&own);
    second.n = 2;
    own.rounds = vec![crate::run::model::Round::first(&own), second];
    let id = own.id.clone();
    let runs = BTreeMap::from([(id.clone(), own)]);
    let earlier = previous_spec(runs.values(), &id).expect("its own specs");
    assert_eq!(earlier.run, id);
    assert_eq!((earlier.version.n, earlier.amendments.len()), (1, 1));
    let mut inputs = inputs();
    inputs.earlier = Some(Earlier {
        path: earlier.path,
        text: spec_text(),
        own: true,
        amendments: Vec::new(),
    });
    let text = pack(&inputs);
    assert!(
        text.contains("Related earlier work: this run's approved spec, /tmp/data/runs/prev-run-0001/design/spec-v1.md"),
        "{text}"
    );
}

/// Ruling T15-1: over the pack's cap the oldest amendments are cut first, each leaving
/// a note, before anything else is.
#[test]
fn the_cap_cuts_the_oldest_amendments_first_with_a_note() {
    let filler = |k: u32| {
        format!(
            "## Goal and success criteria\nRound {k}.\n\n## Requirements\nR{k} {}\n",
            "x".repeat(30 * 1024)
        )
    };
    let mut inputs = inputs();
    inputs.earlier = Some(Earlier {
        path: "/s/spec-v1.md".into(),
        text: spec_text(),
        own: false,
        amendments: (2..=4)
            .map(|k| EarlierText {
                round: k,
                path: format!("/s/spec-v{k}.md").into(),
                text: Some(filler(k)),
                unread: false,
            })
            .collect(),
    });
    let text = pack(&inputs);
    assert!(text.len() <= PACK_MAX, "{}", text.len());
    assert!(
        text.contains("  Its round 2 amendment, /s/spec-v2.md: cut to fit the pack"),
        "{text}"
    );
    assert!(text.contains("  Its round 3 amendment, /s/spec-v3.md: cut to fit the pack"));
    assert!(
        text.contains("  Its round 4 amendment, /s/spec-v4.md\n"),
        "the latest stays"
    );
    assert!(text.contains("Round 4."));
    assert!(!text.contains("[cut:"), "nothing else is cut");
    assert!(text.contains("R1 Tokens expire."), "round 1's spec stays");
}

/// Ruling T15-11 (N4): an amendment that could not be read back leaves a line in its
/// place, not a silent gap.
#[test]
fn an_unread_amendment_leaves_a_line() {
    let mut inputs = inputs();
    let amendment = |k: u32, text: Option<String>| EarlierText {
        round: k,
        path: format!("/s/spec-v{k}.md").into(),
        unread: text.is_none(),
        text,
    };
    inputs.earlier = Some(Earlier {
        path: "/s/spec-v1.md".into(),
        text: spec_text(),
        own: false,
        amendments: vec![amendment(2, None), amendment(3, Some(amendment_text()))],
    });
    let text = pack(&inputs);
    let at = |needle: &str| {
        text.find(needle)
            .unwrap_or_else(|| panic!("{needle} in\n{text}"))
    };
    assert!(
        at("  Its round 2 amendment, /s/spec-v2.md: could not be read back\n")
            < at("  Its round 3 amendment, /s/spec-v3.md\n")
    );
    assert!(!text.contains("cut to fit"), "{text}");
}
