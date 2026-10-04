//! Milestone 8b decision 28: which sessions get the output-filter hook, and where its
//! log goes.

use super::*;
use crate::headless::SessionArg;
use crate::headless::argv::{CLI_CAPS, claude_args, codex_args};
use crate::launch::codex::toml_string;
use crate::output_filter::{FilterHook, LOG_DIR_NAME, codex_filter_note};
use crate::run::contract_patterns::TEST_WRITER_CONTRACT;
use crate::run::role_launch_patterns::{racer_spec, test_writer_spec};
use crate::run::test_support::{PROFILE, plan_with, race_of, run_ok, task_toml};
use proto::{Effort, LaneState, OutputFilter, ProfileSource, Strength};

fn stored_run() -> Run {
    let mut run = run_ok(&plan_with(
        PROFILE,
        &[task_toml("t1", "S", "[\"crates/a/**\"]", "")],
    ));
    run.profile_source = Some(ProfileSource::Stored);
    run.output_filter = OutputFilter::FailuresOnly;
    run.filter_prefixes = vec!["cargo test".into(), "cargo build".into()];
    run
}

fn route(runtime: Runtime) -> Route {
    Route {
        runtime,
        model: "m".into(),
        strength: Strength::Standard,
        effort: Effort::Medium,
    }
}

#[test]
fn worker_spec_sets_the_hook_only_for_a_stored_profile_with_prefixes() {
    let run = stored_run();
    let mut task = run.tasks[0].clone();
    task.route = route(Runtime::Claude);
    assert_eq!(
        worker_spec(&run, &task).output_filter,
        Some(FilterHook {
            mode: OutputFilter::FailuresOnly,
            prefixes: vec!["cargo test".into(), "cargo build".into()],
            log_dir: task_tmp_dir(&run.data_dir, "t1").join(LOG_DIR_NAME),
        })
    );
    let mut tail = run.clone();
    tail.output_filter = OutputFilter::Tail;
    let hook = worker_spec(&tail, &task)
        .output_filter
        .expect("tail filters");
    assert_eq!(hook.mode, OutputFilter::Tail);

    let without = |change: &dyn Fn(&mut Run)| {
        let mut run = run.clone();
        change(&mut run);
        worker_spec(&run, &task).output_filter
    };
    assert_eq!(without(&|r| r.profile_source = None), None);
    assert_eq!(
        without(&|r| r.profile_source = Some(ProfileSource::Plan)),
        None
    );
    assert_eq!(
        without(&|r| r.profile_source = Some(ProfileSource::None)),
        None
    );
    assert_eq!(without(&|r| r.output_filter = OutputFilter::None), None);
    assert_eq!(without(&|r| r.filter_prefixes.clear()), None);

    // Milestone 9.5 decision 28: a Codex worker gets it under the same conditions;
    // every reviewer never does.
    let mut codex = task.clone();
    codex.route = route(Runtime::Codex);
    assert_eq!(
        worker_spec(&run, &codex).output_filter,
        worker_spec(&run, &task).output_filter
    );
    for runtime in [Runtime::Claude, Runtime::Codex] {
        assert_eq!(
            reviewer_spec(&run, &task, &route(runtime)).output_filter,
            None
        );
    }
}

#[test]
fn the_filter_hook_adds_no_writable_root() {
    let run = stored_run();
    let mut task = run.tasks[0].clone();
    task.route = route(Runtime::Claude);
    let spec = worker_spec(&run, &task);
    let hook = spec.output_filter.as_ref().expect("filtered");
    let sandbox = spec.claude_sandbox.as_ref().expect("sandboxed");
    assert_eq!(
        sandbox.writable_roots,
        worker_git_roots(&run.data_dir, "t1")
    );
    let mut plain = run.clone();
    plain.profile_source = None;
    assert_eq!(
        worker_spec(&plain, &task).claude_sandbox.as_ref(),
        Some(sandbox)
    );
    // The log goes under the task's `TMPDIR`, which is already in the grant, and out of
    // the checkout.
    let tmpdir = spec
        .env
        .iter()
        .find(|(key, _)| key == "TMPDIR")
        .map(|(_, value)| PathBuf::from(value))
        .expect("TMPDIR");
    assert_eq!(hook.log_dir, tmpdir.join(LOG_DIR_NAME));
    assert!(sandbox.writable_roots.contains(&tmpdir));
    assert!(!hook.log_dir.starts_with(&task.worktree));
}

const EXE: &str = "/x/anthrex";

/// The `developer_instructions=` value of `spec`'s first Codex turn.
fn codex_instructions(spec: &HeadlessSpec) -> String {
    let session = SessionArg::New { uuid: None };
    let argv = codex_args(
        spec,
        &session,
        "go",
        Path::new(EXE),
        4,
        Path::new("/s/d.sock"),
        &CLI_CAPS,
    );
    let all: Vec<&str> = argv
        .iter()
        .filter_map(|a| a.strip_prefix("developer_instructions="))
        .collect();
    assert_eq!(all.len(), 1, "{argv:?}");
    all[0].to_string()
}

/// `spec`'s `TMPDIR`.
fn tmpdir(spec: &HeadlessSpec) -> PathBuf {
    spec.env
        .iter()
        .find(|(key, _)| key == "TMPDIR")
        .map(|(_, value)| PathBuf::from(value))
        .expect("TMPDIR")
}

/// The note `spec` must carry after `contract`: its log directory is its own
/// `TMPDIR`'s `anthrex-logs`.
fn noted(run: &Run, spec: &HeadlessSpec, contract: &str) -> String {
    let hook = FilterHook {
        mode: run.output_filter,
        prefixes: run.filter_prefixes.clone(),
        log_dir: tmpdir(spec).join(LOG_DIR_NAME),
    };
    let note = codex_filter_note(Path::new(EXE), &hook);
    toml_string(&format!("{contract}\n\n{note}"))
}

/// Milestone 9.5 decision 28 (`CodexFilter::Instruction`): a Codex worker, test writer
/// and racer get `codex_filter_note` after their contract, its log directory their own
/// checkout's `TMPDIR`'s `anthrex-logs` (lane b's for lane b's racer). A Claude worker
/// gets the hook, never the note.
#[test]
fn a_codex_worker_gets_the_instruction_note() {
    let mut run = stored_run();
    run.tasks[0].route = route(Runtime::Codex);
    let task = run.tasks[0].clone();

    let worker = worker_spec(&run, &task);
    assert_eq!(tmpdir(&worker), task_tmp_dir(&run.data_dir, "t1"));
    assert_eq!(
        codex_instructions(&worker),
        noted(&run, &worker, WORKER_CONTRACT)
    );

    let writer = test_writer_spec(&run, &task, &route(Runtime::Codex));
    assert_eq!(
        codex_instructions(&writer),
        noted(&run, &writer, TEST_WRITER_CONTRACT)
    );

    let mut claude_task = task.clone();
    claude_task.route = route(Runtime::Claude);
    let race = race_of(&claude_task, [LaneState::Working, LaneState::Working]);
    claude_task.race = Some(race.clone());
    let b = racer_spec(&run, &claude_task, &race.lanes[1]);
    assert_eq!(b.runtime, Runtime::Codex);
    assert_eq!(tmpdir(&b), task_tmp_dir(&run.data_dir, "t1.b"));
    assert_eq!(codex_instructions(&b), noted(&run, &b, WORKER_CONTRACT));
    assert_eq!(
        b.output_filter.as_ref().map(|hook| hook.log_dir.clone()),
        Some(task_tmp_dir(&run.data_dir, "t1.b").join(LOG_DIR_NAME))
    );

    // Lane a runs on Claude: the hook in its settings, no note in its prompt.
    let a = racer_spec(&run, &claude_task, &race.lanes[0]);
    let argv = claude_args(
        &a,
        &SessionArg::New { uuid: None },
        Path::new(EXE),
        4,
        Path::new("/s/d.sock"),
        &CLI_CAPS,
    );
    let at = argv
        .iter()
        .position(|x| x == "--append-system-prompt")
        .unwrap();
    assert_eq!(argv[at + 1], WORKER_CONTRACT);
    assert!(argv.iter().any(|x| x.contains("filter-hook")), "{argv:?}");
}

/// Pinning (passes before task M9.5.19): a Codex worker's filter log directory is
/// already inside one of its writable roots, the task's temporary directory, so
/// nothing is added to them; for a racer, its lane's.
#[test]
fn the_log_dir_is_already_a_codex_writable_root() {
    let mut run = stored_run();
    run.tasks[0].route = route(Runtime::Codex);
    let task = run.tasks[0].clone();
    let worker = worker_spec(&run, &task);
    let log_dir = task_tmp_dir(&run.data_dir, "t1").join(LOG_DIR_NAME);
    assert_eq!(
        worker.codex_writable_roots,
        worker_git_roots(&run.data_dir, "t1")
    );
    assert!(worker.codex_writable_roots.contains(&tmpdir(&worker)));
    assert!(
        worker
            .codex_writable_roots
            .iter()
            .any(|root| log_dir.starts_with(root)),
        "{:?}",
        worker.codex_writable_roots
    );
    let mut plain = run.clone();
    plain.profile_source = None;
    assert_eq!(
        worker_spec(&plain, &task).codex_writable_roots,
        worker.codex_writable_roots
    );

    let mut task = task;
    let race = race_of(&task, [LaneState::Working, LaneState::Working]);
    task.race = Some(race.clone());
    let b = racer_spec(&run, &task, &race.lanes[1]);
    let lane_log = task_tmp_dir(&run.data_dir, "t1.b").join(LOG_DIR_NAME);
    assert_eq!(
        b.codex_writable_roots,
        worker_git_roots(&run.data_dir, "t1.b")
    );
    assert!(
        b.codex_writable_roots
            .iter()
            .any(|r| lane_log.starts_with(r))
    );
}

/// Decision 28: reviewers and scouts are never worker-like, so neither runtime gives
/// them the hook or the note.
#[test]
fn reviewers_and_scouts_never_get_it() {
    let run = stored_run();
    let task = run.tasks[0].clone();
    for runtime in [Runtime::Claude, Runtime::Codex] {
        let spec = reviewer_spec(&run, &task, &route(runtime));
        assert_eq!(spec.output_filter, None);
        if runtime == Runtime::Codex {
            assert_eq!(
                codex_instructions(&spec),
                toml_string(super::super::contract::REVIEWER_CONTRACT)
            );
        }
    }
    let scout = crate::scout::spec::ScoutSpec {
        id: "api-1".into(),
        kind: proto::ScoutKind::Area,
        run_id: Some(run.id.clone()),
        question: "How is it built?".into(),
        first_turn: "go".into(),
        cwd: "/wt/runs/r1/integration".into(),
        project: "/repo".into(),
        web: false,
        codex_config: Vec::new(),
        base_sha: "ab".repeat(20),
        repo_paths: Vec::new(),
    };
    for runtime in [Runtime::Claude, Runtime::Codex] {
        let ctx = crate::scout::spec::ScoutContext {
            roster: config::default_roster().into(),
            default_runtime: runtime,
            scouts: config::Scouts::default(),
            claude: config::ClaudeHeadless::default(),
            caps: CLI_CAPS,
            data_dir: PathBuf::from("/data"),
        };
        let spec = crate::scout::spec::headless_spec(&scout, &ctx);
        assert_eq!(spec.runtime, runtime);
        assert_eq!(spec.output_filter, None);
        if runtime == Runtime::Codex {
            assert_eq!(codex_instructions(&spec), toml_string(&spec.instructions));
            assert!(!codex_instructions(&spec).contains("filter-run"));
        }
    }
}
