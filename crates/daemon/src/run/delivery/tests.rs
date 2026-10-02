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

/// A run.json written by milestone 9.1's code (the same model as 9.0.6's, which added
/// nothing to it) loads with a local delivery, and writes back what it stored plus
/// the default `delivery`.
#[test]
fn old_run_json_loads_with_local_delivery() {
    let text = include_str!("m9_1_run.json");
    let run: Run = serde_json::from_str(text).expect("a 9.1 run.json loads");
    assert_eq!(run.delivery, RunDelivery::default());
    assert_eq!(run.delivery.mode, DeliveryMode::Local);
    assert_eq!(run.delivery.repo, None);
    // Fix round 1: 9.1 counted no tier-3 runs; its stage loads with 0.
    assert!(run.stages.iter().all(|s| s.full.runs == 0));
    assert!(run.stages.iter().any(|s| s.full.last.is_some()));
    assert!(run.delivery.stages.is_empty());
    assert_eq!(delivery_info(&run), None);
    assert_eq!(stage_pr_info(&run, 1), None);
    // 9.1's fix task still reads as 9.1's.
    let fix = run.task("fix1").expect("the bisect's fix task");
    assert_eq!(fix_text(&run, fix.fixes.as_ref().unwrap()), "bisect of t4");
    let mut back = serde_json::to_value(&run).unwrap();
    let delivery = back.as_object_mut().unwrap().remove("delivery").unwrap();
    assert_eq!(
        serde_json::from_value::<RunDelivery>(delivery).unwrap(),
        RunDelivery::default()
    );
    // Fix round 1's tier-3 run counter is the one other key written back, 0.
    for stage in back["stages"].as_array_mut().unwrap() {
        let full = stage["full"].as_object_mut().unwrap();
        assert_eq!(full.remove("runs"), Some(serde_json::json!(0)));
    }
    let stored: serde_json::Value = serde_json::from_str(text).unwrap();
    assert_eq!(back, stored);
    // A BisectRecord without `ci` (9.1's) is not a CI bisect.
    let record = serde_json::json!({
        "head": "h", "tests": [], "base": "b", "candidates": [], "lo": 0, "hi": 0
    });
    let bisect: BisectRecord = serde_json::from_value(record).unwrap();
    assert_eq!(bisect.ci, None);
}

/// Every field of the model survives `run.json`, and its enums are spelled in snake case.
#[test]
fn the_delivery_model_round_trips_through_run_json() {
    let mut run = staged();
    run.delivery.repo = Some(crate::host::HostRepo {
        host: "github.com".into(),
        owner: "fake".into(),
        name: "app".into(),
        remote: "origin".into(),
        root: "/tmp/x".into(),
    });
    run.delivery.watching = true;
    run.delivery.poll_base_secs = 120;
    run.delivery
        .permissions
        .insert("alice".into(), crate::host::RepoPermission::Maintain);
    run.delivery.base_synced = Some("b".repeat(40));
    run.delivery.base_sync_due.insert(2, "d".repeat(40));
    run.delivery.failures.insert("2/view_pr".into(), 3);
    let line = "PR #142: view_pr keeps failing: boom";
    run.delivery.alerts.insert("2/view_pr".into(), line.into());
    let mut record = pr(142, PrState::Merged);
    record.merged_at = Some(9_000);
    record.merge_commit = Some("e".repeat(40));
    record.merge_method = Some(proto::MergeMethod::SquashOrRebase);
    record.watermark.ci.insert(
        HEAD2.into(),
        CiSeen {
            failing: vec!["test".into()],
            at: 4_000,
            jobs: vec!["28000000001/28000000002".into()],
        },
    );
    record.watermark.mergeable = Some(crate::host::Mergeable::Conflicting);
    record
        .watermark
        .reviews_seen
        .extend([5_000_000_010, 5_000_000_011]);
    record.opened_base = Some(format!("anthrex/{RUN}/stage-1"));
    record.checks = vec![CheckSeen {
        name: "test".into(),
        state: CiState::Red,
        ci_run: Some(28_000_000_001),
    }];
    run.delivery.stages[1] = StageDelivery {
        pr: Some(record),
        skipped: false,
        ready_at: Some(2_500),
        paused_by: Some(1),
        ci: vec![CiRecord {
            head: HEAD2.into(),
            ci_runs: vec![28_000_000_001],
            phase: CiPhase::ToUser,
            log: Some("/tmp/data/delivery/ci-28000000001.log".into()),
            category: Some(proto::CiCategory::Infra),
            failing_tests: vec!["a::b".into()],
            lines: vec!["boom".into()],
            reruns: vec![28_000_000_001],
            key: "a::b".into(),
            fix_task: Some("fix2".into()),
            checks: vec!["test".into()],
            jobs: vec!["28000000001/28000000002".into()],
            external: vec!["ci/ext: FAILURE (https://ci.example/1)".into()],
            infra_only: true,
            fetched: vec![28_000_000_001],
            text: "--- FAIL: a::b".into(),
            decider: Some(4),
            source: Some("decider".into()),
            reruns_answered: vec![28_000_000_001],
            rerun_timeouts: vec![28_000_000_001],
            log_failures: 2,
            probe: Some(31),
            probe_failures: 1,
            retry_at: 5_000,
            command: Some("cargo test -- --exact 'a::b'".into()),
            newest: None,
        }],
        threads: vec![thread(
            "t98765",
            "alice",
            ThreadState::Ignored {
                reason: "a bot".into(),
            },
        )],
        batch: Some(Batch {
            started_at: 1,
            last_at: 2,
            threads: vec!["142:t98765".into()],
        }),
        review_rounds: 2,
        review_wait_secs: 77,
        history_written: true,
        pushed: Some(HEAD2.into()),
        retry_at: Some(3_000),
        remote_head: Some(HEAD2.into()),
        held: Some("the remote refused the push".into()),
        replies: vec![super::ReplyDue {
            thread: "t98765".into(),
            target: ReplyTarget::Thread { comment_id: 98_765 },
            body: "Addressed in 1a2b3c4 by task fix2.".into(),
            marker: format!("<!-- anthrex:reply {RUN} 142:t98765 1a2b3c4 -->"),
            task: Some("fix2".into()),
            push: Some(HEAD2.into()),
            push_done: true,
            ready: false,
            sent: true,
            failures: 2,
        }],
        auto_replies: ["fix2/t98765".to_string()].into(),
        own_comments: [5_000_000_099].into(),
        batches: 3,
        round_batch: 2,
        landed: Some(PrState::Merged),
        sync_red: Some((HEAD2.into(), HEAD2.into())),
        wait_from: Some(4_000),
        reopens: 1,
        local_moved: None,
    };
    run.delivery.permission_retry_at = Some(6_000);
    run.delivery.base_fetch_due = true;
    run.delivery.base_fetch_retry_at = Some(7_000);
    let json = serde_json::to_value(&run.delivery).unwrap();
    assert_eq!(json["stages"][1]["ci"][0]["phase"], "to_user");
    assert_eq!(
        json["stages"][1]["threads"][0]["state"],
        serde_json::json!({"ignored": {"reason": "a bot"}})
    );
    assert_eq!(json["stages"][1]["pr"]["merge_method"], "squash_or_rebase");
    assert_eq!(
        json["stages"][1]["pr"]["watermark"]["mergeable"],
        "conflicting"
    );
    assert_eq!(json["permissions"]["alice"], "maintain");
    let back: RunDelivery = serde_json::from_value(json).unwrap();
    assert_eq!(back, run.delivery);
    let reloaded: Run = serde_json::from_str(&serde_json::to_string(&run).unwrap()).unwrap();
    assert_eq!(reloaded.delivery, run.delivery);
}

/// Decision 8: every host op and result is journaled as an op is.
#[test]
fn host_ops_and_results_round_trip_through_the_journal() {
    let repo = crate::host::HostRepo {
        host: "github.com".into(),
        owner: "fake".into(),
        name: "app".into(),
        remote: "origin".into(),
        root: "/tmp/x".into(),
    };
    let ops = vec![
        HostOp::Push {
            stage: 1,
            sha: HEAD2.into(),
        },
        HostOp::Fetch {
            stage: None,
            branch: "main".into(),
            into: format!("refs/anthrex/{RUN}/remote/base"),
            adopt: Some(Adopt {
                local_ref: format!("anthrex/{RUN}/stage-1"),
                expected_local: HEAD2.into(),
                also_integration: true,
            }),
            parents_of: Some(HEAD2.into()),
        },
        HostOp::OpenPr {
            stage: 1,
            base: "main".into(),
            head: format!("anthrex/{RUN}/stage-1"),
            title: "t".into(),
            body: "b".into(),
        },
        HostOp::ViewPr {
            stage: 1,
            number: 142,
        },
        HostOp::FailedLogs {
            stage: 1,
            ci_run: 7,
            max_bytes: 4_096,
        },
        HostOp::RerunFailed {
            stage: 1,
            ci_run: 7,
        },
        HostOp::Reply {
            stage: 1,
            number: 142,
            thread: "142:t9".into(),
            target: ReplyTarget::Thread { comment_id: 9 },
            body: "Addressed".into(),
            marker: format!("<!-- anthrex:reply {RUN} 142:t9 1a2b3c4 -->"),
        },
        HostOp::Retarget {
            stage: 2,
            number: 143,
            base: "main".into(),
        },
        HostOp::Permission {
            user: "alice".into(),
        },
        HostOp::DeleteBranch { stage: 1 },
    ];
    for op in ops {
        let kind = OpKind::Host {
            repo: repo.clone(),
            op,
        };
        assert_eq!(kind.name(), "Host");
        let text = serde_json::to_string(&kind).unwrap();
        assert_eq!(serde_json::from_str::<OpKind>(&text).unwrap(), kind);
    }
    let view = PrView {
        number: 142,
        state: PrState::Open,
        merged_at: None,
        merge_commit: None,
        base_ref: "main".into(),
        head_oid: HEAD2.into(),
        mergeable: crate::host::Mergeable::Mergeable,
        review_decision: None,
        checks: vec![CheckRun {
            name: "test".into(),
            status: CheckStatus::Completed,
            conclusion: Some(Conclusion::Failure),
            ci_run: Some(7),
            url: "https://github.com/fake/app/actions/runs/7/job/8".into(),
        }],
        reviews: vec![Review {
            id: 5_000_000_001,
            author: Author {
                login: "alice".into(),
                bot: false,
            },
            state: ReviewState::ChangesRequested,
            body: "please".into(),
        }],
        comments: Vec::new(),
        threads: Vec::new(),
    };
    let results = vec![
        HostResult::Pushed(PushOutcome::Rejected {
            reason: "non-fast-forward".into(),
        }),
        HostResult::Fetched(FetchOutcome::NotDescendant {
            remote: HEAD2.into(),
        }),
        HostResult::PrOpened(PrRef {
            number: 142,
            url: "u".into(),
            state: PrState::Open,
            existed: true,
        }),
        HostResult::PrViewed(Box::new(view)),
        HostResult::Logs(LogFile {
            path: "/tmp/l".into(),
            bytes: 9,
            truncated: true,
            tail: "--- FAIL: t".into(),
        }),
        HostResult::Rerun,
        HostResult::Replied { comment_id: 11 },
        HostResult::Retargeted,
        HostResult::Permission {
            user: "alice".into(),
            permission: crate::host::RepoPermission::Write,
        },
        HostResult::Deleted,
        HostResult::Error(HostError::RateLimited("API rate limit exceeded".into())),
    ];
    for result in results {
        let result = OpResult::Host(result);
        let text = serde_json::to_string(&result).unwrap();
        assert_eq!(serde_json::from_str::<OpResult>(&text).unwrap(), result);
    }
}

/// `run.json`'s spelling of `[delivery] sync` is the config's: `on_conflict` and
/// `always` (M9.2.3's review fix 2; moved here from `mod.rs` by task M9.2.6).
#[test]
fn sync_policy_is_persisted_in_snake_case() {
    for (sync, text) in [
        (SyncPolicy::OnConflict, "on_conflict"),
        (SyncPolicy::Always, "always"),
    ] {
        let limits = DeliveryLimits {
            sync,
            ..DeliveryLimits::default()
        };
        let json = serde_json::to_value(&limits).unwrap();
        assert_eq!(json["sync"], serde_json::json!(text));
        let back: DeliveryLimits =
            serde_json::from_value(serde_json::json!({ "sync": text })).unwrap();
        assert_eq!(back.sync, sync);
    }
}
