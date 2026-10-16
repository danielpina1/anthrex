//! Task M9.15.6: decision 22's scoring, in tables, and the block's bounds as a property.

use proptest::prelude::*;
use proto::{LessonLayer, LessonStatus};

use super::*;
use crate::run::model::{LessonBrief, LessonLimits};

const NOW: u64 = 1_800_000_000;
const DAY: u64 = 86_400;

fn brief(id: &str, layer: LessonLayer, status: LessonStatus, files: &[&str]) -> LessonBrief {
    LessonBrief {
        id: id.into(),
        layer,
        status,
        title: format!("title of {id}"),
        body: format!("body of {id}"),
        files: files.iter().map(|f| f.to_string()).collect(),
        last_seen: NOW,
        usefulness: None,
    }
}

fn on_trial(id: &str, files: &[&str]) -> LessonBrief {
    brief(id, LessonLayer::Episodic, LessonStatus::OnTrial, files)
}

fn owns(globs: &[&str]) -> Vec<String> {
    globs.iter().map(|g| g.to_string()).collect()
}

fn limits() -> LessonLimits {
    LessonLimits::default()
}

#[test]
fn overlap_ranks_matching_files_first() {
    // `l-a` would win the tie on id; it matches one of its two files, so it ranks
    // second (a lesson matching none is not shown at all: the next tests).
    let items = [
        brief(
            "l-a",
            LessonLayer::Semantic,
            LessonStatus::Approved,
            &["crates/z/**", "crates/a/src/x.rs"],
        ),
        brief(
            "l-b",
            LessonLayer::Semantic,
            LessonStatus::Approved,
            &["crates/a/**"],
        ),
    ];
    let picked = pick(&items, &owns(&["crates/a/src/x.rs"]), NOW, &limits());
    assert_eq!(picked.ids, ["l-b", "l-a"]);
    // A half match is half the overlap.
    let half = on_trial("l-c", &["crates/a/src/x.rs", "crates/q/y.rs"]);
    assert_eq!(overlap(&half, &owns(&["crates/a/**"])), 500);
}

#[test]
fn unmeasured_gets_median_floor_half() {
    let measured = |values: &[u16]| -> Vec<LessonBrief> {
        (values.iter().enumerate())
            .map(|(n, v)| LessonBrief {
                usefulness: Some(*v),
                ..on_trial(&format!("l-{n}"), &[])
            })
            .chain([on_trial("l-unmeasured", &[])])
            .collect()
    };
    let table: [(&[u16], u32); 5] = [
        (&[200, 900, 1000], 900),
        (&[100, 300], 500),
        (&[600, 800], 700),
        (&[0], 500),
        (&[], 500),
    ];
    for (values, want) in table {
        assert_eq!(unmeasured(&measured(values)), want, "{values:?}");
    }
}

#[test]
fn no_files_semantic_gets_overlap_300_episodic_0() {
    let o = owns(&["crates/a/**"]);
    let table = [
        (LessonLayer::Semantic, LessonStatus::Approved, 300),
        (LessonLayer::Procedural, LessonStatus::Approved, 300),
        (LessonLayer::Episodic, LessonStatus::OnTrial, 0),
        (LessonLayer::Episodic, LessonStatus::Known, 0),
    ];
    for (layer, status, want) in table {
        assert_eq!(
            overlap(&brief("l-x", layer, status, &[]), &o),
            want,
            "{layer:?}"
        );
    }
}

#[test]
fn min_score_excludes() {
    // No overlap (an episodic lesson with no files), last seen 90 days ago: the score
    // is 350 × usefulness, so 1000 reaches MIN_SCORE exactly and 999 falls short.
    let old = |id: &str, usefulness: u16| LessonBrief {
        usefulness: Some(usefulness),
        last_seen: NOW - 90 * DAY,
        ..on_trial(id, &[])
    };
    assert_eq!(score(&old("l-x", 1000), &[], NOW, 500), MIN_SCORE);
    let picked = pick(&[old("l-in", 1000), old("l-out", 999)], &[], NOW, &limits());
    assert_eq!(picked.ids, ["l-in"]);
    // Recency: 45 days is half.
    let mid = LessonBrief {
        last_seen: NOW - 45 * DAY,
        ..old("l-mid", 0)
    };
    assert_eq!(score(&mid, &[], NOW, 500), 200 * 500);
}

/// A fresh lesson: last seen now, unmeasured, so it scores 175 000 + 200 000 = 375 000
/// before any overlap.
fn fresh(id: &str, layer: LessonLayer, files: &[&str]) -> LessonBrief {
    brief(id, layer, LessonStatus::OnTrial, files)
}

#[test]
fn a_lesson_with_files_and_no_overlap_is_never_picked() {
    // The review's correction to decision 22: a lesson that has files is shown only to
    // a task whose `owns` touch at least one of them, whatever its score.
    let beta = owns(&["mods/beta/**"]);
    let alpha = fresh("l-alpha", LessonLayer::Procedural, &["mods/alpha/**"]);
    assert_eq!(overlap(&alpha, &beta), 0);
    assert!(score(&alpha, &beta, NOW, 500) >= MIN_SCORE);
    let perfect = LessonBrief {
        usefulness: Some(1000),
        status: LessonStatus::Known,
        ..fresh("l-perfect", LessonLayer::Episodic, &["mods/alpha/broken"])
    };
    assert_eq!(score(&perfect, &beta, NOW, 500), 550_000);
    assert_eq!(
        pick(&[alpha, perfect], &beta, NOW, &limits()),
        Picked::default()
    );
}

#[test]
fn a_lesson_with_files_and_some_overlap_is_picked() {
    let alpha = fresh("l-alpha", LessonLayer::Procedural, &["mods/alpha/**"]);
    let picked = pick(
        std::slice::from_ref(&alpha),
        &owns(&["mods/alpha/src.rs"]),
        NOW,
        &limits(),
    );
    assert_eq!(picked.ids, ["l-alpha"]);
    // One file of many is enough, even when the per-mille share rounds down to 0.
    let mut files: Vec<String> = (0..1500).map(|n| format!("mods/z{n}/x.rs")).collect();
    files.push("mods/beta/x.rs".into());
    let wide = LessonBrief {
        files,
        ..fresh("l-wide", LessonLayer::Semantic, &[])
    };
    let beta = owns(&["mods/beta/**"]);
    assert_eq!(overlap(&wide, &beta), 0);
    assert_eq!(pick(&[wide], &beta, NOW, &limits()).ids, ["l-wide"]);
}

#[test]
fn a_lesson_without_files_keeps_its_rule() {
    // Decision 22 unchanged: no files is overlap 300 (semantic, procedural) or 0
    // (episodic), and the score alone decides.
    let o = owns(&["mods/beta/**"]);
    let items = [
        fresh("l-episodic", LessonLayer::Episodic, &[]),
        fresh("l-procedural", LessonLayer::Procedural, &[]),
        fresh("l-semantic", LessonLayer::Semantic, &[]),
    ];
    assert_eq!(score(&items[0], &o, NOW, 500), 375_000);
    assert_eq!(score(&items[1], &o, NOW, 500), 510_000);
    let picked = pick(&items, &o, NOW, &limits());
    assert_eq!(picked.ids, ["l-procedural", "l-semantic", "l-episodic"]);
    // Below MIN_SCORE a file-less lesson still drops out (`min_score_excludes`).
    let stale = LessonBrief {
        usefulness: Some(999),
        last_seen: NOW - 90 * DAY,
        ..fresh("l-stale", LessonLayer::Episodic, &[])
    };
    assert!(pick(&[stale], &o, NOW, &limits()).ids.is_empty());
}

#[test]
fn proposed_lessons_never_selected() {
    let perfect = |id: &str, status| LessonBrief {
        usefulness: Some(1000),
        ..brief(id, LessonLayer::Semantic, status, &["crates/a/**"])
    };
    let items = [
        perfect("l-proposed", LessonStatus::Proposed),
        perfect("l-rejected", LessonStatus::Rejected),
        perfect("l-retired", LessonStatus::Retired),
        perfect("l-kept", LessonStatus::Kept),
    ];
    let picked = pick(&items, &owns(&["crates/a/**"]), NOW, &limits());
    assert_eq!(picked.ids, ["l-kept"]);
}

#[test]
fn ties_break_on_id() {
    let items = [
        on_trial("l-c", &["crates/a/**"]),
        on_trial("l-a", &["crates/a/**"]),
        on_trial("l-b", &["crates/a/**"]),
    ];
    let picked = pick(&items, &owns(&["crates/a/**"]), NOW, &limits());
    assert_eq!(picked.ids, ["l-a", "l-b", "l-c"]);
    // At most `max_items`.
    let one = LessonLimits {
        max_items: 1,
        ..limits()
    };
    assert_eq!(
        pick(&items, &owns(&["crates/a/**"]), NOW, &one).ids,
        ["l-a"]
    );
}

fn text() -> impl Strategy<Value = String> {
    prop::collection::vec(
        prop_oneof![
            Just("-".to_string()),
            Just("\u{1b}".to_string()),
            Just("\n".to_string()),
            Just("é".to_string()),
            "[a-z ]{1,12}",
        ],
        0..120,
    )
    .prop_map(|parts| parts.concat())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]
    #[test]
    fn block_never_exceeds_block_chars_item_item_chars(
        lessons in prop::collection::vec((text(), text()), 0..12),
        block_chars in 500u32..=2000,
        item_chars in 100u32..=600,
        max_items in 1u32..=10,
    ) {
        let items: Vec<LessonBrief> = (lessons.into_iter().enumerate())
            .map(|(n, (title, body))| LessonBrief {
                title,
                body,
                ..on_trial(&format!("l-{n:02}"), &["crates/a/**"])
            })
            .collect();
        let limits = LessonLimits { block_chars, item_chars, max_items, ..limits() };
        let picked = pick(&items, &owns(&["crates/a/**"]), NOW, &limits);
        prop_assert!(picked.items.len() <= max_items as usize);
        prop_assert_eq!(picked.items.len(), picked.ids.len());
        for item in &picked.items {
            prop_assert!(item.chars().count() <= item_chars as usize, "{}", item);
            prop_assert!(!item.contains('\n'), "{}", item);
        }
        for heading in [
            crate::run::contract_lessons::WORKER_HEADING,
            crate::run::contract_lessons::REVIEWER_HEADING,
        ] {
            let block = render(heading, &picked);
            prop_assert!(block.chars().count() <= block_chars as usize, "{}", block);
        }
    }
}
