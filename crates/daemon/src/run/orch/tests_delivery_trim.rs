//! Task M9.2.13's fix round: the delivery block's trim steps, each pinned on its own,
//! and the cap held whatever the delivery holds (the controller's ruling, I1, m1, m5).
//! Pure.

use super::*;
use crate::run::delivery::digest::{FIX_TASKS_LAST, STRINGS_LAST, THREADS_LAST};
use crate::run::delivery::quote::CUT_NOTE;

/// Every comment left is `null` or a quote whose fence closes.
fn closed(d: &Value) -> bool {
    (d["stages"].as_array().unwrap().iter())
        .flat_map(|s| s["threads"].as_array().into_iter().flatten())
        .all(|t| match &t["comment"] {
            Value::Null => true,
            Value::String(c) => c.ends_with(&format!("```\n{CUT_NOTE}")) || c.ends_with("```\n"),
            _ => false,
        })
}

/// m5: threads cut to 10 while the resolved stages are still shown, when that alone
/// fits.
#[test]
fn the_thread_cut_alone_keeps_resolved_stages() {
    let run = heavy(4, 3, THREADS_SHOWN, 250);
    let d = digest(&run, NOW);
    assert!(size(&d) <= DIGEST_MAX_BYTES, "{}", size(&d));
    let block = &d["delivery"];
    let shown: Vec<u64> = (block["stages"].as_array().unwrap().iter())
        .map(|s| s["stage"].as_u64().unwrap())
        .collect();
    assert_eq!(shown, [1, 2, 3, 4], "the merged stages 1 and 2 are kept");
    assert_eq!(thread_counts(block), [THREADS_TRIMMED; 4]);
    let all = comments(block);
    assert_eq!(all.len(), 4 * THREADS_TRIMMED);
    assert!(all.iter().all(|c| c.chars().count() <= COMMENT_TRIMMED));
    assert!(closed(block));
    assert_eq!(d["omitted_tasks"], 0);
}

/// m1: a block still over the cap after decision 29's three steps loses its comments
/// before the digest's general 120-character cut, which would cut a fence off.
#[test]
fn comments_go_before_the_string_cut_could_break_a_fence() {
    let run = heavy(16, 1, THREADS_TRIMMED, 10);
    let d = digest(&run, NOW);
    assert!(size(&d) <= DIGEST_MAX_BYTES, "{}", size(&d));
    let block = &d["delivery"];
    assert_eq!(thread_counts(block), [THREADS_TRIMMED; 16]);
    assert!(closed(block), "{block}");
    assert!(comments(block).is_empty(), "every comment is null");
    let first = &block["stages"][0]["threads"][0];
    assert_eq!(first["comment"], Value::Null);
    assert_eq!(d["omitted_tasks"], 0);
}

/// I1: `STAGES_MAX` open stages of 20 threads on long paths by a 39-character login,
/// with two fix tasks a stage: the digest fits and no task is dropped, because the
/// delivery block reaches its bounded form first.
#[test]
fn the_cap_holds_whatever_the_delivery_holds() {
    let stages = proto::STAGES_MAX;
    let mut run = heavy(stages, 1, THREADS_SHOWN, 250);
    let login = "a".repeat(39);
    run.delivery.limits.reviewers = vec![login.clone()];
    for s in &mut run.delivery.stages {
        for t in &mut s.threads {
            t.author = login.clone();
            t.comments[0].author = login.clone();
        }
    }
    for n in 1..=stages {
        let plan = run.task(&format!("t{n}")).unwrap().clone();
        for (k, origin) in [(0, TaskOrigin::Ci), (1, TaskOrigin::Review)] {
            let mut fix = plan.clone();
            fix.spec.id = format!("fix{:0>13}", u32::from(n) * 2 + k);
            fix.origin = origin;
            run.tasks.push(fix);
        }
    }
    // Without the delivery block the run's tasks fit: the block is what is trimmed.
    let mut local = run.clone();
    local.delivery.mode = DeliveryMode::Local;
    assert_eq!(digest(&local, NOW)["omitted_tasks"], 0);
    let d = digest(&run, NOW);
    assert!(size(&d) <= DIGEST_MAX_BYTES, "{}", size(&d));
    assert_eq!(d["omitted_tasks"], 0, "no task goes for the delivery block");
    let block = &d["delivery"];
    let listed = block["stages"].as_array().unwrap();
    assert_eq!(listed.len(), usize::from(stages));
    for s in listed {
        match s.get("threads_omitted") {
            Some(n) => assert_eq!(n, THREADS_SHOWN, "{s}"),
            None => {
                let threads = s["threads"].as_array().unwrap();
                assert!(threads.len() <= THREADS_LAST, "{s}");
                assert!(s["fix_tasks"]["ci"].as_array().unwrap().len() <= FIX_TASKS_LAST);
            }
        }
    }
    assert!(closed(block));
    // The last, bounded form: each stage's PR, state and CI, and a count; and the
    // string cut before it.
    let compact =
        crate::run::delivery::digest::block(&run, crate::run::delivery::digest::Shape::LAST[4]);
    assert_eq!(
        compact["stages"][0],
        json!({"stage": 1, "pr": 101, "state": "open", "ci": "none", "threads_omitted": 20})
    );
    assert!(size(&compact) < 4 * 1024, "{}", size(&compact));
    let strings =
        crate::run::delivery::digest::block(&run, crate::run::delivery::digest::Shape::LAST[2]);
    let file = strings["stages"][0]["threads"][0]["file"].as_str().unwrap();
    assert!(file.chars().count() <= STRINGS_LAST + 1, "{file}");
}

/// The final fix wave (task 13's deferred item): the digest's general string cut runs
/// before the delivery block's last steps, so `LAST[0]` and `LAST[1]`, which rebuild the
/// block, apply it too; without it they would bring the long strings back.
#[test]
fn the_first_last_steps_keep_the_general_string_cut() {
    use crate::run::delivery::digest::{STRINGS_GENERAL, Shape, block};
    let run = heavy(2, 1, THREADS_SHOWN, 400);
    for shape in &Shape::LAST[..2] {
        let b = block(&run, *shape);
        let file = b["stages"][0]["threads"][0]["file"].as_str().unwrap();
        assert!(file.chars().count() <= STRINGS_GENERAL + 1, "{file}");
    }
}
