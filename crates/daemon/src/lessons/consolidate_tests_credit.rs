//! Task M9.15.4: decisions 17 and 25 (trial and credit) and the list order, over the
//! fixtures of `consolidate_tests.rs`.

use proto::LessonInfo;

use super::super::{TRIAL_ENDED, consolidate};
use super::*;

const TRIAL: &str = "the proof of the parser was not red";

/// `n` run records after `T0`, 100 s apart.
fn runs(n: u32) -> Vec<HistoryLine> {
    (1..=n)
        .map(|i| run(&rid(100 + i), T0 + u64::from(i) * 100, "accepted"))
        .collect()
}

fn one<'a>(lessons: &'a [super::super::Lesson], id: &str) -> &'a super::super::Lesson {
    lessons.iter().find(|l| l.id == id).expect(id)
}

#[test]
fn episodic_retires_after_trial_runs() {
    let id = id_of(TRIAL);
    let store = fold(&entry(&rid(0), T0, &[(LessonLayer::Episodic, TRIAL)]));
    let now = T0 + DAY;

    let got = consolidate(&store, &runs(19), now, &cfg());
    assert_eq!(one(&got, &id).status, LessonStatus::OnTrial);

    let got = consolidate(&store, &runs(20), now, &cfg());
    let lesson = one(&got, &id);
    assert_eq!(lesson.status, LessonStatus::Retired);
    assert_eq!(lesson.retired_reason.as_deref(), Some(TRIAL_ENDED));
    assert_eq!(TRIAL_ENDED, "trial ended");

    let mut green = runs(20);
    green.push(line(shown(
        task(&rid(103), "t1", T0 + 250, TaskOutcome::Merged),
        &[&id],
    )));
    let got = consolidate(&store, &green, now, &cfg());
    let lesson = one(&got, &id);
    assert_eq!(lesson.status, LessonStatus::Kept);
    assert_eq!((lesson.green, lesson.outcomes), (1, 1));
    assert_eq!(lesson.retired_reason, None);

    // Reconfirmed between runs 15 and 16: five runs since, still on trial.
    let mut lines = entry(&rid(0), T0, &[(LessonLayer::Episodic, TRIAL)]).to_vec();
    lines.extend(entry(
        &rid(115),
        T0 + 1550,
        &[(LessonLayer::Episodic, TRIAL)],
    ));
    let store = fold(&lines);
    let got = consolidate(&store, &runs(20), now, &cfg());
    assert_eq!(one(&got, &id).status, LessonStatus::OnTrial);
}

#[test]
fn revert_turns_green_to_notgreen() {
    let id = id_of(TRIAL);
    let store = fold(&entry(&rid(0), T0, &[(LessonLayer::Episodic, TRIAL)]));
    let merged = shown(task(&rid(1), "t1", T0 + 10, TaskOutcome::Merged), &[&id]);
    let mut history = vec![line(merged.clone())];
    let got = consolidate(&store, &history, T0 + 20, &cfg());
    let lesson = one(&got, &id);
    assert_eq!((lesson.green, lesson.outcomes), (1, 1));
    assert_eq!(lesson.usefulness(), Some(1000));

    history.push(revert(&merged, T0 + 30));
    let got = consolidate(&store, &history, T0 + 40, &cfg());
    let lesson = one(&got, &id);
    assert_eq!((lesson.green, lesson.outcomes), (0, 1));
    assert_eq!(lesson.usefulness(), Some(0));
}

#[test]
fn cancelled_not_counted() {
    let id = id_of(TRIAL);
    let store = fold(&entry(&rid(0), T0, &[(LessonLayer::Episodic, TRIAL)]));
    let history: Vec<HistoryLine> = [
        TaskOutcome::Cancelled,
        TaskOutcome::Unfinished,
        TaskOutcome::Reported,
    ]
    .into_iter()
    .enumerate()
    .map(|(i, outcome)| {
        line(shown(
            task(&rid(1), &format!("t{i}"), T0 + 10, outcome),
            &[&id],
        ))
    })
    .collect();
    let got = consolidate(&store, &history, T0 + 20, &cfg());
    let lesson = one(&got, &id);
    assert_eq!((lesson.green, lesson.outcomes), (0, 0));
    assert_eq!(lesson.usefulness(), None);

    let mut blocked = history.clone();
    blocked.push(line(shown(
        task(&rid(1), "t9", T0 + 10, TaskOutcome::Blocked),
        &[&id],
    )));
    let got = consolidate(&store, &blocked, T0 + 20, &cfg());
    let lesson = one(&got, &id);
    assert_eq!((lesson.green, lesson.outcomes), (0, 1));
    assert_eq!(lesson.usefulness(), Some(0));

    // Usefulness is per mille, rounded down: 2 green of 3.
    let mut mixed = blocked.clone();
    for t in ["ta", "tb"] {
        mixed.push(line(shown(
            task(&rid(1), t, T0 + 10, TaskOutcome::MergedWithoutApproval),
            &[&id],
        )));
    }
    let got = consolidate(&store, &mixed, T0 + 20, &cfg());
    assert_eq!(one(&got, &id).usefulness(), Some(666));
}

/// Milestone 9.17 decision 26 (pinning): a replaced task is cancelled by the time its
/// record is written, and a cancelled record counts as nothing (decision 25), so a
/// lesson shown only to it gains no outcome; one shown to its replacement too is judged
/// by the replacement alone.
#[test]
fn replaced_task_is_no_credit() {
    let id = id_of(TRIAL);
    let store = fold(&entry(&rid(0), T0, &[(LessonLayer::Episodic, TRIAL)]));
    let replaced = shown(task(&rid(1), "t1", T0 + 10, TaskOutcome::Cancelled), &[&id]);
    let mut replacement = task(&rid(1), "t2", T0 + 20, TaskOutcome::Blocked);
    replacement.replaces = vec!["t1".into()];
    let history = vec![line(replaced.clone()), line(replacement.clone())];
    let got = consolidate(&store, &history, T0 + 30, &cfg());
    let lesson = one(&got, &id);
    assert_eq!((lesson.green, lesson.outcomes), (0, 0));
    assert_eq!(lesson.usefulness(), None);

    let history = vec![line(replaced), line(shown(replacement, &[&id]))];
    let got = consolidate(&store, &history, T0 + 30, &cfg());
    let lesson = one(&got, &id);
    assert_eq!((lesson.green, lesson.outcomes), (0, 1));
}

#[test]
fn consolidate_lists_active_proposed_and_with_all_the_gone() {
    const APPROVED: &str = "crates/a owns the parser";
    const KEPT: &str = "the lexer test needs a fixture";
    const ENDED: &str = "a lesson nobody used";
    const ON_TRIAL: &str = "the proof was not red";
    const PROPOSED: &str = "run cargo fmt before done";
    const REJECTED: &str = "the parser is slow";
    const FORGOTTEN: &str = "the build needs network";
    let mut lines = Vec::new();
    lines.extend(entry(&rid(1), T0, &[(LessonLayer::Semantic, APPROVED)]));
    lines.push(verdict(&id_of(APPROVED), T0 + 5, true));
    lines.extend(entry(&rid(2), T0 + 10, &[(LessonLayer::Episodic, KEPT)]));
    lines.extend(entry(&rid(3), T0 + 20, &[(LessonLayer::Episodic, ENDED)]));
    lines.extend(entry(
        &rid(4),
        T0 + 5000,
        &[(LessonLayer::Episodic, ON_TRIAL)],
    ));
    lines.extend(entry(
        &rid(5),
        T0 + 30,
        &[(LessonLayer::Procedural, PROPOSED)],
    ));
    lines.extend(entry(
        &rid(6),
        T0 + 40,
        &[(LessonLayer::Semantic, REJECTED)],
    ));
    lines.push(verdict(&id_of(REJECTED), T0 + 45, false));
    lines.extend(entry(
        &rid(7),
        T0 + 50,
        &[(LessonLayer::Semantic, FORGOTTEN)],
    ));
    lines.push(retired(&id_of(FORGOTTEN), T0 + 55, "forgotten by the user"));
    lines.push(dismissed(FP2, T0 + 60));
    let store = fold(&lines);

    let mut history = runs(20);
    let kept_task = shown(
        task(&rid(103), "t1", T0 + 250, TaskOutcome::Merged),
        &[&id_of(KEPT)],
    );
    history.push(line(kept_task));
    history.extend([hit(8, T0 + 100, FP), hit(9, T0 + 200, FP)]);
    for (n, at) in [(8, T0 + 100), (9, T0 + 200)] {
        let t2 = task(&rid(n), "t2", at, TaskOutcome::Blocked);
        history.push(line(failed(t2, FP2, "another failure")));
    }

    let got = consolidate(&store, &history, T0 + DAY, &cfg());
    let order: Vec<(String, LessonStatus)> = got.iter().map(|l| (l.id.clone(), l.status)).collect();
    assert_eq!(
        order,
        vec![
            (kf(FP), LessonStatus::Known),
            (id_of(APPROVED), LessonStatus::Approved),
            (id_of(KEPT), LessonStatus::Kept),
            (id_of(ON_TRIAL), LessonStatus::OnTrial),
            (id_of(PROPOSED), LessonStatus::Proposed),
            (id_of(REJECTED), LessonStatus::Rejected),
            // Both retired: the newer `last_seen` first.
            (id_of(FORGOTTEN), LessonStatus::Retired),
            (id_of(ENDED), LessonStatus::Retired),
        ]
    );
    assert_eq!(
        one(&got, &id_of(FORGOTTEN)).retired_reason.as_deref(),
        Some("forgotten by the user")
    );
    assert_eq!(
        one(&got, &id_of(ENDED)).retired_reason.as_deref(),
        Some(TRIAL_ENDED)
    );

    assert_eq!(
        one(&got, &id_of(KEPT)).info(),
        LessonInfo {
            id: id_of(KEPT),
            layer: LessonLayer::Episodic,
            status: LessonStatus::Kept,
            origin: LessonOrigin::Distilled,
            title: KEPT.into(),
            body: format!("body of {KEPT}"),
            files: vec!["crates/a/**".into()],
            created_at: T0 + 11,
            last_seen: T0 + 11,
            runs: vec![rid(2)],
            green: 1,
            outcomes: 1,
            reconfirmed: 0,
            headlines: Vec::new(),
            retired_reason: None,
        }
    );
    let k = one(&got, &kf(FP)).info();
    assert_eq!(
        (k.origin, k.status, k.headlines.clone(), k.last_seen),
        (
            LessonOrigin::Signature,
            LessonStatus::Known,
            vec![HEADLINE.to_string()],
            T0 + 200
        )
    );
    assert_eq!(k.runs, vec![rid(8), rid(9)]);
}
