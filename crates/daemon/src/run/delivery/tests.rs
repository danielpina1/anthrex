//! Task M9.2.6: the delivery model (decisions 8, 23–31), and a 9.1 `run.json` that
//! loads with a local delivery; and the fixture the other test files share (a
//! three-stage `pr` run). Pure: no host, no git. The PR title and body are tested in
//! `tests_body.rs`, quoting and the snapshot in `tests_snapshot.rs`.

use std::collections::BTreeSet;

use proto::{CiState, DeliveryMode, PrState, TaskOrigin, TaskState, TestMode, Verdict};

use super::ops::{HostOp, HostResult};
use super::snapshot::{delivery_info, stage_pr_info};
use super::*;
use crate::host::{
    Adopt, Author, CheckRun, CheckStatus, Conclusion, FetchOutcome, HostError, LogFile, PrRef,
    PrView, PushOutcome, ReplyTarget, Review, ReviewState,
};
use crate::run::engine::{OpKind, OpResult, fix_text};
use crate::run::model::{
    BisectRecord, CheckRecord, FixOf, ReviewRecord, Run, StageLayout, StageRecord, Task, TierRecord,
};
use crate::run::test_support::{plan_with, run_ok, task_toml};
use crate::run::tiers::Signal;

pub(super) const RUN: &str = "pr-body-1a2b";
pub(super) const HEAD2: &str = "c2c2c2c2c2c2c2c2c2c2c2c2c2c2c2c2c2c2c2c2";

pub(super) const PROFILE: &str = r#"
goal = "Test goal"

[profile]
modules = ["crates/*"]
hub = ["crates/proto/**"]
source = ["crates/*/src/**"]
check = "cargo test"
single_test = "cargo test -- --exact {test}"
test_passed = 'test {test} \.\.\. ok'
"#;

pub(super) const FOOTER: &str = "---\nOpened by anthrex for run pr-body-1a2b. anthrex pushes fixes to this branch when CI fails or a reviewer with write access comments, and replies on the threads it addressed. **anthrex never merges this pull request**, never approves it and never resolves a thread: that is yours. For a stack, merge with a merge commit; a squash or rebase merge also works, and anthrex then merges the new base into the next stage before retargeting it.\n";

pub(super) fn task_mut<'a>(run: &'a mut Run, id: &str) -> &'a mut Task {
    run.tasks
        .iter_mut()
        .find(|t| t.spec.id == id)
        .unwrap_or_else(|| panic!("no task {id}"))
}

pub(super) fn tier(tier: u8, flaky: &[&str], commit: &str, secs: u64) -> TierRecord {
    TierRecord {
        tier,
        affected: "crates/b".into(),
        steps: 1,
        cached: 0,
        ok: true,
        secs,
        flaky: flaky.iter().map(|s| s.to_string()).collect(),
        failing: Vec::new(),
        at: 2_000,
        commit: commit.into(),
    }
}

pub(super) fn check_with(record: TierRecord) -> CheckRecord {
    CheckRecord {
        at: 2_000,
        ok: true,
        code: Some(0),
        timed_out: false,
        tail: String::new(),
        secs: record.secs,
        on_candidate: record.tier == 2,
        summary: None,
        summary_source: None,
        tier: Some(record),
        lane: None,
    }
}

pub(super) fn review(task: &Task, round: u32, verdict: Verdict) -> ReviewRecord {
    ReviewRecord {
        round,
        route: task.route.clone(),
        base: "b".repeat(40),
        head: HEAD2.into(),
        verdict: Some(verdict),
        summary: String::new(),
        findings: Vec::new(),
        lane: None,
    }
}

pub(super) fn pr(number: u64, state: PrState) -> PrRecord {
    PrRecord {
        number,
        url: format!("https://github.com/fake/app/pull/{number}"),
        base: "main".into(),
        opened_at: 3_000,
        pushed_head: HEAD2.into(),
        state,
        merged_at: None,
        merge_commit: None,
        merge_method: None,
        next_poll_at: 0,
        unchanged_views: 0,
        last_view_at: None,
        watermark: Watermark::default(),
        checks: Vec::new(),
        retargeted_to: None,
        opened_base: None,
        branch_deleted: false,
        confirmed: None,
    }
}

/// A three-stage `pr` run whose stage 2 has two merged tasks: `t2`, a hub task that
/// reached rung 2, and `t3`, merged without approval with one test-weakening signal
/// and owning the protected `.claude/settings.json` by name. Stage 1's PR is #141.
pub(super) fn staged() -> Run {
    let plan = plan_with(
        PROFILE,
        &[
            task_toml("t1", "S", r#"["crates/a/**"]"#, ""),
            task_toml(
                "t2",
                "M",
                r#"["crates/proto/src/lib.rs"]"#,
                "stage = 2\ntest_to_write = \"proto::round_trips\"",
            ),
            task_toml(
                "t3",
                "S",
                r#"["crates/b/**", ".claude/settings.json"]"#,
                "stage = 2",
            ),
            task_toml("t4", "S", r#"["crates/c/**"]"#, "stage = 3"),
        ],
    );
    let mut run = run_ok(&plan);
    run.id = RUN.into();
    run.stage_layout = StageLayout::Multi;
    run.profile.tiers.full_shards = 2;
    let mut stage2 = StageRecord::new(
        2,
        format!("anthrex/{RUN}/stage-2"),
        &"a".repeat(40),
        BTreeSet::from(["t2".to_string(), "t3".to_string()]),
        1_000,
    );
    stage2.head = HEAD2.into();
    stage2.full.green_at = Some(HEAD2.into());
    stage2.full.last = Some(tier(3, &[], HEAD2, 184));
    stage2.full.runs = 3;
    run.stages = vec![
        StageRecord::new(
            1,
            format!("anthrex/{RUN}/stage-1"),
            "a",
            BTreeSet::new(),
            1_000,
        ),
        stage2,
    ];
    {
        let t2 = task_mut(&mut run, "t2");
        t2.state = TaskState::Merged;
        t2.hub = true;
        t2.size = proto::Size::M;
        t2.test_mode = TestMode::Tdd;
        t2.max_rung = 2;
        t2.reviews = vec![
            review(t2, 1, Verdict::Changes),
            review(t2, 2, Verdict::Approve),
        ];
        t2.checks = vec![
            check_with(tier(1, &[], "", 30)),
            check_with(tier(2, &["a::flaky"], "", 60)),
        ];
    }
    {
        let t3 = task_mut(&mut run, "t3");
        t3.state = TaskState::Merged;
        t3.hub = false;
        t3.size = proto::Size::S;
        t3.test_mode = TestMode::Check;
        t3.max_rung = 0;
        t3.merged_without_approval = Some("the reviewer timed out".into());
        t3.reviews = vec![review(t3, 1, Verdict::Changes)];
        t3.signals = vec![Signal::DeletedTestFile {
            path: "crates/b/tests/old.rs".into(),
        }];
        t3.checks = vec![check_with(tier(1, &[], "", 20))];
    }
    run.delivery.mode = DeliveryMode::Pr;
    run.delivery.stages = vec![
        StageDelivery {
            pr: Some(pr(141, PrState::Open)),
            ..StageDelivery::default()
        },
        StageDelivery::default(),
        StageDelivery::default(),
    ];
    run
}

/// The snapshot's `RunInfo` of `run` alone.
pub(super) fn snap(run: &Run) -> proto::RunInfo {
    let mut state = crate::run::engine::EngineState::default();
    state.runs.insert(run.id.clone(), run.clone());
    crate::run::snapshot::snapshot(&state, 10_000)
        .runs
        .remove(0)
}

pub(super) fn thread(key: &str, author: &str, state: ThreadState) -> ThreadRecord {
    ThreadRecord {
        key: key.into(),
        author: author.into(),
        path: Some("crates/b/src/lib.rs".into()),
        line: Some(12),
        diff_hunk: "@@ -1 +1 @@".into(),
        text: "a comment that must never reach the snapshot".into(),
        state,
        seen_at: 4_000,
        last_comment_id: 1,
        comments: vec![super::SeenComment {
            id: 1,
            author: author.into(),
            text: "a comment that must never reach the snapshot".into(),
        }],
        candidates: Vec::new(),
        counted: true,
        batch: 1,
        waiting_since: 3_900,
        replies: 1,
    }
}

/// A fix task of stage `stage`, cloned from `t3`.
pub(super) fn fix_task(
    run: &mut Run,
    id: &str,
    origin: TaskOrigin,
    fixes: FixOf,
    state: TaskState,
) {
    let mut t = run.task("t3").unwrap().clone();
    t.spec.id = id.into();
    t.spec.stage = 2;
    t.origin = origin;
    t.fixes = Some(fixes);
    t.state = state;
    run.tasks.push(t);
}

#[cfg(test)]
#[path = "tests_persist.rs"]
mod persist;
