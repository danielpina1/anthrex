//! Milestone 9.5 decision 13: the line thresholds in triage's and the size check's
//! prompts. With the defaults the prompts are byte-identical to before; tuned ones are
//! written in all three places.

use super::prompt::render;
use super::*;
use proto::{Size, SizeThresholds};

/// Triage's prompt head as milestone 9.3 sent it (the cap at its default 12).
const TRIAGE_SIZES: &str = "Size S: one file, no interface change, about 20 changed lines or fewer. Size M: 1 to 3 files inside one module, about 100 changed lines or fewer. Anything bigger is not single.";

/// The size check's head as milestone 9.3 sent it, byte for byte.
const SIZE_CHECK_TODAY: &str = "[anthrex decider] size_check v1
You check the size of planned coding tasks against what scouts found in the repository. Answer with one JSON object that matches the schema, and nothing else, with one entry per task id below.
Size S: one file, no interface change, a mechanical check exists, about 20 changed lines or fewer. Size M: 1 to 3 files inside one module, a clear spec, about 100 changed lines or fewer. Size L: files in more than one module plus an interface change, more than about 100 lines, an unclear spec, or a new dependency.
Judge each task from the evidence, not from its stated size. Give the size you believe and a one-sentence reason naming the evidence.

Tasks:
";

fn triage(thresholds: SizeThresholds) -> DeciderRequest {
    DeciderRequest::Triage(TriageInput {
        goal: "Fix the typo".into(),
        profile_summary: String::new(),
        report_summary: None,
        report_files: vec![],
        files: vec!["README.md".into()],
        files_total: 1,
        planner_task_cap: 12,
        thresholds,
    })
}

fn size_check(thresholds: SizeThresholds) -> DeciderRequest {
    DeciderRequest::SizeCheck(SizeCheckInput {
        tasks: vec![SizeCheckTask {
            id: "t1".into(),
            title: "Title".into(),
            brief: "Brief".into(),
            acceptance: vec!["it works".into()],
            owns: vec!["src/a.rs".into()],
            deps: vec![],
            size: Size::S,
            interface_change: false,
            hub: false,
        }],
        evidence_refs: vec![],
        evidence: vec![],
        modules: vec![],
        hub: vec![],
        thresholds,
    })
}

#[test]
fn default_thresholds_render_todays_exact_prompts() {
    let triage = render(&triage(SizeThresholds::default()));
    assert_eq!(triage.lines().nth(4), Some(TRIAGE_SIZES), "{triage}");
    let check = render(&size_check(SizeThresholds::default()));
    assert!(check.starts_with(SIZE_CHECK_TODAY), "{check}");
    // An input recorded before decision 13 reads the defaults.
    let old = serde_json::json!({
        "tasks": [], "evidence_refs": [], "evidence": [], "modules": [], "hub": []
    });
    let old: SizeCheckInput = serde_json::from_value(old).unwrap();
    assert_eq!(old.thresholds, SizeThresholds::default());
    let DeciderRequest::Triage(new) = triage_request() else {
        unreachable!()
    };
    let mut value = serde_json::to_value(&new).unwrap();
    value.as_object_mut().unwrap().remove("thresholds");
    let old: TriageInput = serde_json::from_value(value).unwrap();
    assert_eq!(old.thresholds, SizeThresholds::default());
}

fn triage_request() -> DeciderRequest {
    triage(SizeThresholds {
        s_lines: 35,
        m_lines: 140,
    })
}

#[test]
fn tuned_thresholds_render_their_numbers() {
    let tuned = SizeThresholds {
        s_lines: 35,
        m_lines: 140,
    };
    let triage = render(&triage(tuned));
    assert!(
        triage.contains("Size S: one file, no interface change, about 35 changed lines or fewer. Size M: 1 to 3 files inside one module, about 140 changed lines or fewer."),
        "{triage}"
    );
    let check = render(&size_check(tuned));
    assert!(
        check.contains("a mechanical check exists, about 35 changed lines or fewer. Size M: 1 to 3 files inside one module, a clear spec, about 140 changed lines or fewer. Size L: files in more than one module plus an interface change, more than about 140 lines, an unclear spec"),
        "{check}"
    );
    for prompt in [&triage, &check] {
        assert!(
            !prompt.contains("about 20 ") && !prompt.contains("about 100 "),
            "{prompt}"
        );
    }
}
