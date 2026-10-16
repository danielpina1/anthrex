//! Task M9.15.11 (decisions 27 to 30, rulings C11 and C12): the pre-review risk note.
//! A reviewed task's first gate queues one `risk_note` decider, written from the brief
//! alone; a queued note never delays review; a note that lands is kept for every later
//! round and reaches the reviewer's prompt under its heading; a failed one changes
//! nothing.

use proto::{DeciderSource, LessonLayer, LessonStatus, TaskKind, TaskState};
use serde_json::json;

use super::fixture::*;
use super::gates::{CHECK_MODE, accepted, check_result, only_op, tdd_args};
use super::gates_support::{blocking, reviewer, submit, verdict};
use super::merge::{claim, head_of, pending_one, window_of};
use crate::decider::{DeciderAnswer, DeciderKind, DeciderRequest, Decision};
use crate::run::contract_lessons::{REVIEWER_HEADING, RISK_NOTE_HEADING};
use crate::run::engine::risk_note::queue_once;
use crate::run::engine::schedule::readers_busy;
use crate::run::engine::{Effect, OpKind, OpResult};
use crate::run::model::{LessonBrief, LessonLimits, OpId, ReviewLevel, Run, RunLessons};

/// The run's frozen lessons: none to show, risk notes as `risk_note` says.
fn lessons(risk_note: bool) -> RunLessons {
    RunLessons {
        limits: LessonLimits {
            risk_note,
            ..LessonLimits::default()
        },
        items: Vec::new(),
        frozen_at: 2_000,
    }
}

/// A running run of `tasks` on `profile`, deciders on, frozen with `lessons` after
/// `edit`; every worker launched.
fn running(
    profile: &str,
    tasks: &[String],
    lessons: Option<RunLessons>,
    edit: impl FnOnce(&mut Run),
) -> (Fixture, Vec<(String, u32)>) {
    let mut fx = Fixture::deciding(&plan_with(profile, tasks), config::Orchestrator::default());
    fx.start_with(true, |run| {
        run.lessons = lessons;
        edit(run);
    });
    let (op, _) = fx.op("CreateRunBranch");
    fx.done(op, OpResult::Worktree { head: BASE.into() });
    let windows = fx.launch_all();
    (fx, windows)
}

/// One check-mode M `t1` owning `crates/a/**` (reviewed), its window.
fn one_reviewed(profile: &str, lessons: Option<RunLessons>) -> (Fixture, u32) {
    let tasks = [task("t1", "M", "a", CHECK_MODE)];
    let (fx, windows) = running(profile, &tasks, lessons, |_| {});
    assert!(fx.task("t1").review_level.is_some());
    (fx, windows[0].1)
}

/// The `RiskNote` requests among `effects`' `Decide` ops: (op, task ids).
fn notes_in(effects: &[Effect]) -> Vec<(OpId, Vec<String>)> {
    ops_in(effects, "Decide")
        .into_iter()
        .filter_map(|(op, kind)| match kind {
            OpKind::Decide {
                task_ids,
                request: DeciderRequest::RiskNote(_),
                ..
            } => Some((op, task_ids)),
            _ => None,
        })
        .collect()
}

fn queued_notes(fx: &Fixture) -> Vec<Vec<String>> {
    (fx.queued_deciders().into_iter())
        .filter(|q| matches!(q.request, DeciderRequest::RiskNote(_)))
        .map(|q| q.task_ids)
        .collect()
}

/// A decider's answer: `note`.
fn note(note: &str) -> Decision {
    Decision {
        kind: DeciderKind::RiskNote,
        answer: DeciderAnswer::RiskNote { note: note.into() },
        source: DeciderSource::Decider,
        fallback_reason: None,
        usage: None,
        secs: 2,
    }
}

/// `t1` claims done; its check starts and its note is asked. The check and the note.
fn to_check(fx: &mut Fixture, window: u32) -> (OpId, OpId) {
    let effects = accepted(fx, window, json!({"summary": "s"}));
    let (check, _) = only_op(&effects, "Check");
    let notes = notes_in(&effects);
    assert_eq!(notes.len(), 1, "{effects:#?}");
    (check, notes[0].0)
}

/// `t1`'s check passes and its reviewer starts: the reviewer's first turn.
fn review_turn(fx: &mut Fixture, check: OpId) -> String {
    let effects = fx.done(check, check_result(true));
    let (prepare, _) = only_op(&effects, "PrepareReview");
    let (_, kind) = reviewer(fx, prepare, "diff --git a/x b/x");
    let OpKind::CreateWindow { first_turn, .. } = kind else {
        unreachable!()
    };
    first_turn
}

#[test]
fn risk_note_queued_once_at_first_gate() {
    // A TDD M task: its proof is its first gate, then its check.
    let tasks = [task("t1", "M", "a", "")];
    let (mut fx, windows) = running(PROFILE, &tasks, Some(lessons(true)), |_| {});
    assert_eq!(fx.task("t1").review_level, Some(ReviewLevel::Medium));
    let effects = accepted(&mut fx, windows[0].1, tdd_args());
    assert_eq!(fx.task("t1").state, TaskState::Proof);
    let notes = notes_in(&effects);
    assert_eq!(notes.len(), 1, "{effects:#?}");
    assert_eq!(notes[0].1, ["t1"]);
    assert!(fx.task("t1").risk_note_asked);
    // Written from the brief alone: the task's plan text, no diff.
    let (_, kind) = pending_one(&fx, "Decide", Some("t1"));
    let OpKind::Decide {
        request: DeciderRequest::RiskNote(input),
        ..
    } = kind
    else {
        unreachable!()
    };
    let spec = &fx.task("t1").spec;
    assert_eq!(
        (&input.task_id, &input.title, &input.brief),
        (&spec.id, &spec.title, &spec.brief)
    );
    assert_eq!(
        (&input.acceptance, &input.owns),
        (&spec.acceptance, &spec.owns)
    );

    // The proof passes: the check asks nothing more, nor does a later pass.
    let (proof, _) = only_op(&effects, "Proof");
    let proved = OpResult::Proof {
        red_failed: true,
        head_passed: true,
        matched: true,
        red_tail: "red".into(),
        head_tail: "head".into(),
    };
    let effects = fx.done(proof, proved);
    assert_eq!(fx.task("t1").state, TaskState::Check);
    assert!(notes_in(&effects).is_empty(), "{effects:#?}");
    fx.tick();
    assert_eq!(notes_in(&fx.log).len(), 1);
}

#[test]
fn small_review_level_skips() {
    // A TDD S task is reviewed at `small`: its proof asks nothing.
    let tasks = [task("t1", "S", "a", "")];
    let (mut fx, windows) = running(PROFILE, &tasks, Some(lessons(true)), |_| {});
    assert_eq!(fx.task("t1").review_level, Some(ReviewLevel::Small));
    let effects = accepted(&mut fx, windows[0].1, tdd_args());
    assert_eq!(fx.task("t1").state, TaskState::Proof);
    assert!(notes_in(&effects).is_empty(), "{effects:#?}");
    assert!(!fx.task("t1").risk_note_asked);

    // Each rule alone, against a positive control.
    let plan = plan_with(PROFILE, &[task("t1", "M", "a", "")]);
    let base = || {
        let mut run = build(&plan, &config::Orchestrator::default(), true);
        run.lessons = Some(lessons(true));
        run
    };
    let asked = |edit: &dyn Fn(&mut Run)| {
        let mut run = base();
        edit(&mut run);
        let before = run.tasks[0].risk_note_asked;
        queue_once(&mut run, 0, 5);
        let queued = run.decider_queue.len();
        assert!(queued <= 1);
        assert_eq!(run.tasks[0].risk_note_asked, before || queued == 1);
        queued == 1
    };
    assert!(asked(&|_| {}));
    assert!(asked(
        &|run| run.tasks[0].review_level = Some(ReviewLevel::Frontier)
    ));
    assert!(!asked(
        &|run| run.tasks[0].review_level = Some(ReviewLevel::Small)
    ));
    assert!(!asked(&|run| run.tasks[0].review_level = None));
    assert!(!asked(&|run| run.tasks[0].spec.kind = TaskKind::Research));
    assert!(!asked(&|run| run.tasks[0].spec.kind = TaskKind::Review));
    assert!(!asked(&|run| run.lessons = Some(lessons(false))));
    // Ruling C2: a run without frozen lessons (a 9.14 run) asks none.
    assert!(!asked(&|run| run.lessons = None));
    // Once asked, never again.
    assert!(!asked(&|run| run.tasks[0].risk_note_asked = true));
    let mut run = base();
    queue_once(&mut run, 0, 5);
    queue_once(&mut run, 0, 6);
    assert_eq!(run.decider_queue.len(), 1);
}

#[test]
fn review_never_waits_for_risk_note() {
    // One reader slot; t2 asks no note, so only t1's is in play.
    let tasks = [
        task("t1", "M", "a", CHECK_MODE),
        task("t2", "M", "b", CHECK_MODE),
    ];
    let profile = profile_with("max_readers = 1");
    let (mut fx, windows) = running(&profile, &tasks, Some(lessons(true)), |run| {
        run.tasks[1].risk_note_asked = true;
    });
    // t2's review holds the slot.
    claim(&mut fx, "t2", window_of(&windows, "t2"), &head_of("t2"));
    let (check, _) = pending_one(&fx, "Check", Some("t2"));
    fx.done(check, check_result(true));
    let (review, _) = pending_one(&fx, "PrepareReview", Some("t2"));
    assert_eq!(readers_busy(fx.run()), 1);
    // t1's check asks its note, which waits for the slot.
    let effects = claim(&mut fx, "t1", window_of(&windows, "t1"), &head_of("t1"));
    assert!(notes_in(&effects).is_empty(), "{effects:#?}");
    assert_eq!(queued_notes(&fx), [vec!["t1".to_string()]]);
    // t1 reaches review and needs the slot too.
    let (check, _) = pending_one(&fx, "Check", Some("t1"));
    fx.done(check, check_result(true));
    assert_eq!(fx.task("t1").state, TaskState::Review);
    assert_eq!(queued_notes(&fx).len(), 1);

    // The slot frees: t1's reviewer takes it, not the note, which is then dropped.
    let failed = OpResult::Failed {
        message: "no space".into(),
    };
    let effects = fx.done(review, failed);
    assert!(ops_in(&effects, "Decide").is_empty(), "{effects:#?}");
    let prepares = ops_in(&effects, "PrepareReview");
    assert_eq!(prepares.len(), 1, "{effects:#?}");
    assert_eq!(op_task(&prepares[0].1), "t1");
    assert!(queued_notes(&fx).is_empty());
    assert_eq!(readers_busy(fx.run()), 1);
    // A dropped note is no call and no fallback; it is never asked again.
    assert_eq!((fx.run().decider_calls, fx.run().decider_fallbacks), (0, 0));
    fx.tick();
    assert!(notes_in(&fx.log).is_empty());
    assert_eq!(fx.task("t1").risk_note, None);
}

/// Ruling C12: a note already in flight holds its reader slot until its call ends.
#[test]
fn an_in_flight_note_holds_its_slot_until_its_call_ends() {
    let (mut fx, window) = one_reviewed(&profile_with("max_readers = 1"), Some(lessons(true)));
    let (check, note_op) = to_check(&mut fx, window);
    let effects = fx.done(check, check_result(true));
    assert_eq!(fx.task("t1").state, TaskState::Review);
    assert!(ops_in(&effects, "PrepareReview").is_empty(), "{effects:#?}");
    assert_eq!(readers_busy(fx.run()), 1);
    assert!(ops_in(&fx.tick(), "PrepareReview").is_empty());

    let effects = fx.decided(note_op, note("- check the retry counter"));
    assert_eq!(ops_in(&effects, "PrepareReview").len(), 1, "{effects:#?}");
    let t1 = fx.task("t1");
    assert_eq!(t1.risk_note.as_deref(), Some("- check the retry counter"));
}

#[test]
fn a_late_note_is_kept_for_the_next_round() {
    let (mut fx, window) = one_reviewed(PROFILE, Some(lessons(true)));
    let (check, note_op) = to_check(&mut fx, window);
    // Round 1's reviewer starts before the note lands: no note.
    let first = review_turn(&mut fx, check);
    assert!(!first.contains(RISK_NOTE_HEADING), "{first}");
    // It lands during the round and is kept.
    fx.decided(note_op, note("- the counter resets per upload"));
    assert_eq!(
        fx.task("t1").risk_note.as_deref(),
        Some("- the counter resets per upload")
    );
    let detail = crate::run::snapshot_detail::task_detail(fx.run(), "t1").unwrap();
    assert_eq!(
        detail.risk_note.as_deref(),
        Some("- the counter resets per upload")
    );

    // Round 2 carries it, and asks no second note.
    let rwindow = fx.task("t1").rounds.last().unwrap().window_id.unwrap();
    submit(&mut fx, rwindow, verdict("changes", blocking()));
    super::turns::exited(&mut fx, rwindow);
    fx.turn_completed(window);
    let effects = accepted(&mut fx, window, json!({"summary": "s"}));
    assert!(notes_in(&effects).is_empty(), "{effects:#?}");
    let (check, _) = only_op(&effects, "Check");
    let second = review_turn(&mut fx, check);
    let block = format!("{RISK_NOTE_HEADING}\n- the counter resets per upload");
    assert!(second.contains(&block), "{second}");
    assert_eq!(notes_in(&fx.log).len(), 1);
}

#[test]
fn failed_note_leaves_prompt_unchanged() {
    // With risk notes off, the reviewer's first turn as it always was.
    let (mut fx, window) = one_reviewed(PROFILE, Some(lessons(false)));
    let effects = accepted(&mut fx, window, json!({"summary": "s"}));
    assert!(notes_in(&effects).is_empty(), "{effects:#?}");
    let (check, _) = only_op(&effects, "Check");
    let plain = review_turn(&mut fx, check);
    // Ruling C2: and as a run without frozen lessons sees it.
    let (mut fx, window) = one_reviewed(PROFILE, None);
    let effects = accepted(&mut fx, window, json!({"summary": "s"}));
    let (check, _) = only_op(&effects, "Check");
    assert_eq!(review_turn(&mut fx, check), plain);

    let failures: [(&str, OpResult); 4] = [
        (
            "the call failed",
            OpResult::Failed {
                message: "no binary".into(),
            },
        ),
        (
            "the fallback",
            OpResult::Decided(Box::new(crate::decider::fallback::fallback_decision(
                &DeciderRequest::RiskNote(crate::decider::RiskNoteInput {
                    task_id: "t1".into(),
                    title: String::new(),
                    brief: String::new(),
                    acceptance: Vec::new(),
                    owns: Vec::new(),
                }),
                "timed out".into(),
            ))),
        ),
        ("an empty note", OpResult::Decided(Box::new(note(" \n ")))),
        (
            "an answer of another kind",
            OpResult::Decided(Box::new(Decision {
                kind: DeciderKind::BlockedReason,
                answer: DeciderAnswer::BlockedReason {
                    kind: crate::decider::BlockKind::Question,
                    reason: "- not a note".into(),
                },
                ..note("x")
            })),
        ),
    ];
    for (case, result) in failures {
        let (mut fx, window) = one_reviewed(PROFILE, Some(lessons(true)));
        let (check, note_op) = to_check(&mut fx, window);
        fx.done(note_op, result);
        assert_eq!(fx.task("t1").risk_note, None, "{case}");
        assert_eq!(review_turn(&mut fx, check), plain, "{case}");
        let detail = crate::run::snapshot_detail::task_detail(fx.run(), "t1").unwrap();
        assert_eq!(detail.risk_note, None, "{case}");
    }
}

/// Ruling C11: a decider-sourced answer goes through `on_decided`'s `RiskNote` arms.
#[test]
fn note_reaches_the_reviewer_prompt_under_its_heading() {
    // One lesson on t1's files, so the known pitfalls come first.
    let mut frozen = lessons(true);
    frozen.items.push(LessonBrief {
        id: "l-1".into(),
        layer: LessonLayer::Episodic,
        status: LessonStatus::OnTrial,
        title: "the fixture moved".into(),
        body: "read it from tests/fixtures".into(),
        files: vec!["crates/a/**".into()],
        last_seen: 2_000,
        usefulness: None,
    });
    let (mut fx, window) = one_reviewed(PROFILE, Some(frozen));
    let (check, note_op) = to_check(&mut fx, window);
    // A hostile answer is cleaned before it is kept.
    let ghp = format!("ghp_{}", "FAKE".repeat(9));
    let answer = format!("- check the retry counter\n- \u{1b}[31mkeep {ghp} out\u{1b}[0m");
    fx.decided(note_op, note(&answer));
    let kept = fx.task("t1").risk_note.clone().expect("the note is kept");
    assert!(
        kept.starts_with("- check the retry counter\n- keep "),
        "{kept:?}"
    );
    assert!(
        !kept.contains(&ghp[4..]) && !kept.contains('\u{1b}'),
        "{kept:?}"
    );
    assert_eq!((fx.run().decider_calls, fx.run().decider_fallbacks), (1, 0));

    let turn = review_turn(&mut fx, check);
    let block = format!("{RISK_NOTE_HEADING}\n{kept}");
    let at = |text: &str| {
        turn.find(text)
            .unwrap_or_else(|| panic!("{text:?} in {turn}"))
    };
    let brief = &fx.task("t1").spec.brief;
    assert!(at(REVIEWER_HEADING) < at(&block), "{turn}");
    assert!(at(&block) < turn.rfind(brief.as_str()).unwrap(), "{turn}");
    assert!(at("Diff (") < at(&block), "{turn}");
}
