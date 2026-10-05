//! A design run in `pr` mode (decisions 23 and 31, DF §8.3) on `FakeHost`: the
//! documents commit is the bottom stage's first commit, each stage PR's body says which
//! requirements it covers, and 9.2's empty stage is still skipped when it does not hold
//! the documents (ruling T15-15). anthrex never merges: the rig's drop asserts nothing
//! asked the fake GitHub to merge, approve or enable auto-merge.

use daemon::host::Conclusion;
use daemon::host::fake::CiRule;
use proto::{DocGateKind, DocKind, PlanEdit, RunReply, RunRequest, TaskState};
use serde_json::json;

use crate::common::*;
use crate::support::orch_script::*;
use crate::support::run_design::*;
use crate::support::run_harness::RunHarness;
use crate::support::run_orch::ORCH_WAIT;
use crate::support::run_pr::{DELIVERY_TOML, PR_OPEN_WAIT, PrRig, log_lines, pr_goal};

/// A design harness (`RunHarness::design`) with the brief's `[delivery]` test
/// configuration, its daemon restarted on the rig's fake GitHub, as `orch_pr_harness`
/// does for a planned run.
fn design_pr_harness() -> (RunHarness, PrRig) {
    let mut h = harness("");
    let config = h.dir.path().join("config.toml");
    let text = std::fs::read_to_string(&config).unwrap();
    std::fs::write(&config, format!("{text}\n{DELIVERY_TOML}\n")).unwrap();
    let rig = PrRig::new(&h);
    let env = rig.env();
    let extra: Vec<(&str, &str)> = env.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    h.restart_daemon(&extra);
    (h, rig)
}

#[test]
fn e2e_pr_mode_docs_on_the_bottom_stage_and_covers_in_the_body() {
    let (h, rig) = design_pr_harness();
    rig.ctl()
        .set_ci(vec![CiRule::new("build", Conclusion::Success)]);
    drafts(&h);
    h.reviewer("spec", 1, 1, Findings::Submits(vec![]));
    h.reviewer("plan", 1, 1, Findings::Submits(vec![]));
    green(&h, "t1", PLAN_FILE);
    green(&h, "t3", "b.txt");
    // Three stages: t1 covers R1, t3 covers R2, and t2 in stage 2, which the user
    // cancels at the plan gate, leaves that stage empty.
    let plan = vec![
        covered("t1", PLAN_FILE, &["R1"], json!({"stage": 1})),
        covered("t2", "c.txt", &[], json!({"stage": 2})),
        covered("t3", "b.txt", &["R2"], json!({"stage": 3})),
    ];
    let mut steps = ask(None);
    steps.extend(merge(&labels(), &report(LABELS)));
    steps.push(approved("brainstorm"));
    steps.extend(spec_for_review(SPEC, 1));
    steps.push(spec_ready(SPEC, &[]));
    steps.push(approved("spec"));
    steps.extend(plan_for_review(plan));
    steps.push(plan_ready(&[]));
    steps.push(approved("plan"));
    steps.extend([marker(), read(None)]);
    h.script(ORCH, &steps);
    let run = pr_goal(&h, GOAL);

    approve(&h, &run, DocGateKind::Brainstorm, 1);
    approve(&h, &run, DocGateKind::Spec, 1);
    h.wait_doc_gate(&run, DocGateKind::Plan, 1, ORCH_WAIT);
    // The user's edit at the open plan gate is the plan's v2 (ruling T7-7).
    let cancel = PlanEdit::CancelTask {
        task_id: "t2".into(),
    };
    let edit = RunRequest::Edit {
        run_id: run.clone(),
        edits: vec![cancel],
        submit: false,
    };
    match h.request(edit) {
        RunReply::Done { .. } => {}
        other => panic!("the edit was refused: {other:?}"),
    }
    let info = approve(&h, &run, DocGateKind::Plan, 2);
    let plans: Vec<u32> = (info.docs.iter())
        .filter(|d| d.kind == DocKind::Plan)
        .map(|d| d.version)
        .collect();
    assert_eq!(plans, [1, 2]);
    wait_passed(&h, 1);

    // Stage 1's PR: its first commit past the base is the documents commit, and its
    // body names the requirements its task covers and the committed spec.
    let spec_path = doc_path(&h, &run, "specs", "");
    let wait = PR_OPEN_WAIT.saturating_add(DOCS_COMMIT_WAIT);
    rig.wait_stage_within(&h, &run, 1, "/state", &json!("open"), wait);
    let one = rig.wait_pr(1, |_| true);
    let base = h.run(&run).unwrap().base_sha;
    let range = format!("{base}..{}", one.head_oid);
    let firsts = rig.bare_git(&["rev-list", "--first-parent", "--reverse", &range]);
    let docs = firsts
        .lines()
        .next()
        .expect("a commit past the base")
        .to_string();
    let subject = rig.bare_git(&["log", "-1", "--format=%s", &docs]);
    assert_eq!(subject, format!("docs: spec and plan for {GOAL}"));
    let shown = rig.bare_git(&["show", &format!("{}:{spec_path}", one.head_oid)]);
    assert_eq!(shown, SPEC.trim_end());
    let covers = |ids: &str| format!("Covers {ids} (spec: {spec_path})\n");
    assert!(one.body.contains(&covers("R1")), "{}", one.body);
    assert!(!one.body.contains("R2"), "{}", one.body);

    // Stage 2 merged nothing and holds no documents: skipped, with no PR. Stage 3's PR
    // stacks on stage 1's and covers R2.
    rig.wait_stage(&h, &run, 3, "/state", &json!("open"));
    let three = rig.wait_pr(2, |_| true);
    assert_eq!(three.base, format!("anthrex/{run}/stage-1"));
    assert_eq!(three.head, format!("anthrex/{run}/stage-3"));
    assert!(three.body.contains(&covers("R2")), "{}", three.body);
    assert_eq!(rig.wait_prs(2).len(), 2);
    let log = log_lines(&h, &run);
    assert!(
        log.iter().any(|l| l == "stage 2: skipped (no changes)"),
        "{log:#?}"
    );
    let info = h.run(&run).unwrap();
    let states: Vec<_> = info
        .tasks
        .iter()
        .map(|t| (t.id.as_str(), t.state))
        .collect();
    assert_eq!(
        states,
        [
            ("t1", TaskState::Merged),
            ("t2", TaskState::Cancelled),
            ("t3", TaskState::Merged)
        ]
    );
}
