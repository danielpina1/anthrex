//! Task M9.6: the digest's trimming past its cap (decision 16 and the review fixes).
//! Pure.

use super::*;

fn size(d: &Value) -> usize {
    crate::run::orch::json::size(d)
}

fn shown(d: &Value) -> Vec<&str> {
    d["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["id"].as_str().unwrap())
        .collect()
}

/// Beyond the Interfaces: the cap holds for any run, however much its agents wrote.
#[test]
fn the_cap_holds_whatever_the_run_holds() {
    let mut run = run_of(60);
    run.goal = "g".repeat(20_000);
    for task in run.tasks.iter_mut() {
        task.spec.title = "t".repeat(120);
        task.spec.deps = (0..20).map(|i| format!("dep-{i:012}")).collect();
        block(task, BlockReason::Question, &"q".repeat(4_000));
        event(task, 1, &"h".repeat(4_000));
    }
    run.orch.run_scouts = (0..50)
        .map(|i| {
            let mut s = scout(&format!("3f9a-s{i}"), RunScoutState::Queued, &["x/**"]);
            s.question = "?".repeat(2_000);
            s
        })
        .collect();
    let d = digest(&run, NOW);
    assert!(
        crate::run::orch::json::size(&d) <= DIGEST_MAX_BYTES,
        "{}",
        crate::run::orch::json::size(&d)
    );
    assert_eq!(
        d["tasks"].as_array().unwrap().len() as u64 + d["omitted_tasks"].as_u64().unwrap(),
        60
    );
}

/// M9.6 review fix I-3: at the default `max_tasks`, 50 blocked tasks with
/// 500-character CJK reasons keep every task; their texts are shortened instead.
#[test]
fn texts_are_shortened_before_an_unfinished_task_is_dropped() {
    let mut run = run_of(50);
    assert_eq!(
        config::Orchestrator::default().max_tasks,
        50,
        "the default max_tasks"
    );
    for task in run.tasks.iter_mut() {
        block(task, BlockReason::Question, &"語".repeat(500));
    }
    let d = digest(&run, NOW);
    let size = crate::run::orch::json::size(&d);
    assert!(size <= DIGEST_MAX_BYTES, "{size}");
    assert_eq!(d["omitted_tasks"], 0);
    let tasks = d["tasks"].as_array().unwrap();
    assert_eq!(tasks.len(), 50);
    for task in tasks {
        let text = task["block"]["text"].as_str().unwrap();
        assert!(text.chars().count() <= BLOCK_TEXT_TRIMMED + 1, "{text}");
        assert!(text.starts_with('語'));
    }
}

/// M9.6 review fix I-3, second review: with tasks dropped at the end of the plan,
/// `attention` never names a task that `tasks` left out. Each task carries 400
/// dependency ids, so only a few tasks fit and the attention lines of the ten kept by
/// the attention cut must lose those of the tasks dropped after it.
#[test]
fn attention_names_only_tasks_the_digest_shows() {
    let mut run = run_of(50);
    for task in run.tasks.iter_mut() {
        task.spec.deps = (0..400).map(|i| format!("d{i:015}")).collect();
        block(task, BlockReason::Question, &"語".repeat(500));
    }
    let d = digest(&run, NOW);
    assert!(size(&d) <= DIGEST_MAX_BYTES, "{}", size(&d));
    let shown = shown(&d);
    assert!(
        !shown.is_empty() && shown.len() < 10,
        "the shape must show fewer tasks than the attention cut keeps: {shown:?}"
    );
    let blocked: Vec<&str> = d["attention"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|l| {
            l.as_str()
                .unwrap()
                .split_once(" blocked (")
                .map(|(id, _)| id)
        })
        .collect();
    assert_eq!(blocked, shown, "{}", d["attention"]);
}

/// M9.6 second review (Important): the run-wide attention lines (a moved base, a
/// failed final check, a stale profile, a promotion) are never cut for blocked tasks'
/// lines; within the 10-line cap they come first.
#[test]
fn run_wide_attention_lines_survive_the_cut() {
    let mut run = run_of(50);
    for task in run.tasks.iter_mut() {
        block(task, BlockReason::Question, &"語".repeat(500));
    }
    run.final_check_failed = true;
    let d = digest(&run, NOW);
    assert!(size(&d) <= DIGEST_MAX_BYTES, "{}", size(&d));
    let lines = d["attention"].as_array().unwrap();
    assert!(lines.len() <= 10, "{lines:?}");
    assert_eq!(lines[0], "final check failed on the run head", "{lines:?}");
    assert!(
        lines[1..]
            .iter()
            .all(|l| l.as_str().unwrap().contains(" blocked ("))
    );
}

/// M9.6 second review (Minor 1): scouts' and planners' texts go to 40 characters,
/// then both lists to 3 entries, before an unfinished task is dropped. The texts are
/// `\u{1}`, which JSON writes in 6 bytes, and `𝄞`, 4 bytes in UTF-8.
#[test]
fn scout_and_planner_texts_are_shortened_before_a_task_is_dropped() {
    for unit in ["\u{1}", "𝄞"] {
        let text = |n: usize| unit.repeat(n);
        let mut run = run_of(10);
        run.goal = text(2000);
        for task in run.tasks.iter_mut() {
            task.spec.title = text(120);
            block(task, BlockReason::Question, &text(500));
            event(task, 1, &text(300));
        }
        run.orch.run_scouts = (0..50)
            .map(|i| {
                let mut s = scout(
                    &format!("3f9a-s{i}"),
                    RunScoutState::Failed { reason: text(300) },
                    &["x/**"],
                );
                s.question = text(500);
                s
            })
            .collect();
        run.orch.epics = (0..50)
            .map(|i| {
                let mut e = EpicRecord::new(&format!("e{i}"), PlannerPhase::Finished);
                e.last_rejection = Some(text(300));
                e.note = Some(text(300));
                e
            })
            .collect();
        let d = digest(&run, NOW);
        assert!(size(&d) <= DIGEST_MAX_BYTES, "{unit:?}: {}", size(&d));
        assert_eq!(d["omitted_tasks"], 0, "{unit:?}");
        assert_eq!(shown(&d).len(), 10, "{unit:?}");
    }
}

/// M9.6 second review (Minor 3): an edit's `recipients` is capped at 20 in the digest,
/// the rest counted in `recipients_omitted`.
#[test]
fn edit_recipients_are_capped_with_an_omitted_count() {
    let mut run = run_of(50);
    for task in run.tasks.iter_mut() {
        block(task, BlockReason::Question, &"q".repeat(300));
    }
    run.plan_edits = (0..10)
        .map(|i| PlanEditRecord {
            at: 2_000 + i,
            text: "message running (info)".into(),
            source: "orchestrator".into(),
            accepted: true,
            error: None,
            recipients: (0..500).map(|r| format!("r{r:015}")).collect(),
        })
        .collect();
    let d = digest(&run, NOW);
    assert!(size(&d) <= DIGEST_MAX_BYTES, "{}", size(&d));
    assert_eq!(d["omitted_tasks"], 0);
    for edit in d["edits"].as_array().unwrap() {
        assert_eq!(edit["recipients"].as_array().unwrap().len(), 20, "{edit}");
        assert_eq!(edit["recipients_omitted"], 480, "{edit}");
    }
    // A short list has no omitted count, so the Interfaces' shape is unchanged.
    run.plan_edits
        .iter_mut()
        .for_each(|e| e.recipients.truncate(2));
    let d = digest(&run, NOW);
    assert!(d["edits"][0].get("recipients_omitted").is_none());
}

/// Ruling C-24: stages with long failing and flaky lists and a long note never push
/// the digest past its cap, and are cut before any unfinished task is dropped (the
/// review's reproduction: 32 stages, 50 failing and 40 flaky names of about 320
/// characters each, a 5000-character note).
#[test]
fn stages_are_cut_before_an_unfinished_task_and_the_cap_holds() {
    use crate::run::model::{StageLayout, StageRecord, TierRecord};
    let mut run = run_of(50);
    run.stage_layout = StageLayout::Multi;
    let name = |kind: &str, s: u16, i: usize| format!("{kind}::s{s}::t{i}::{}", "x".repeat(300));
    run.stages = (1..=32u16)
        .map(|n| {
            let mut s = StageRecord::new(
                n,
                format!("anthrex/{}/stage-{n}", run.id),
                &"a".repeat(40),
                Default::default(),
                0,
            );
            s.full.note = Some("n".repeat(5_000));
            s.full.last = Some(TierRecord {
                tier: 3,
                affected: "full suite (full suite)".into(),
                steps: 1,
                cached: 0,
                ok: false,
                secs: 60,
                flaky: (0..40).map(|i| name("flaky", n, i)).collect(),
                failing: (0..50).map(|i| name("failing", n, i)).collect(),
                at: 1,
                commit: "a".repeat(40),
            });
            s
        })
        .collect();
    let d = digest(&run, NOW);
    assert!(size(&d) <= DIGEST_MAX_BYTES, "{} bytes", size(&d));
    assert_eq!(shown(&d).len(), 50, "every unfinished task is kept");
    assert_eq!(d["omitted_tasks"], 0);
    // What is left of the stages is cut, never absent.
    let stages = d["stages"].as_array().unwrap();
    assert!(!stages.is_empty());
    for s in stages {
        assert!(s["full"]["failing"].as_array().unwrap().len() <= 20);
        assert!(s["full"]["flaky"].as_array().unwrap().len() <= 20);
    }
}

/// Ruling C-24 at build time: before any trim, a stage's lists hold at most 20 names
/// and every string is cut like the digest's other engine lines.
#[test]
fn stage_strings_and_lists_are_capped_when_built() {
    use crate::run::model::{StageLayout, StageRecord, TierRecord};
    let mut run = run_of(1);
    run.stage_layout = StageLayout::Multi;
    let mut s = StageRecord::new(1, "b".into(), &"a".repeat(40), Default::default(), 0);
    s.full.note = Some("n".repeat(5_000));
    s.full.last = Some(TierRecord {
        tier: 3,
        affected: String::new(),
        steps: 1,
        cached: 0,
        ok: false,
        secs: 1,
        flaky: (0..40)
            .map(|i| format!("f{i}{}", "x".repeat(400)))
            .collect(),
        failing: (0..50)
            .map(|i| format!("t{i}{}", "x".repeat(400)))
            .collect(),
        at: 1,
        commit: "a".repeat(40),
    });
    run.stages = vec![s];
    let d = digest(&run, NOW);
    let full = &d["stages"][0]["full"];
    assert_eq!(full["flaky"].as_array().unwrap().len(), 20);
    assert_eq!(full["failing"].as_array().unwrap().len(), 20);
    let longest = full["flaky"]
        .as_array()
        .unwrap()
        .iter()
        .chain(full["failing"].as_array().unwrap())
        .chain([&full["note"]])
        .map(|v| v.as_str().unwrap().chars().count())
        .max()
        .unwrap();
    // 300 characters and the cut marker `…`, as `cut` writes every digest line.
    assert_eq!(longest, 301);
}

/// Ruling C-24's order: a stage's lists are cut before any stage is dropped, so ten
/// stages whose names alone overflow the cap are all kept, each with its first three.
#[test]
fn a_stages_lists_are_cut_before_the_stages_themselves() {
    use crate::run::model::{StageLayout, StageRecord, TierRecord};
    let mut run = run_of(10);
    run.stage_layout = StageLayout::Multi;
    let name = |kind: &str, s: u16, i: usize| format!("{kind}::s{s}::t{i}::{}", "x".repeat(300));
    run.stages = (1..=10u16)
        .map(|n| {
            let mut s =
                StageRecord::new(n, format!("b{n}"), &"a".repeat(40), Default::default(), 0);
            s.full.last = Some(TierRecord {
                tier: 3,
                affected: String::new(),
                steps: 1,
                cached: 0,
                ok: false,
                secs: 1,
                flaky: (0..20).map(|i| name("flaky", n, i)).collect(),
                failing: (0..20).map(|i| name("failing", n, i)).collect(),
                at: 1,
                commit: "a".repeat(40),
            });
            s
        })
        .collect();
    let d = digest(&run, NOW);
    assert!(size(&d) <= DIGEST_MAX_BYTES, "{} bytes", size(&d));
    assert_eq!(shown(&d).len(), 10);
    let stages = d["stages"].as_array().unwrap();
    assert_eq!(stages.len(), 10, "no stage dropped");
    for s in stages {
        assert_eq!(s["full"]["failing"].as_array().unwrap().len(), 3);
        assert_eq!(s["full"]["flaky"].as_array().unwrap().len(), 3);
    }
}
