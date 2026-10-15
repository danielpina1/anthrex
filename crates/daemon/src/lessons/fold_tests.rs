//! Task M9.15.3: decision 4's fold and decisions 15 and 16's dedup, with ruling C16
//! (a verdict applies only to a proposed or on-trial lesson).

use proto::{
    DismissedLine, DistillCause, DistilledLine, GaveUpLine, LESSONS_VERSION, LessonDraft,
    LessonLayer, LessonLine, LessonOrigin, LessonStatus, QueuedLine, RetiredLine, VerdictLine,
};

use super::{RUNS_MAX, fold, fold_text, title_fp};

fn queued(run: &str, task: &str, at: u64, fps: &[&str]) -> LessonLine {
    LessonLine::Queued(QueuedLine {
        v: LESSONS_VERSION,
        record_id: format!("{run}/{task}/distill"),
        at,
        run_id: run.to_string(),
        task_id: Some(task.to_string()),
        cause: DistillCause::TaskRecovered,
        dossier: "## outcome".to_string(),
        fps: fps.iter().map(|s| s.to_string()).collect(),
    })
}

fn item(layer: LessonLayer, title: &str) -> LessonDraft {
    LessonDraft {
        layer,
        title: title.to_string(),
        body: format!("body of {title}"),
        files: vec!["crates/a/**".to_string()],
    }
}

fn distilled(run: &str, task: &str, at: u64, items: Vec<LessonDraft>) -> LessonLine {
    let queued = format!("{run}/{task}/distill");
    LessonLine::Distilled(DistilledLine {
        v: LESSONS_VERSION,
        record_id: format!("{queued}/distilled"),
        at,
        queued,
        items,
        dropped: 0,
        usage: None,
    })
}

fn verdict(id: &str, at: u64, approved: bool) -> LessonLine {
    LessonLine::Verdict(VerdictLine {
        v: LESSONS_VERSION,
        record_id: format!("{id}/verdict/{at}"),
        at,
        id: id.to_string(),
        approved,
    })
}

fn retired(id: &str, at: u64, reason: &str) -> LessonLine {
    LessonLine::Retired(RetiredLine {
        v: LESSONS_VERSION,
        record_id: format!("{id}/retired"),
        at,
        id: id.to_string(),
        reason: reason.to_string(),
    })
}

fn id_of(title: &str) -> String {
    format!("l-{}", title_fp(title))
}

#[test]
fn distilled_items_create_on_trial_and_proposed() {
    let lines = [
        queued("r1-aaaa", "t1", 10, &["0123456789abcdef"]),
        distilled(
            "r1-aaaa",
            "t1",
            20,
            vec![
                item(LessonLayer::Episodic, "the proof of t1 was not red"),
                item(LessonLayer::Semantic, "crates/a owns the parser"),
                item(
                    LessonLayer::Procedural,
                    "run cargo fmt before claiming done",
                ),
            ],
        ),
    ];
    let store = fold(&lines);
    assert!(store.problems.is_empty());
    assert_eq!(store.lessons.len(), 3);
    let episodic = &store.lessons[&id_of("the proof of t1 was not red")];
    assert_eq!(episodic.status, LessonStatus::OnTrial);
    assert_eq!(episodic.layer, LessonLayer::Episodic);
    assert_eq!(episodic.origin, LessonOrigin::Distilled);
    assert_eq!(episodic.trial_start, Some(20));
    assert_eq!((episodic.created_at, episodic.last_seen), (20, 20));
    assert_eq!(episodic.runs, ["r1-aaaa"]);
    assert_eq!(episodic.reconfirmed, 0);
    assert_eq!(episodic.queued.as_deref(), Some("r1-aaaa/t1/distill"));
    assert_eq!(episodic.files, ["crates/a/**"]);
    assert_eq!(episodic.body, "body of the proof of t1 was not red");
    for title in [
        "crates/a owns the parser",
        "run cargo fmt before claiming done",
    ] {
        let lesson = &store.lessons[&id_of(title)];
        assert_eq!(lesson.status, LessonStatus::Proposed, "{title}");
        assert_eq!(lesson.trial_start, None, "{title}");
        assert_eq!(lesson.title, title);
    }
    assert_eq!(
        id_of("run cargo fmt before claiming done"),
        "l-023b379bb7d41db9"
    );
    // The entry is closed by its answer.
    let entry = &store.queued["r1-aaaa/t1/distill"];
    assert!(!entry.open);
    assert_eq!(entry.run_id, "r1-aaaa");
    assert_eq!(entry.fps, ["0123456789abcdef"]);
}

#[test]
fn duplicate_title_reconfirms_not_duplicates() {
    let lines = [
        queued("r1-aaaa", "t1", 10, &[]),
        distilled(
            "r1-aaaa",
            "t1",
            20,
            vec![item(
                LessonLayer::Procedural,
                "run cargo fmt before claiming done",
            )],
        ),
        queued("r2-bbbb", "t4", 30, &[]),
        distilled(
            "r2-bbbb",
            "t4",
            40,
            vec![item(
                LessonLayer::Procedural,
                "Run cargo fmt  before CLAIMING done!",
            )],
        ),
    ];
    let store = fold(&lines);
    assert_eq!(store.lessons.len(), 1, "{:?}", store.lessons.keys());
    let lesson = &store.lessons[&id_of("run cargo fmt before claiming done")];
    assert_eq!(lesson.reconfirmed, 1);
    assert_eq!(lesson.last_seen, 40);
    assert_eq!(lesson.created_at, 20);
    assert_eq!(lesson.runs, ["r1-aaaa", "r2-bbbb"]);
    // The first wording is kept.
    assert_eq!(lesson.title, "run cargo fmt before claiming done");
    assert_eq!(lesson.status, LessonStatus::Proposed);
}

#[test]
fn a_reconfirmed_episodic_lesson_restarts_its_trial_and_keeps_ten_runs() {
    let title = "the check of t1 flaked";
    let mut lines = Vec::new();
    for n in 0..12u64 {
        let run = format!("r{n}-{n:04x}");
        lines.push(queued(&run, "t1", 100 * n, &[]));
        lines.push(distilled(
            &run,
            "t1",
            100 * n + 1,
            vec![item(LessonLayer::Episodic, title)],
        ));
    }
    // The same run twice counts its run once.
    lines.push(queued("r11-000b", "t2", 1999, &[]));
    lines.push(distilled(
        "r11-000b",
        "t2",
        2000,
        vec![item(LessonLayer::Episodic, title)],
    ));
    let store = fold(&lines);
    let lesson = &store.lessons[&id_of(title)];
    assert_eq!(lesson.reconfirmed, 12);
    assert_eq!(lesson.trial_start, Some(2000));
    assert_eq!(lesson.created_at, 1);
    assert_eq!(lesson.runs.len(), RUNS_MAX);
    assert_eq!(lesson.runs.first().map(String::as_str), Some("r2-0002"));
    assert_eq!(lesson.runs.last().map(String::as_str), Some("r11-000b"));
}

#[test]
fn rejected_fp_never_reproposed() {
    let title = "crates/a owns the parser";
    let id = id_of(title);
    let lines = [
        queued("r1-aaaa", "t1", 10, &[]),
        distilled(
            "r1-aaaa",
            "t1",
            20,
            vec![item(LessonLayer::Semantic, title)],
        ),
        verdict(&id, 30, false),
        queued("r2-bbbb", "t1", 40, &[]),
        distilled(
            "r2-bbbb",
            "t1",
            50,
            vec![item(LessonLayer::Semantic, "Crates/A owns the PARSER.")],
        ),
    ];
    let store = fold(&lines);
    assert_eq!(store.lessons.len(), 1);
    let lesson = &store.lessons[&id];
    assert_eq!(lesson.status, LessonStatus::Rejected);
    assert_eq!(lesson.reconfirmed, 0);
    assert_eq!(lesson.last_seen, 20);
    assert_eq!(lesson.runs, ["r1-aaaa"]);
}

#[test]
fn a_verdict_on_an_approved_lesson_is_ignored() {
    let title = "crates/a owns the parser";
    let id = id_of(title);
    let lines = [
        distilled(
            "r1-aaaa",
            "t1",
            20,
            vec![item(LessonLayer::Semantic, title)],
        ),
        verdict(&id, 30, true),
        verdict(&id, 31, false),
    ];
    assert_eq!(fold(&lines).lessons[&id].status, LessonStatus::Approved);
    // An on-trial lesson can be approved or rejected.
    let episodic = "the proof of t1 was not red";
    let eid = id_of(episodic);
    let lines = [
        distilled(
            "r1-aaaa",
            "t1",
            20,
            vec![item(LessonLayer::Episodic, episodic)],
        ),
        verdict(&eid, 30, false),
        verdict(&eid, 31, true),
    ];
    assert_eq!(fold(&lines).lessons[&eid].status, LessonStatus::Rejected);
}

#[test]
fn verdict_and_retired_apply_only_to_lessons_that_exist() {
    let title = "run cargo fmt before claiming done";
    let id = id_of(title);
    let lines = [
        verdict("l-ffffffffffffffff", 5, true),
        retired("l-eeeeeeeeeeeeeeee", 6, "forgotten by the user"),
        distilled(
            "r1-aaaa",
            "t1",
            20,
            vec![item(LessonLayer::Procedural, title)],
        ),
        verdict(&id, 30, true),
        retired(&id, 40, "forgotten by the user"),
        // A second retirement keeps the first reason.
        retired(&id, 41, "something else"),
    ];
    let store = fold(&lines);
    assert_eq!(store.lessons.len(), 1);
    let lesson = &store.lessons[&id];
    assert_eq!(lesson.status, LessonStatus::Retired);
    assert_eq!(
        lesson.retired_reason.as_deref(),
        Some("forgotten by the user")
    );
    // Nothing about the unknown ids is a problem; they are ignored.
    assert!(store.problems.is_empty());
    // A rejected lesson is not retired over.
    let lines = [
        distilled(
            "r1-aaaa",
            "t1",
            20,
            vec![item(LessonLayer::Procedural, title)],
        ),
        verdict(&id, 30, false),
        retired(&id, 40, "forgotten by the user"),
    ];
    let store = fold(&lines);
    assert_eq!(store.lessons[&id].status, LessonStatus::Rejected);
    assert_eq!(store.lessons[&id].retired_reason, None);
}

#[test]
fn retired_lesson_title_is_dropped_when_distilled_again() {
    let title = "the proof of t1 was not red";
    let id = id_of(title);
    let lines = [
        distilled(
            "r1-aaaa",
            "t1",
            20,
            vec![item(LessonLayer::Episodic, title)],
        ),
        retired(&id, 30, "forgotten by the user"),
        distilled(
            "r2-bbbb",
            "t1",
            40,
            vec![item(LessonLayer::Episodic, title)],
        ),
    ];
    let store = fold(&lines);
    assert_eq!(store.lessons.len(), 1);
    let lesson = &store.lessons[&id];
    assert_eq!(lesson.status, LessonStatus::Retired);
    assert_eq!((lesson.reconfirmed, lesson.last_seen), (0, 20));
    assert_eq!(lesson.trial_start, Some(20));
}

#[test]
fn queued_entries_track_attempts_and_close() {
    let attempt = |n: u8, at: u64| {
        LessonLine::Attempt(proto::AttemptLine {
            v: LESSONS_VERSION,
            record_id: format!("r1-aaaa/t1/distill/attempt/{n}"),
            at,
            queued: "r1-aaaa/t1/distill".to_string(),
            n,
            error: Some("timed out".to_string()),
            usage: None,
        })
    };
    let lines = [
        queued("r1-aaaa", "t1", 10, &["ab"]),
        queued("r1-aaaa", "t2", 11, &[]),
        attempt(1, 12),
        attempt(2, 13),
        attempt(3, 14),
        LessonLine::GaveUp(GaveUpLine {
            v: LESSONS_VERSION,
            record_id: "r1-aaaa/t1/distill/gave_up".to_string(),
            at: 15,
            queued: "r1-aaaa/t1/distill".to_string(),
            reason: "timed out".to_string(),
        }),
        LessonLine::Dismissed(DismissedLine {
            v: LESSONS_VERSION,
            record_id: "kf-0123456789abcdef/dismissed".to_string(),
            at: 16,
            fp: "0123456789abcdef".to_string(),
        }),
    ];
    let store = fold(&lines);
    let t1 = &store.queued["r1-aaaa/t1/distill"];
    assert_eq!((t1.open, t1.attempts), (false, 3));
    let t2 = &store.queued["r1-aaaa/t2/distill"];
    assert_eq!((t2.open, t2.attempts, t2.at), (true, 0, 11));
    assert_eq!(t2.task_id.as_deref(), Some("t2"));
    assert!(store.dismissed.contains("0123456789abcdef"));
    assert!(store.lessons.is_empty());
}

#[test]
fn unknown_and_torn_lines_are_skipped_and_counted() {
    let good = serde_json::to_string(&distilled(
        "r1-aaaa",
        "t1",
        20,
        vec![item(
            LessonLayer::Procedural,
            "run cargo fmt before claiming done",
        )],
    ))
    .unwrap();
    let text = format!(
        "{}\n{{\"type\":\"lesson\",\"v\":2,\"record_id\":\"x\",\"at\":1}}\n\n{good}\nnot json\n{{\"type\":\"retired\",\"v\":1,\"rec",
        serde_json::to_string(&queued("r1-aaaa", "t1", 10, &[])).unwrap()
    );
    let store = fold_text(&text, "lessons.jsonl");
    assert_eq!(store.lessons.len(), 1);
    assert_eq!(store.queued.len(), 1);
    assert_eq!(store.problems.len(), 3, "{:?}", store.problems);
    assert!(
        store.problems[0].starts_with("lessons.jsonl line 2: "),
        "{:?}",
        store.problems
    );
    assert!(store.problems[1].starts_with("lessons.jsonl line 5: "));
    assert!(store.problems[2].starts_with("lessons.jsonl line 6: "));
}

#[test]
fn title_fp_is_stable() {
    for (title, fp) in [
        ("run cargo fmt before claiming done", "023b379bb7d41db9"),
        ("Run cargo fmt  before CLAIMING done!", "023b379bb7d41db9"),
        ("CI needs `cargo fmt --check`", "fa586e842f1e146d"),
        ("t1: the proof's red tail was empty", "430fbfaa06b02ffd"),
        ("Ünïcode Tést", "843dad133e0b06d3"),
        ("x", "2d711642b726b044"),
        ("   ", "e3b0c44298fc1c14"),
    ] {
        assert_eq!(title_fp(title), fp, "{title:?}");
    }
}
