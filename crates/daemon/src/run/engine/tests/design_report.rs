//! Milestone 9.6 task M9.6.9: the merged brainstorm report (DF §3.4, §3.5, decision 13)
//! and rethink (decision 7). The report is refused until the drafts are in; it is
//! checked (tags, recommendation, the single-brainstorm line), stored with the drafts
//! attached as its appendix, and opens the brainstorm gate; a rethink relaunches both
//! brainstormers with the user's note and that report, and the next report is the gate's
//! next version. The brainstormers' sessions are stubbed by their ops' results and
//! their ended events (`design_agents.rs`'s helpers).

use proto::{AgentRole, DocGateAction, DocGateKind, DocKind, RunState, ToolCall};
use serde_json::json;

use super::design_agents::*;
use super::design_fixture::*;
use super::fixture::*;
use super::orch::{ORCH, orch_tool};
use crate::run::design::pack::FrozenRethink;
use crate::run::design::report::{APPENDIX, attach};
use crate::run::design::state::{self, DesignAgentState, DocVersion, sha256_hex};
use crate::run::engine::{Effect, EventKind, OrchEvent, ScoutEnd};

pub(super) const NOT_IN: &str = "the brainstorm drafts are not in yet";

/// Codex's draft, told apart from Claude's.
pub(super) fn codex_draft() -> String {
    DRAFT.replace("A table. Size M.", "A table, from codex. Size M.")
}

/// Both brainstormers' drafts in, their sessions ended.
pub(super) fn both_drafts(fx: &mut Fixture) {
    let session = |fx: &Fixture, k: usize| agents(fx)[k].session;
    let (claude, codex) = (session(fx, 0), session(fx, 1));
    let window = |fx: &Fixture, k: usize| agents(fx)[k].window_id.unwrap();
    let (a, b) = (window(fx, 0), window(fx, 1));
    assert!(answered(&submit_draft(fx, a, DRAFT)).0);
    assert!(answered(&submit_draft(fx, b, &codex_draft())).0);
    ended(fx, "claude", claude, ScoutEnd::Reported);
    ended(fx, "codex", codex, ScoutEnd::Reported);
}

/// Every design document the effects write: `(path, text)`.
pub(super) fn writes(effects: &[Effect]) -> Vec<(std::path::PathBuf, String)> {
    (effects.iter())
        .filter_map(|e| match e {
            Effect::WriteDoc { path, text, .. } => Some((path.clone(), text.clone())),
            _ => None,
        })
        .collect()
}

fn version(fx: &Fixture, kind: DocKind, n: u32) -> DocVersion {
    let design = fx.run().orch.design.as_ref().unwrap();
    design.find(kind, Some(n)).cloned().unwrap()
}

/// DF §3.4 and decision 13: the report opens the brainstorm gate at v1; its written
/// file is the report with both drafts attached under `## Appendix: the drafts`, and
/// the version's length and SHA-256 are that file's. The orchestrator's reply carries
/// no appendix. Task 5's concern 2: v1's change summary is empty, nothing is disputed or
/// unreviewed, and the gate shows the report's counts and tags.
#[test]
fn the_merged_report_opens_gate_one_with_the_appendix_attached() {
    let mut fx = brainstorming();
    both_drafts(&mut fx);
    let effects = submit(&mut fx, "brainstorm", REPORT);
    let reply = match &super::dispatch::replies(&effects)[..] {
        [Ok(text)] => text.clone(),
        other => panic!("{other:?}"),
    };
    assert!(!reply.contains(APPENDIX), "{reply}");
    let value: serde_json::Value = serde_json::from_str(&reply).unwrap();
    assert_eq!(value["version"], 1);
    assert_eq!(gate(&fx), Some((DocGateKind::Brainstorm, 1, None)));
    let drafts = vec![
        ("claude".to_string(), Ok(DRAFT.to_string())),
        ("codex".to_string(), Ok(codex_draft())),
    ];
    let expected = attach(REPORT, &drafts);
    let dir = state::design_dir(fx.run());
    assert_eq!(
        writes(&effects),
        [(dir.join("brainstorm-v1.md"), expected.clone())]
    );
    let v1 = version(&fx, DocKind::Brainstorm, 1);
    assert_eq!(v1.bytes, expected.len() as u64);
    assert_eq!(v1.sha256, sha256_hex(expected.as_bytes()));
    assert_eq!(
        (&v1.changes, &v1.disputed, &v1.not_reviewed, v1.same_runtime),
        (&Vec::new(), &Vec::new(), &None, false)
    );
    let info = crate::run::snapshot::snapshot(&fx.state, fx.now).runs[0]
        .doc_gate
        .clone()
        .unwrap();
    let summary = info.report.expect("the report's summary");
    assert_eq!((summary.agree, summary.disagree), (1, 1));
    let tags: Vec<(String, String)> = (summary.approaches.into_iter())
        .map(|a| (a.name, a.tag))
        .collect();
    assert_eq!(
        tags,
        [
            ("Signed tokens".into(), "claude".into()),
            ("Stored tokens".into(), "both".into())
        ]
    );
    assert!(
        notes(&fx).iter().all(|n| !n.contains(APPENDIX)),
        "{:?}",
        notes(&fx)
    );

    // v2 after the user's changes: its summary compares the reports, not the appendix.
    let changes = DocGateAction::Changes {
        note: "Say more.".into(),
        review: false,
    };
    act(&mut fx, DocGateKind::Brainstorm, changes).unwrap();
    let text = REPORT.replace("Both want tokens.", "Both want tokens.\nAnd mail.");
    submitted(&mut fx, "brainstorm", &text);
    let v2 = version(&fx, DocKind::Brainstorm, 2);
    assert_eq!(v2.changes, ["~ Where they agree: 1 line"]);
}

/// The carry from task 7's review (ruling T8-5): the orchestrator's report is refused,
/// exactly, until both drafts are in, or one draft and the other brainstormer's failure.
#[test]
fn the_report_is_refused_until_the_drafts_are_in() {
    let mut fx = design_launched(false);
    assert_eq!(refused(&submit(&mut fx, "brainstorm", REPORT)), NOT_IN);
    let mut fx = brainstorming();
    assert!(answered(&submit_draft(&mut fx, CLAUDE, DRAFT)).0);
    assert_eq!(refused(&submit(&mut fx, "brainstorm", REPORT)), NOT_IN);
    let design = fx.run().orch.design.as_ref().unwrap();
    assert!(design.versions.is_empty() && design.gate.is_none());
    assert!(answered(&submit_draft(&mut fx, CODEX, &codex_draft())).0);
    assert_eq!(submitted(&mut fx, "brainstorm", REPORT)["version"], 1);
}

/// The carry from task 4's review: the template the orchestrator receives with the
/// drafts-in wake says each approach is a `### <name> [tag]` heading under
/// `## Approaches`, with the run's labels.
#[test]
fn the_drafts_in_wake_carries_the_reports_template() {
    let mut fx = brainstorming();
    both_drafts(&mut fx);
    let template = notes(&fx)
        .into_iter()
        .find(|n| n.starts_with("the merged report's template:"))
        .unwrap_or_else(|| panic!("{:?}", notes(&fx)));
    for needle in [
        "## Where they agree",
        "## Where they disagree",
        "## Approaches",
        "### <name> [claude]",
        "[codex] or [both]",
        "## Recommendation",
        "## Questions for you",
    ] {
        assert!(template.contains(needle), "{needle} in {template}");
    }
    assert!(!template.contains("single brainstorm"), "{template}");
}

/// Ruling T4-3 and DF §3.4, through the submit: an untagged approach and a
/// recommendation naming no listed approach are refused exactly, and nothing is stored.
#[test]
fn an_untagged_approach_or_an_unlisted_recommendation_is_refused() {
    let mut fx = design_launched(false);
    drafts_in(&mut fx);
    let untagged = REPORT.replace("### 1. Signed tokens [claude]", "### 1. Signed tokens");
    assert_eq!(
        refused(&submit(&mut fx, "brainstorm", &untagged)),
        "approach \"1. Signed tokens\" has no [claude], [codex] or [both] tag"
    );
    let unlisted = REPORT.replace(
        "Go with Stored tokens, because links must be revocable.",
        "Go with magic links.",
    );
    assert_eq!(
        refused(&submit(&mut fx, "brainstorm", &unlisted)),
        "the recommendation must name one of the listed approaches"
    );
    assert_eq!(fx.run().state, RunState::Brainstorming);
    let design = fx.run().orch.design.as_ref().unwrap();
    assert_eq!(design.find(DocKind::Brainstorm, None), None);
    assert_eq!(submitted(&mut fx, "brainstorm", REPORT)["version"], 1);
}

/// DF §3.5: with one brainstormer failed, the report must begin with the line naming
/// it, as the engine expects it (its label); the wake's template says so; the appendix
/// names the failed one with its reason.
#[test]
fn a_single_brainstorm_report_must_name_the_failure() {
    let mut fx = brainstorming();
    assert!(answered(&submit_draft(&mut fx, CLAUDE, DRAFT)).0);
    let reason = "the brainstormer ran longer than 900 s";
    let failed = ScoutEnd::Failed {
        reason: reason.into(),
    };
    ended(&mut fx, "codex", 1, failed);
    let line = format!("single brainstorm: codex failed: {reason}");
    let template = (notes(&fx).into_iter())
        .find(|n| n.starts_with("the merged report's template:"))
        .unwrap();
    assert!(
        template.contains(&format!("begin with the line \"{line}\"")),
        "{template}"
    );
    let expected = format!("a single brainstorm must begin \"{line}\"");
    assert_eq!(refused(&submit(&mut fx, "brainstorm", REPORT)), expected);
    let wrong = format!("single brainstorm: claude failed: {reason}\n\n{REPORT}");
    assert_eq!(refused(&submit(&mut fx, "brainstorm", &wrong)), expected);
    // The engine checks the label; the orchestrator may word the reason.
    let named = format!("single brainstorm: codex failed: it ran out of time\n\n{REPORT}");
    let effects = submit(&mut fx, "brainstorm", &named);
    let drafts = vec![
        ("claude".to_string(), Ok(DRAFT.to_string())),
        ("codex".to_string(), Err(reason.to_string())),
    ];
    let file = &writes(&effects)[0].1;
    assert_eq!(file, &attach(&named, &drafts));
    assert!(
        file.ends_with(&format!("### codex\n\n(no draft: {reason})\n")),
        "{file}"
    );
}

/// Decision 7, DF §2.1, ruling T8-5 and T8-6: a rethink returns the run to
/// brainstorming and relaunches both brainstormers as new sessions; the round's pack is
/// frozen anew (the same scout reports, plus the user's note and the report it
/// replaces) and written to its own file; the drafts settle again, written beside the
/// first round's without replacing them; the next report opens the gate's v2, with the
/// new drafts attached.
#[test]
fn rethink_relaunches_both_with_the_note_and_reopens_a_new_version() {
    let mut fx = brainstorming();
    both_drafts(&mut fx);
    submitted(&mut fx, "brainstorm", REPORT);
    let v1 = version(&fx, DocKind::Brainstorm, 1);
    let before = fx.run().orch.design.as_ref().unwrap().pack.clone().unwrap();
    fx.run_mut().scout_reports.push("late".into());
    let rethink = DocGateAction::Rethink {
        note: "Think about SSO.".into(),
    };
    act(&mut fx, DocGateKind::Brainstorm, rethink).unwrap();
    assert_eq!(fx.run().state, RunState::Brainstorming);
    assert_eq!(gate(&fx), None);
    let design = fx.run().orch.design.as_ref().unwrap();
    assert!(!design.drafts_settled);
    let pack = design.pack.clone().unwrap();
    let path = state::design_dir(fx.run()).join("brainstorm-v1.md");
    assert_eq!(pack.reports, before.reports, "the same scout reports");
    assert_eq!((pack.round, &pack.file), (2, &None));
    assert_eq!(
        pack.rethink,
        Some(FrozenRethink {
            note: "Think about SSO.".into(),
            path,
            version: v1,
        })
    );
    // Both relaunch at once, each as its next session.
    let relaunched: Vec<(String, u32)> = (launches(&fx).into_iter().skip(2))
        .map(|(_, s)| (s.kind.label(), s.session))
        .collect();
    assert_eq!(relaunched, [("claude".into(), 2), ("codex".into(), 2)]);
    assert_eq!(
        states(&fx),
        [DesignAgentState::Running, DesignAgentState::Running]
    );
    assert_eq!(refused(&submit(&mut fx, "brainstorm", REPORT)), NOT_IN);
    started(&mut fx, "claude", CLAUDE + 10);
    started(&mut fx, "codex", CODEX + 10);
    let round_two = DRAFT.replace("Stored tokens.\n", "Stored tokens, with SSO.\n");
    let effects = submit_draft(&mut fx, CLAUDE + 10, &round_two);
    assert!(answered(&effects).0);
    // Ruling T8-1 in round 2 too: held, not written, while the other runs.
    assert!(writes(&effects).is_empty(), "{effects:?}");
    let effects = submit_draft(&mut fx, CODEX + 10, &codex_draft());
    let dir = state::design_dir(fx.run());
    let paths: Vec<_> = writes(&effects).into_iter().map(|(p, _)| p).collect();
    assert_eq!(
        paths,
        [
            dir.join("brainstorm/draft-claude-v3.md"),
            dir.join("brainstorm/draft-codex-v4.md")
        ]
    );
    let note = "both brainstorm drafts are in; read them with get_doc and submit the merged report";
    assert!(notes(&fx).contains(&note.to_string()));
    let effects = submit(&mut fx, "brainstorm", REPORT);
    assert_eq!(gate(&fx), Some((DocGateKind::Brainstorm, 2, None)));
    let drafts = vec![
        ("claude".to_string(), Ok(round_two)),
        ("codex".to_string(), Ok(codex_draft())),
    ];
    assert_eq!(
        writes(&effects),
        [(dir.join("brainstorm-v2.md"), attach(REPORT, &drafts))]
    );
}

/// DF §3.3 and §3.4, each document by its own author: the orchestrator cannot submit a
/// draft, and a brainstormer cannot submit the report; neither stores anything.
#[test]
fn the_orchestrator_cannot_submit_a_draft_and_a_brainstormer_cannot_submit_the_report() {
    let mut fx = brainstorming();
    let args = json!({"kind": "brainstorm_draft", "text": DRAFT});
    let refusal = refused(&orch_tool(&mut fx, ORCH, "submit_doc", args));
    assert!(
        refusal.ends_with("kind: must be one of brainstorm, spec"),
        "{refusal}"
    );
    let reply = fx.reply();
    let effects = fx.next(EventKind::Orch(OrchEvent::Tool {
        reply,
        call: ToolCall {
            run_id: RUN_ID.into(),
            task_id: None,
            role: AgentRole::Brainstormer,
            window_id: CLAUDE,
            tool: "submit_doc".into(),
            args: json!({"kind": "brainstorm", "text": REPORT}),
            scout_id: None,
            epic: None,
            chain: None,
            lane: None,
        },
        refusals: Vec::new(),
    }));
    let (ok, value) = answered(&effects);
    assert!(!ok);
    let refusal = value["error"].as_str().unwrap();
    assert!(
        refusal.ends_with("kind: must be brainstorm_draft"),
        "{refusal}"
    );
    assert_eq!(
        states(&fx),
        [DesignAgentState::Running, DesignAgentState::Running]
    );
    let design = fx.run().orch.design.as_ref().unwrap();
    assert!(design.versions.is_empty() && design.held.is_empty());
}

/// Ruling T9-1 and T9-1a: after a restart the engine holds no draft's text until the
/// restore's read-back answers. Until then the report, the orchestrator's and the
/// user's edit at the gate alike, is refused as not in; once read back it is taken with
/// the real drafts; a draft the read-back could not read is attached as unread, with
/// why, and its brainstormer's outcome is unchanged (no single-brainstorm line).
#[test]
fn a_report_after_a_restart_waits_for_the_drafts_read_back() {
    use crate::run::engine::DocChecked;
    let at_gate = || {
        let mut fx = brainstorming();
        both_drafts(&mut fx);
        submitted(&mut fx, "brainstorm", REPORT);
        // A restart: the texts are memory only.
        fx.run_mut().orch.design.as_mut().unwrap().texts.clear();
        fx
    };
    let changes = |fx: &mut Fixture| {
        let changes = DocGateAction::Changes {
            note: "More.".into(),
            review: false,
        };
        act(fx, DocGateKind::Brainstorm, changes).unwrap();
    };
    let read = |n: u32, read: Result<&str, &str>| DocChecked {
        kind: DocKind::BrainstormDraft,
        n,
        read: read.map(|t| Some(t.to_string())).map_err(String::from),
    };
    let checked = |fx: &mut Fixture, checked: Vec<DocChecked>| {
        fx.next(EventKind::DesignChecked {
            run_id: RUN_ID.into(),
            checked,
        });
    };

    // Before the read-back: refused, nothing stored, for the orchestrator and the user.
    let mut fx = at_gate();
    let edit = DocGateAction::Edit {
        text: REPORT.into(),
    };
    assert_eq!(
        act(&mut fx, DocGateKind::Brainstorm, edit),
        Err(NOT_IN.into())
    );
    changes(&mut fx);
    assert_eq!(refused(&submit(&mut fx, "brainstorm", REPORT)), NOT_IN);
    assert_eq!(gate(&fx).map(|g| g.1), Some(1));

    // Read back: taken, with the real drafts.
    checked(
        &mut fx,
        vec![read(1, Ok(DRAFT)), read(2, Ok(&codex_draft()))],
    );
    let effects = submit(&mut fx, "brainstorm", REPORT);
    let drafts = vec![
        ("claude".to_string(), Ok(DRAFT.to_string())),
        ("codex".to_string(), Ok(codex_draft())),
    ];
    assert_eq!(writes(&effects)[0].1, attach(REPORT, &drafts));

    // One unreadable: taken, that draft attached as unread with why; both still `Done`.
    let mut fx = at_gate();
    changes(&mut fx);
    let why = "its file differs from what was stored";
    checked(&mut fx, vec![read(1, Ok(DRAFT)), read(2, Err(why))]);
    let effects = submit(&mut fx, "brainstorm", REPORT);
    let drafts = vec![
        ("claude".to_string(), Ok(DRAFT.to_string())),
        (
            "codex".to_string(),
            Err(format!("its draft could not be read back: {why}")),
        ),
    ];
    assert_eq!(writes(&effects)[0].1, attach(REPORT, &drafts));
    assert!((writes(&effects)[0].1).contains(&format!(
        "(no draft: its draft could not be read back: {why})"
    )));
    assert_eq!(
        states(&fx),
        [DesignAgentState::Done, DesignAgentState::Done]
    );
}

/// Review minor 7: the merged report's template, pinned whole.
#[test]
fn the_reports_template_is_exact() {
    let mut fx = brainstorming();
    both_drafts(&mut fx);
    let expected = "the merged report's template: ## Where they agree; ## Where they \
                    disagree (each side, then your judgment); ## Approaches, each approach \
                    a \"### <name> [claude]\" heading tagged [claude], [codex] or [both]; ## \
                    Recommendation, naming one listed approach; ## Questions for you";
    assert_eq!(notes(&fx)[1], expected);
    let mut fx = brainstorming();
    assert!(answered(&submit_draft(&mut fx, CLAUDE, DRAFT)).0);
    ended(&mut fx, "codex", 1, ScoutEnd::Failed { reason: "x".into() });
    let single = format!("{expected}; begin with the line \"single brainstorm: codex failed: x\"");
    assert_eq!(notes(&fx)[1], single);
}

/// The brainstorm gate's latest version's text, as the engine keeps it.
pub(super) fn version_text(fx: &Fixture) -> String {
    let design = fx.run().orch.design.as_ref().unwrap();
    design.text_of(DocKind::Brainstorm).unwrap().1.to_string()
}

/// Review minor 5: an appendix heading of the report's own cuts it there, and the run's
/// log says so; the engine's own appendix sent back is cut silently.
#[test]
fn a_reports_own_appendix_heading_is_warned_in_the_log() {
    use crate::run::engine::design::report::CUT_WARNING;
    let mut fx = brainstorming();
    both_drafts(&mut fx);
    let own = format!("{REPORT}\n{APPENDIX}\nmy own notes\n");
    submitted(&mut fx, "brainstorm", &own);
    assert_eq!(
        log_lines(&fx).iter().filter(|l| *l == CUT_WARNING).count(),
        1
    );
    // What came after the report's own heading is not stored.
    assert!(!version_text(&fx).contains("my own notes"));
    let changes = DocGateAction::Changes {
        note: "Again.".into(),
        review: false,
    };
    act(&mut fx, DocGateKind::Brainstorm, changes).unwrap();
    // The stored file, as `get_doc` returns it, resubmitted: no warning.
    let drafts = vec![
        ("claude".to_string(), Ok(DRAFT.to_string())),
        ("codex".to_string(), Ok(codex_draft())),
    ];
    let file = attach(REPORT, &drafts);
    assert_eq!(version_text(&fx), file);
    submitted(&mut fx, "brainstorm", &file);
    assert_eq!(
        log_lines(&fx).iter().filter(|l| *l == CUT_WARNING).count(),
        1
    );
}
