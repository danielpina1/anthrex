//! Task M9.11.10 (decision 11, B5): a brief's character cap and the plan's byte cap, at
//! every place a plan is validated: a plan file, `run edit` (a user batch) and
//! `edit_plan` (an orchestrator batch). Pure.

use proto::PlanEdit;

use crate::run::edits::apply_edits;
use crate::run::orch::EditSource;
use crate::run::plan::{PlanError, parse_plan};
use crate::run::test_support::{PROFILE, errors_of, plan_with, run_ok, task_toml};
use crate::run::validate::{BRIEF_MAX_CHARS, EditScope, PLAN_BRIEFS_MAX_BYTES};

fn with_brief(id: &str, brief: &str) -> String {
    task_toml(id, "S", &format!("[\"crates/{id}/src/lib.rs\"]"), "").replace(
        &format!("brief = \"Brief {id}\""),
        &format!("brief = \"{brief}\""),
    )
}

fn texts(errors: &[PlanError]) -> Vec<String> {
    errors.iter().map(ToString::to_string).collect()
}

fn edited(source: EditSource, edits: Vec<PlanEdit>) -> Vec<String> {
    let run = run_ok(&plan_with(PROFILE, &[with_brief("t1", "short")]));
    match apply_edits(&run, &edits, &EditScope::Run, &source, 5_000) {
        Ok(_) => panic!("the batch must be refused"),
        Err(errors) => texts(&errors),
    }
}

fn amend(brief: String) -> PlanEdit {
    PlanEdit::AmendTask {
        task_id: "t1".into(),
        brief: Some(brief),
        acceptance: None,
        route: None,
        test_mode: None,
        test_mode_reason: None,
        priority: None,
        size: None,
        deps: None,
        stage: None,
        race: None,
        pair: None,
    }
}

fn add(id: &str, brief: &str) -> PlanEdit {
    let mut plan = parse_plan(&plan_with(PROFILE, &[with_brief(id, brief)])).unwrap();
    PlanEdit::add_task(plan.tasks.pop().unwrap())
}

const CHAR_CAP: &str = "task t1: brief: at most 1000000 characters";

/// 1,000,001 characters of `é` (2 bytes each, 2,000,002 bytes): the character cap
/// refuses it, which a byte count of 1,000,000 would also do, so one character under
/// (1,000,000 `é`, 2,000,000 bytes, over a byte count's 1,000,000) must be accepted.
#[test]
fn a_brief_over_the_cap_is_refused() {
    assert_eq!(BRIEF_MAX_CHARS, 1_000_000);
    let over = "é".repeat(BRIEF_MAX_CHARS + 1);
    let errors = texts(&errors_of(&plan_with(PROFILE, &[with_brief("t1", &over)])));
    assert!(errors.contains(&CHAR_CAP.to_string()), "{errors:?}");
    let at = "é".repeat(BRIEF_MAX_CHARS);
    run_ok(&plan_with(PROFILE, &[with_brief("t1", &at)]));
    for source in [EditSource::User, EditSource::Orchestrator] {
        let errors = edited(source, vec![amend(over.clone())]);
        assert!(errors.contains(&CHAR_CAP.to_string()), "{errors:?}");
    }
}

/// Eight ASCII briefs of 524,288 bytes are exactly `PLAN_BRIEFS_MAX_BYTES` and accepted;
/// one byte more, in a ninth task, is refused with the total.
#[test]
fn briefs_over_the_plan_cap_are_refused() {
    let brief = "b".repeat(524_288);
    assert_eq!(8 * brief.len(), PLAN_BRIEFS_MAX_BYTES);
    let mut tasks: Vec<String> = (1..=8)
        .map(|i| with_brief(&format!("t{i}"), &brief))
        .collect();
    run_ok(&plan_with(PROFILE, &tasks));
    tasks.push(with_brief("t9", "x"));
    let total = PLAN_BRIEFS_MAX_BYTES + 1;
    let refusal = format!("the plan's briefs total {total} bytes; at most 4194304");
    let errors = texts(&errors_of(&plan_with(PROFILE, &tasks)));
    assert!(errors.contains(&refusal), "{errors:?}");

    // Through a batch: t1 ("short", 5 bytes) plus seven 524,288-byte tasks, then an
    // eighth of 524,284 bytes: 5 + 8 × 524,288 - 4 = 4,194,305, one byte over.
    let mut edits: Vec<PlanEdit> = (2..=8).map(|i| add(&format!("t{i}"), &brief)).collect();
    edits.push(add("t9", &"b".repeat(524_284)));
    for source in [EditSource::User, EditSource::Orchestrator] {
        let errors = edited(source, edits.clone());
        assert!(errors.contains(&refusal), "{errors:?}");
    }
}
