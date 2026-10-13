//! Task M9.15.4: decisions 7, 8, 17 and 25 over constructed history and store lines.
//! Every task, run, flaky and revert line is built by a helper that fills every field
//! consistently with its outcome, so no input here is one the daemon could not write.

use config::Lessons;
use proto::{
    BlockReason, DiffStats, DismissedLine, DistillCause, DistilledLine, DoneSignal, Effort,
    FlakyRecord, GateCounts, GateTally, HISTORY_VERSION, HistoryLine, LESSONS_VERSION, LessonDraft,
    LessonLayer, LessonLine, LessonOrigin, LessonStatus, PhaseSecs, QueuedLine, RetiredLine,
    RevertRecord, Route, RunRecord, Runtime, SeverityTally, SigGate, SigRef, Size, TaskKind,
    TaskOutcome, TaskRecord, TestMode, TokenUsage, VerdictLine,
};
use sha2::{Digest, Sha256};

use super::super::fold::{Store, fold, title_fp};
use super::known;

const T0: u64 = 1_800_000_000; // 2027-01-15 08:00:00Z
const DAY: u64 = 86_400;
const FP: &str = "0123456789abcdef";
const FP2: &str = "fedcba9876543210";
const HEADLINE: &str = "assertion `left == right` failed: parser rejects <id>";

fn cfg() -> Lessons {
    Lessons::default()
}

fn route() -> Route {
    Route {
        runtime: Runtime::Claude,
        model: "m".into(),
        effort: Effort::MEDIUM,
    }
}

fn sha(seed: &str) -> String {
    let digest = Sha256::digest(seed.as_bytes());
    digest
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>()[..40]
        .to_string()
}

/// A run id as `plan::slug` makes one.
fn rid(n: u32) -> String {
    format!("run{n}-{n:04x}")
}

/// A finished task whose every field agrees with `outcome`: a merged one has a merge
/// commit, a diff, a review round and a done signal; a blocked one a block reason; a
/// reported one is research and ran no check.
fn task(run: &str, id: &str, at: u64, outcome: TaskOutcome) -> TaskRecord {
    let merged = matches!(
        outcome,
        TaskOutcome::Merged | TaskOutcome::MergedWithoutApproval
    );
    let reported = outcome == TaskOutcome::Reported;
    TaskRecord {
        v: HISTORY_VERSION,
        record_id: format!("{run}/{id}"),
        at,
        run_id: run.into(),
        task_id: id.into(),
        path: None,
        kind: if reported {
            TaskKind::Research
        } else {
            TaskKind::Code
        },
        hub: false,
        test_mode: if reported {
            TestMode::None
        } else {
            TestMode::Check
        },
        planned_size: Size::S,
        final_size: Size::S,
        size_check: None,
        route: route(),
        review_routes: if merged { vec![route()] } else { Vec::new() },
        routing_decisions: Vec::new(),
        outcome,
        block: (outcome == TaskOutcome::Blocked).then_some(BlockReason::Human),
        diff: merged.then_some(DiffStats {
            files: 1,
            hunks: 1,
            added: 3,
            removed: 1,
        }),
        tool_calls: 4,
        worker_usage: TokenUsage::default(),
        reviewer_usage: TokenUsage::default(),
        decider_usage: TokenUsage::default(),
        phases: PhaseSecs::default(),
        wall_secs: 60,
        gates: GateTally {
            checks: u32::from(!reported),
            review_rounds: u32::from(merged),
            ..GateTally::default()
        },
        severities: SeverityTally::default(),
        bounces: GateCounts::default(),
        failures: 0,
        stalls: 0,
        budget_exceeded: 0,
        conflicts: 0,
        max_rung: 0,
        sessions: 1,
        done_signal: (merged || reported).then_some(DoneSignal::TaskDone),
        merge_commit: merged.then(|| sha(&format!("{run}/{id}"))),
        stage: 1,
        origin: proto::TaskOrigin::Plan,
        pattern: None,
        race_winner: None,
        race_adopted: false,
        writer_failures: 0,
        round: 1,
        signatures: Vec::new(),
        owns: vec!["crates/a/**".into()],
        lessons_shown: Vec::new(),
    }
}

/// `t` with one more failed check, signed `fp`.
fn failed(mut t: TaskRecord, fp: &str, headline: &str) -> TaskRecord {
    t.gates.checks += 1;
    t.gates.checks_failed += 1;
    t.signatures.push(SigRef {
        gate: SigGate::Check,
        fp: fp.into(),
        headline: headline.into(),
    });
    t
}

fn owning(mut t: TaskRecord, owns: &[&str]) -> TaskRecord {
    t.owns = owns.iter().map(|s| s.to_string()).collect();
    t
}

fn shown(mut t: TaskRecord, ids: &[&str]) -> TaskRecord {
    t.lessons_shown = ids.iter().map(|s| s.to_string()).collect();
    t
}

fn line(t: TaskRecord) -> HistoryLine {
    HistoryLine::Task(t)
}

fn run(run: &str, at: u64, outcome: &str) -> HistoryLine {
    HistoryLine::Run(RunRecord {
        discard_reason: None,
        v: HISTORY_VERSION,
        record_id: run.into(),
        at,
        run_id: run.into(),
        goal: "g".into(),
        path: None,
        triage: None,
        profile_source: None,
        outcome: outcome.into(),
        base_branch: "main".into(),
        accepted_commit: (outcome == "accepted").then(|| sha(run)),
        tasks: 1,
        usage: None,
    })
}

fn flaky(run: &str, at: u64, test: &str) -> HistoryLine {
    HistoryLine::Flaky(FlakyRecord {
        v: HISTORY_VERSION,
        record_id: format!("{run}/flaky/1/{test}"),
        at,
        run_id: run.into(),
        task_id: Some("t1".into()),
        tier: 1,
        test: test.into(),
    })
}

fn revert(t: &TaskRecord, at: u64) -> HistoryLine {
    let reverted = t.merge_commit.clone().expect("a merged task");
    HistoryLine::Revert(RevertRecord {
        v: HISTORY_VERSION,
        record_id: format!("revert/{}", sha(&reverted)),
        at,
        run_id: t.run_id.clone(),
        task_id: Some(t.task_id.clone()),
        revert_commit: sha(&reverted),
        reverted,
    })
}

// ---- store lines ----

fn queued(run: &str, at: u64) -> LessonLine {
    LessonLine::Queued(QueuedLine {
        v: LESSONS_VERSION,
        record_id: format!("{run}/t1/distill"),
        at,
        run_id: run.into(),
        task_id: Some("t1".into()),
        cause: DistillCause::TaskRecovered,
        dossier: "## outcome".into(),
        fps: Vec::new(),
    })
}

fn distilled(run: &str, at: u64, items: &[(LessonLayer, &str)]) -> LessonLine {
    let queued = format!("{run}/t1/distill");
    LessonLine::Distilled(DistilledLine {
        v: LESSONS_VERSION,
        record_id: format!("{queued}/distilled"),
        at,
        queued,
        items: (items.iter())
            .map(|&(layer, title)| LessonDraft {
                layer,
                title: title.into(),
                body: format!("body of {title}"),
                files: vec!["crates/a/**".into()],
            })
            .collect(),
        dropped: 0,
        usage: None,
    })
}

/// One distill entry of `run` at `at` with `items`.
fn entry(run: &str, at: u64, items: &[(LessonLayer, &str)]) -> [LessonLine; 2] {
    [queued(run, at), distilled(run, at + 1, items)]
}

fn verdict(id: &str, at: u64, approved: bool) -> LessonLine {
    LessonLine::Verdict(VerdictLine {
        v: LESSONS_VERSION,
        record_id: format!("{id}/verdict/{at}"),
        at,
        id: id.into(),
        approved,
    })
}

fn retired(id: &str, at: u64, reason: &str) -> LessonLine {
    LessonLine::Retired(RetiredLine {
        v: LESSONS_VERSION,
        record_id: format!("{id}/retired"),
        at,
        id: id.into(),
        reason: reason.into(),
    })
}

fn dismissed(fp: &str, at: u64) -> LessonLine {
    LessonLine::Dismissed(DismissedLine {
        v: LESSONS_VERSION,
        record_id: format!("kf-{fp}/dismissed"),
        at,
        fp: fp.into(),
    })
}

fn id_of(title: &str) -> String {
    format!("l-{}", title_fp(title))
}

fn kf(fp: &str) -> String {
    format!("kf-{fp}")
}

fn ids(lessons: &[super::Lesson]) -> Vec<String> {
    lessons.iter().map(|l| l.id.clone()).collect()
}

/// Task `t1` of run `n` at `at`, blocked on `fp`.
fn hit(n: u32, at: u64, fp: &str) -> HistoryLine {
    line(failed(
        task(&rid(n), "t1", at, TaskOutcome::Blocked),
        fp,
        HEADLINE,
    ))
}

/// A clean merged task of run `n` at `at` owning `owns`.
fn clean(n: u32, at: u64, owns: &str) -> HistoryLine {
    line(owning(
        task(&rid(n), "t1", at, TaskOutcome::Merged),
        &[owns],
    ))
}

// ---- known failures (decisions 7 and 8) ----

#[test]
fn known_needs_two_runs_in_window() {
    let store = Store::default();
    // One run, two of its tasks: one run is not enough.
    let one_run = [
        hit(1, T0, FP),
        line(failed(
            task(&rid(1), "t2", T0 + 5, TaskOutcome::Blocked),
            FP,
            HEADLINE,
        )),
    ];
    assert!(known(&one_run, &store, T0 + 10, &cfg()).is_empty());

    // Two runs 13 days apart.
    let within = [hit(1, T0, FP), hit(2, T0 + 13 * DAY, FP)];
    let got = known(&within, &store, T0 + 13 * DAY, &cfg());
    assert_eq!(ids(&got), vec![kf(FP)]);
    let k = &got[0];
    assert_eq!(k.layer, LessonLayer::Episodic);
    assert_eq!(k.origin, LessonOrigin::Signature);
    assert_eq!(k.status, LessonStatus::Known);
    assert_eq!(k.title, format!("Known failure: {HEADLINE}"));
    assert_eq!(
        k.body,
        format!("Seen 2 times in 2 runs (last 2027-01-28, gate check): {HEADLINE}")
    );
    assert_eq!((k.created_at, k.last_seen), (T0, T0 + 13 * DAY));
    assert_eq!(k.runs, vec![rid(1), rid(2)]);
    assert_eq!(k.headlines, vec![HEADLINE.to_string()]);

    // Two runs 15 days apart: never two in one 14-day window.
    let apart = [hit(1, T0, FP), hit(2, T0 + 15 * DAY, FP)];
    assert!(known(&apart, &store, T0 + 15 * DAY, &cfg()).is_empty());

    // The title is clipped to 120 characters.
    let long = "x".repeat(160);
    let long_hits = [
        line(failed(
            task(&rid(1), "t1", T0, TaskOutcome::Blocked),
            FP,
            &long,
        )),
        line(failed(
            task(&rid(2), "t1", T0 + DAY, TaskOutcome::Blocked),
            FP,
            &long,
        )),
    ];
    let got = known(&long_hits, &store, T0 + DAY, &cfg());
    assert_eq!(got[0].title.chars().count(), 120);
    assert!(got[0].title.starts_with("Known failure: xxx"));
}

#[test]
fn known_files_are_the_union_of_owns() {
    let history = [
        line(owning(
            failed(task(&rid(1), "t1", T0, TaskOutcome::Blocked), FP, HEADLINE),
            &["crates/a/**", "crates/b/**"],
        )),
        line(owning(
            failed(
                task(&rid(2), "t1", T0 + DAY, TaskOutcome::Blocked),
                FP,
                HEADLINE,
            ),
            &["crates/b/**", "crates/c/**"],
        )),
        line(owning(
            failed(
                task(&rid(3), "t1", T0 + 2 * DAY, TaskOutcome::Blocked),
                FP,
                HEADLINE,
            ),
            &["d", "e", "f", "g", "h", "i"],
        )),
    ];
    let got = known(&history, &Store::default(), T0 + 2 * DAY, &cfg());
    assert_eq!(
        got[0].files,
        [
            "crates/a/**",
            "crates/b/**",
            "crates/c/**",
            "d",
            "e",
            "f",
            "g",
            "h"
        ]
    );
    assert_eq!(
        got[0].body,
        format!("Seen 3 times in 3 runs (last 2027-01-17, gate check): {HEADLINE}")
    );
}

#[test]
fn five_clean_runs_supersede() {
    let store = Store::default();
    let now = T0 + 20 * DAY;
    let mut history = vec![hit(1, T0, FP), hit(2, T0 + DAY, FP)];
    for n in 0..4 {
        history.push(clean(
            10 + n,
            T0 + (2 + u64::from(n)) * DAY,
            "crates/a/src/x.rs",
        ));
    }
    assert_eq!(ids(&known(&history, &store, now, &cfg())), vec![kf(FP)]);
    let mut fifth = history.clone();
    fifth.push(clean(14, T0 + 6 * DAY, "crates/a/src/x.rs"));
    assert!(known(&fifth, &store, now, &cfg()).is_empty());

    // A clean run over a disjoint glob counts nothing.
    let mut disjoint = vec![hit(1, T0, FP), hit(2, T0 + DAY, FP)];
    for n in 0..5 {
        disjoint.push(clean(10 + n, T0 + (2 + u64::from(n)) * DAY, "crates/b/**"));
    }
    assert_eq!(ids(&known(&disjoint, &store, now, &cfg())), vec![kf(FP)]);

    // A merged task with no check run counts nothing either.
    let mut unchecked = vec![hit(1, T0, FP), hit(2, T0 + DAY, FP)];
    for n in 0..5 {
        let mut t = task(
            &rid(10 + n),
            "t1",
            T0 + (2 + u64::from(n)) * DAY,
            TaskOutcome::Merged,
        );
        t.gates.checks = 0;
        unchecked.push(line(t));
    }
    assert_eq!(ids(&known(&unchecked, &store, now, &cfg())), vec![kf(FP)]);

    // The fp again after four clean runs resets nothing but refreshes `last_seen`.
    let mut again = history.clone();
    again.push(hit(3, T0 + 6 * DAY + DAY / 2, FP));
    let got = known(&again, &store, now, &cfg());
    assert_eq!(ids(&got), vec![kf(FP)]);
    assert_eq!(got[0].last_seen, T0 + 6 * DAY + DAY / 2);
    again.push(clean(14, T0 + 7 * DAY, "crates/a/src/x.rs"));
    assert!(known(&again, &store, now, &cfg()).is_empty());
}

#[test]
fn known_expires_after_days() {
    let store = Store::default();
    let history = [hit(1, T0, FP), hit(2, T0 + DAY, FP)];
    let last = T0 + DAY;
    assert!(known(&history, &store, last + 91 * DAY, &cfg()).is_empty());
    assert_eq!(
        ids(&known(&history, &store, last + 89 * DAY, &cfg())),
        vec![kf(FP)]
    );
}

/// The controller's ruling on task 4's review, I1: each life of a known failure forms on
/// its own. Once five clean runs end one, only hits after that end can form the next,
/// and they still need `known_min` runs within `known_window_days`.
#[test]
fn a_regression_after_the_clean_run_end_forms_a_new_known_failure() {
    let store = Store::default();
    let mut history = vec![hit(1, T0, FP), hit(2, T0 + DAY, FP)];
    for n in 0..5 {
        history.push(clean(10 + n, T0 + (2 + u64::from(n)) * DAY, "crates/a/**"));
    }
    let now = T0 + 40 * DAY;
    assert!(known(&history, &store, now, &cfg()).is_empty(), "ended");
    // One run of the regression is not a known failure yet.
    history.push(hit(3, T0 + 30 * DAY, FP));
    assert!(known(&history, &store, now, &cfg()).is_empty(), "one run");
    // A second run within the window forms the new life, counted from its own hits.
    history.push(hit(4, T0 + 31 * DAY, FP));
    let got = known(&history, &store, now, &cfg());
    assert_eq!(ids(&got), vec![kf(FP)]);
    assert_eq!(got[0].created_at, T0 + 30 * DAY);
    assert_eq!(got[0].runs, vec![rid(3), rid(4)]);
    assert!(
        got[0].body.starts_with("Seen 2 times in 2 runs"),
        "{}",
        got[0].body
    );
}

/// I1's ruling: after the expiry ended a life, one new hit does not bring it back.
#[test]
fn a_single_hit_after_expiry_does_not_form_one() {
    let store = Store::default();
    let late = T0 + DAY + 100 * DAY;
    let history = [hit(1, T0, FP), hit(2, T0 + DAY, FP), hit(3, late, FP)];
    assert!(known(&history, &store, late + DAY, &cfg()).is_empty());
}

/// I1's ruling: after the expiry, two runs within the window form a new life.
#[test]
fn two_hits_within_the_window_after_expiry_form_one() {
    let store = Store::default();
    let late = T0 + DAY + 100 * DAY;
    let history = [
        hit(1, T0, FP),
        hit(2, T0 + DAY, FP),
        hit(3, late, FP),
        hit(4, late + 13 * DAY, FP),
    ];
    let got = known(&history, &store, late + 14 * DAY, &cfg());
    assert_eq!(ids(&got), vec![kf(FP)]);
    assert_eq!(
        (got[0].created_at, got[0].last_seen),
        (late, late + 13 * DAY)
    );
    assert_eq!(got[0].runs, vec![rid(3), rid(4)]);
    // Fifteen days apart, they do not.
    let apart = [
        hit(1, T0, FP),
        hit(2, T0 + DAY, FP),
        hit(3, late, FP),
        hit(4, late + 15 * DAY, FP),
    ];
    assert!(known(&apart, &store, late + 16 * DAY, &cfg()).is_empty());
}

#[test]
fn dismissed_fp_absent() {
    let store = fold(&[dismissed(FP, T0 + 2 * DAY)]);
    let history = [
        hit(1, T0, FP),
        hit(2, T0 + DAY, FP),
        // The same test failing differently: a new signature, a new known failure.
        line(failed(
            task(&rid(3), "t1", T0 + 3 * DAY, TaskOutcome::Blocked),
            FP2,
            "assertion `left == right` failed: parser accepts <id>",
        )),
        line(failed(
            task(&rid(4), "t1", T0 + 4 * DAY, TaskOutcome::Blocked),
            FP2,
            "assertion `left == right` failed: parser accepts <id>",
        )),
        // Later hits of the dismissed fp do not bring it back.
        hit(5, T0 + 5 * DAY, FP),
        hit(6, T0 + 5 * DAY, FP),
    ];
    let got = known(&history, &store, T0 + 5 * DAY, &cfg());
    assert_eq!(ids(&got), vec![kf(FP2)]);
}

#[test]
fn flaky_test_is_not_a_known_failure() {
    let headline = "thread 'b::flaky' (<tid>) panicked at src/b.rs:<l>:<c>";
    let hits = |n: u32, at: u64| {
        line(failed(
            task(&rid(n), "t1", at, TaskOutcome::Blocked),
            FP,
            headline,
        ))
    };
    let store = Store::default();
    let flaky_line = [
        hits(1, T0),
        hits(2, T0 + DAY),
        flaky(&rid(7), T0 - DAY, "b::flaky"),
    ];
    assert!(known(&flaky_line, &store, T0 + DAY, &cfg()).is_empty());
    // Only a whole word counts: `b::fla` is not `b::flaky`.
    let partial = [
        hits(1, T0),
        hits(2, T0 + DAY),
        flaky(&rid(7), T0 - DAY, "b::fla"),
    ];
    assert_eq!(
        ids(&known(&partial, &store, T0 + DAY, &cfg())),
        vec![kf(FP)]
    );
}

#[path = "consolidate_tests_credit.rs"]
mod credit;

#[path = "consolidate_tests_lives.rs"]
mod lives;

#[path = "consolidate_tests_redact.rs"]
mod redact;

#[path = "consolidate_tests_scale.rs"]
mod scale;
