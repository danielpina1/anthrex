use super::*;

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
    // Milestone 9.3 decision 18: round 1, on each stage and each task.
    for key in ["stages", "tasks"] {
        for node in back[key].as_array_mut().unwrap() {
            let round = node.as_object_mut().unwrap().remove("round");
            assert_eq!(round, Some(serde_json::json!(1)), "{key}");
        }
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
        review_paused_secs: 33,
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
        maybe_sent: vec!["<!-- anthrex:reply r 7:c5 1a2b3c4 -->".into()],
        pushes: vec![HEAD2.into()],
        undecided: Some((HEAD2.into(), "f".repeat(40))),
        undecided_fails: 2,
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
            contains: None,
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
            base: None,
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

/// Removes every `key` from `value`, at any depth: the form an older binary wrote.
fn strip(value: &mut serde_json::Value, key: &str) {
    match value {
        serde_json::Value::Object(map) => {
            map.remove(key);
            map.values_mut().for_each(|v| strip(v, key));
        }
        serde_json::Value::Array(items) => items.iter_mut().for_each(|v| strip(v, key)),
        _ => {}
    }
}

/// Milestone 9.7 (DH §1.2, ruling R1a): an undecided stage, its failed checks, the base
/// fetch's question and its answer round-trip through `run.json` and the journal; a
/// `run.json` or journal line written before them loads with none.
#[test]
fn an_undecided_delivery_round_trips_and_older_forms_load() {
    let stage = StageDelivery {
        undecided: Some((HEAD2.into(), "f".repeat(40))),
        undecided_fails: 2,
        ..StageDelivery::default()
    };
    let mut json = serde_json::to_value(&stage).unwrap();
    assert_eq!(
        json["undecided"],
        serde_json::json!([HEAD2, "f".repeat(40)])
    );
    assert_eq!(json["undecided_fails"], 2);
    assert_eq!(
        serde_json::from_value::<StageDelivery>(json.clone()).unwrap(),
        stage
    );
    strip(&mut json, "undecided");
    strip(&mut json, "undecided_fails");
    assert_eq!(
        serde_json::from_value::<StageDelivery>(json).unwrap(),
        StageDelivery::default()
    );
    let asked = |contains| HostOp::Fetch {
        stage: None,
        branch: "main".into(),
        into: format!("refs/anthrex/{RUN}/remote/base"),
        adopt: None,
        parents_of: None,
        contains,
    };
    let question = crate::host::Contains {
        stage: 1,
        branch: format!("anthrex/{RUN}/stage-1"),
        into: format!("refs/anthrex/{RUN}/remote/stage-1"),
        head: HEAD2.into(),
        merged: "f".repeat(40),
        pr: Some(7),
    };
    let answered = |contains| {
        OpResult::Host(HostResult::Fetched(FetchOutcome::Fetched {
            sha: "b".repeat(40),
            parents: Some(1),
            contains,
        }))
    };
    let op = asked(Some(question));
    let text = serde_json::to_string(&op).unwrap();
    assert_eq!(serde_json::from_str::<HostOp>(&text).unwrap(), op);
    let result = answered(Some(false));
    let text = serde_json::to_string(&result).unwrap();
    assert_eq!(serde_json::from_str::<OpResult>(&text).unwrap(), result);
    let mut old = serde_json::to_value(asked(None)).unwrap();
    strip(&mut old, "contains");
    assert_eq!(serde_json::from_value::<HostOp>(old).unwrap(), asked(None));
    let mut old = serde_json::to_value(answered(None)).unwrap();
    strip(&mut old, "contains");
    assert_eq!(
        serde_json::from_value::<OpResult>(old).unwrap(),
        answered(None)
    );
}
