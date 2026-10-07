//! M9.17 fix round 2, item 3: decision 17's `installed` reads `PATH` as a shell does, a
//! leading `~` expanded to `HOME`.

use std::ffi::OsStr;

use super::executable_in;

#[test]
fn a_tilde_entry_on_path_is_expanded_to_home() {
    let home = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(home.path().join("bin")).unwrap();
    for file in ["bin/agent", "top"] {
        testexec::write_executable(home.path().join(file), "#!/bin/sh\n");
    }
    let h = Some(home.path().as_os_str());
    let path = |p: &'static str| Some(OsStr::new(p));
    assert!(executable_in("agent", path("/nonexistent:~/bin"), h));
    assert!(executable_in("top", path("~"), h));
    assert!(!executable_in("agent", path("~"), h));
    // No HOME: the entry names nothing.
    assert!(!executable_in("agent", path("~/bin"), None));
    // `~user` is not the user's home; an absolute entry is read as it is.
    assert!(!executable_in("agent", path("~nobody/bin"), h));
    let absolute = home.path().join("bin");
    assert!(executable_in("agent", Some(absolute.as_os_str()), None));
    // A binary with a `/` never reads PATH.
    assert!(!executable_in("bin/agent", path("~"), h));
    assert!(!executable_in("agent", None, h));
}

/// M9.17 fix round 3, item 3: an empty `HOME` is no home; `~/bin` is not read as the
/// relative `bin`.
#[test]
fn an_empty_home_expands_nothing() {
    use std::path::{Path, PathBuf};
    let entry =
        |dir: &str, home: Option<&str>| super::path_entry(PathBuf::from(dir), home.map(OsStr::new));
    assert_eq!(entry("~/bin", Some("")), None);
    assert_eq!(entry("~", Some("")), None);
    assert_eq!(entry("~/bin", None), None);
    assert_eq!(
        entry("~/bin", Some("/h")),
        Some(Path::new("/h/bin").to_path_buf())
    );
    assert_eq!(entry("/usr/bin", Some("")), Some(PathBuf::from("/usr/bin")));
    assert_eq!(entry("~x/bin", Some("/h")), Some(PathBuf::from("~x/bin")));
}

/// M9.17 fix round 3, item 3: the check matches what launches the binary. Only the
/// orchestrator's window goes through a shell (`/bin/sh -c 'exec "$0" "$@"'`), and only
/// macOS's `/bin/sh` (bash) expands `~` in `PATH`; a headless session (a sub-planner, a
/// scout) is spawned directly, whose `PATH` search never expands it.
#[test]
fn only_a_window_on_macos_finds_a_binary_through_a_tilde_entry() {
    let home = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(home.path().join("bin")).unwrap();
    testexec::write_executable(home.path().join("bin/claude"), "#!/bin/sh\n");
    let path = Some(OsStr::new("~/bin"));
    let h = Some(home.path().as_os_str());
    let yes = |map: &std::collections::BTreeMap<String, bool>| map.get("claude") == Some(&true);

    let mac = super::found_in(("claude", "codex"), path, h, true);
    assert!(yes(&mac.window), "a macOS window's shell expands it");
    assert!(!yes(&mac.headless), "a headless spawn does not");
    let linux = super::found_in(("claude", "codex"), path, h, false);
    assert!(!yes(&linux.window) && !yes(&linux.headless));
    let empty = super::found_in(("claude", "codex"), path, Some(OsStr::new("")), true);
    assert!(!yes(&empty.window));
    // An absolute entry is found by both.
    let absolute = home.path().join("bin");
    let both = super::found_in(("claude", "codex"), Some(absolute.as_os_str()), None, false);
    assert!(yes(&both.window) && yes(&both.headless));
    assert_eq!(both.headless.get("codex"), Some(&false));
}

/// Whole-branch review, item 3: a runtime the orchestrator's window finds only through
/// a `~` entry in `PATH` is refused for the sub-planners with the reason, not with
/// "choose another runtime", which a user with only that runtime cannot follow.
#[test]
fn a_planner_runtime_found_only_through_a_tilde_entry_says_so() {
    use proto::Runtime;
    let mut run = crate::run::orch::test_support::run_of(1);
    run.roster = config::default_roster();
    let mut o = crate::run::orch::test_support::orchestrator();
    o.route.runtime = Runtime::Codex;
    o.route.model = String::new();
    run.orch.orchestrator = Some(o);
    let map = |claude: bool, codex: bool| -> std::collections::BTreeMap<String, bool> {
        [("claude".to_string(), claude), ("codex".to_string(), codex)].into()
    };
    run.orch.installed = map(false, false);
    let bins = ("/nonexistent/claude".to_string(), "codex".to_string());
    let headless = map(false, false);
    let missing = super::missing_in(&headless, &bins);
    let window = map(false, true);
    let refusal = super::planner_refusal(&run, &missing, &window).unwrap();
    assert_eq!(
        refusal,
        "the sub-planners' runtime codex is found only through a `~` entry in PATH, which \
         headless sessions (sub-planners and scouts) do not search; put codex's directory \
         in PATH as an absolute path"
    );
    // Not found at all: the plain refusal.
    let refusal = super::planner_refusal(&run, &missing, &map(false, false)).unwrap();
    assert!(
        refusal.starts_with("the sub-planners' runtime codex is not installed (codex is not"),
        "{refusal}"
    );
}

/// Task M9.3.6a fix round 1, m1: a chain id keeps only the run id's last four
/// characters (`o-<h4>`), so a new id is redrawn while its suffix is a chain's in the
/// table, or a run's own chain. With every suffix taken, no id can be drawn.
#[test]
fn a_run_id_never_takes_a_chains_suffix() {
    use crate::manager::{GitRoots, ManagerConfig, WindowManager};
    use crate::run::chain::{Chain, ChainState};
    use crate::run::driver::RunService;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    struct NoRoots;
    impl GitRoots for NoRoots {
        fn register(&self, _: PathBuf) {}
        fn unregister(&self, _: &Path) {}
    }
    let dir = tempfile::tempdir().unwrap();
    let config = ManagerConfig::for_tests(dir.path().join("d.sock"), "/bin/sh".into());
    let (manager, _events) = WindowManager::new(config);
    let service = RunService::for_manager(&manager, dir.path().join("data"), Arc::new(NoRoots));
    assert!(service.pick_id("add login", &[]).is_ok());

    let chain = |id: String| Chain {
        id,
        project: dir.path().into(),
        runs: Vec::new(),
        window_id: 1,
        runtime: proto::Runtime::Claude,
        model: "m".into(),
        state: ChainState::Idle,
        ended: false,
    };
    {
        let mut state = crate::lock(&service.state);
        for n in 0..=u16::MAX {
            let c = chain(format!("o-{n:04x}"));
            state.chains.insert(c.id.clone(), c);
        }
    }
    assert_eq!(
        service.pick_id("add login", &[]),
        Err("could not pick a free run id".to_string())
    );

    // A run's own chain counts too: a chain dropped from the table keeps its id on its
    // runs, which a restart's rebuild would merge with a new run of the same suffix.
    let mut state = crate::lock(&service.state);
    state.chains.clear();
    let base = crate::run::orch::test_support::run_of(1);
    for n in 0..=u16::MAX {
        let mut run = base.clone();
        run.id = format!("old-{n:04x}");
        run.chain = Some(format!("o-{n:04x}"));
        state.runs.insert(run.id.clone(), run);
    }
    drop(state);
    assert_eq!(
        service.pick_id("add login", &[]),
        Err("could not pick a free run id".to_string())
    );

    // Task 6b (6a re-review N1): an unchained run's own suffix counts too, since a
    // promotion would start `o-<its suffix>` (`engine/chains.rs::assign`).
    let mut state = crate::lock(&service.state);
    state.runs.clear();
    for n in 0..=u16::MAX {
        let mut run = base.clone();
        run.id = format!("plain-{n:04x}");
        run.chain = None;
        state.runs.insert(run.id.clone(), run);
    }
    drop(state);
    assert_eq!(
        service.pick_id("add login", &[]),
        Err("could not pick a free run id".to_string())
    );
}

/// Milestone 9.5 task 9: a start's tuning, through `build_delivered`, in a real
/// temporary repository with the recorded history `refit.jsonl` in its data directory.
/// The agent binaries stand in as installed and are never launched; delivery is local.
mod tuning {
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    use super::super::Shape;
    use crate::manager::{GitRoots, ManagerConfig, WindowManager};
    use crate::run::driver::RunService;
    use crate::run::driver::delivery::tests::git;
    use crate::run::model::Run;
    use crate::run::test_support::{PROFILE, plan_with, task_toml};

    struct NoRoots;
    impl GitRoots for NoRoots {
        fn register(&self, _: PathBuf) {}
        fn unregister(&self, _: &Path) {}
    }

    /// The start lines of a start that wrote `refit.jsonl`'s refit (decision 12).
    const BUDGET_S: &str = "tuning: budget S 55 calls 18m from 34 samples";
    const WEIGHTS: &str = "tuning: path weights S 550s, M 1650s (derived), hub 1650s (derived)";
    const REFIT_S: &str = "tuning: budget S 40 calls 15m → 55 calls 18m from 34 samples";

    /// A repository with one commit, a service over `data` whose Claude and Codex are
    /// an executable that is never run, and the repository's data directory holding
    /// `refit.jsonl` as its history.
    fn rig(tmp: &Path) -> (PathBuf, PathBuf, Arc<RunService>) {
        let work = tmp.join("work");
        std::fs::create_dir_all(work.join("crates/a/src")).unwrap();
        git(&work, &["init", "-q", "-b", "main"]);
        // The repository's own identity: preflight needs one, and CI has no global one.
        git(&work, &["config", "user.name", "t"]);
        git(&work, &["config", "user.email", "t@t"]);
        std::fs::write(work.join("crates/a/src/lib.rs"), "// a\n").unwrap();
        git(&work, &["add", "-A"]);
        git(&work, &["commit", "-q", "-m", "base"]);
        let data = tmp.join("data");
        let mut config = ManagerConfig::for_tests(tmp.join("d.sock"), "/bin/sh".into());
        config.claude_bin = "/usr/bin/false".into();
        config.codex_bin = "/usr/bin/false".into();
        config.cli_caps = crate::headless::argv::CLI_CAPS;
        config.worktrees_root = data.join("worktrees");
        let (manager, _events) = WindowManager::new(config);
        let service = RunService::for_manager(&manager, data.clone(), Arc::new(NoRoots));
        let project = work.canonicalize().unwrap();
        let repo_dir = crate::profile::repo_dir(&data, &project);
        std::fs::create_dir_all(&repo_dir).unwrap();
        let history =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/history/refit.jsonl");
        std::fs::copy(history, repo_dir.join("history.jsonl")).unwrap();
        (work, repo_dir, service)
    }

    fn planned(triaged: bool) -> Shape {
        Shape::Planned(Box::new(super::super::Planned {
            triage: triaged.then(|| proto::TriageInfo {
                kinds: vec![proto::TaskKind::Code],
                scale: proto::Scale::Plan,
                path: proto::RunPath::Plan,
                reason: "several modules".into(),
                source: proto::DeciderSource::Decider,
                fallback_reason: None,
                at: 1,
            }),
            usage: None,
            yes: false,
            choice: Some(proto::OrchestratorChoice {
                runtime: proto::Runtime::Claude,
                model: None,
            }),
            design: None,
        }))
    }

    /// A plan with one S task, or none (a planned run's plan, decision 26).
    fn plan(task: bool) -> proto::Plan {
        let tasks = [task_toml("t1", "S", "[\"crates/a/src/lib.rs\"]", "")];
        let mut plan = crate::run::plan::parse_plan(&plan_with(PROFILE, &tasks)).unwrap();
        if !task {
            plan.tasks.clear();
        }
        plan
    }

    async fn start(service: &RunService, work: &Path, task: bool, shape: Shape) -> Run {
        match service
            .build_plan(plan(task), work.to_path_buf(), false, false, true, shape)
            .await
        {
            Ok(run) => run,
            Err(error) => panic!("{}", error.text()),
        }
    }

    fn log(run: &Run) -> Vec<&str> {
        run.log.iter().map(|e| e.text.as_str()).collect()
    }

    /// Decision 12 at a planned (goal) start: the run log opens with the refit-write
    /// lines of the refit this start wrote, then the start lines, `budget S` first; the
    /// run's limits hold what it froze.
    #[tokio::test]
    async fn a_planned_run_is_tuned_at_start() {
        let tmp = tempfile::tempdir().unwrap();
        let (work, repo_dir, service) = rig(tmp.path());
        let run = start(&service, &work, false, planned(true)).await;
        assert_eq!(run.state, proto::RunState::Planning);
        assert_eq!(run.repo_dir, repo_dir);
        // Ruling T9-5: the weights' refit-write and start lines are one text, written once.
        assert_eq!(log(&run)[..3], [REFIT_S, WEIGHTS, BUDGET_S]);
        assert_eq!(log(&run).iter().filter(|l| **l == WEIGHTS).count(), 1);
        assert_eq!(
            (run.limits.budget_s.tool_calls, run.limits.budget_s.minutes),
            (55, 18)
        );
        assert_eq!(
            run.limits.path_weights.as_ref().map(|w| w.s_secs),
            Some(550)
        );
        assert!(repo_dir.join("tuning.toml").exists());
    }

    /// Ruling T9-5: a build given a start's settings read (`TuneOnce::with_config`, a
    /// goal's, shared by its fast build and its fallback) builds and tunes with it, not
    /// with a second read of the live settings.
    #[tokio::test]
    async fn a_shared_settings_read_is_the_builds() {
        let tmp = tempfile::tempdir().unwrap();
        let (work, _repo_dir, service) = rig(tmp.path());
        let read = config::Orchestrator {
            budget_s: proto::Budget {
                tool_calls: 33,
                minutes: 11,
                tokens: None,
            },
            ..Default::default()
        };
        let mut configured = read.clone();
        configured.tuning.configured.s = true;
        let once = super::super::TuneOnce::with_config(configured);
        let delivery = crate::run::driver::delivery::DeliveryStart::Resolve(None);
        let flags = (false, false, true);
        let built = service
            .build_delivered(
                plan(true),
                work.clone(),
                flags,
                Shape::PlanFile,
                delivery,
                &once,
            )
            .await;
        let run = built.unwrap_or_else(|e| panic!("{}", e.text()));
        let t1 = run.task("t1").unwrap();
        assert_eq!((t1.budget.tool_calls, t1.budget.minutes), (33, 11));
        assert!(
            log(&run).contains(
                &"tuning: budget S 33 calls 11m configured (refit would be 55 calls 18m)"
            ),
            "{:?}",
            log(&run)
        );
    }

    /// Ruling RH-8: a plan start, a fast goal start, a planned goal start and a
    /// continued run each tune exactly once (the service's tuning count), through
    /// `build_delivered`; only the first one's refit wrote the file. The continued run
    /// is a stand-in: the untriaged `Shape::Planned` that `chain_goal.rs` passes to
    /// `build_delivered`, built here through `build_plan`.
    #[tokio::test]
    async fn every_start_kind_is_tuned_once() {
        let tmp = tempfile::tempdir().unwrap();
        let (work, _repo_dir, service) = rig(tmp.path());
        let kinds = [
            ("plan", true, Shape::PlanFile),
            ("fast goal", true, Shape::Fast),
            ("planned goal", false, planned(true)),
            ("continued run", false, planned(false)),
        ];
        for (n, (kind, task, shape)) in kinds.into_iter().enumerate() {
            let before = service.tuning.tunings();
            let run = start(&service, &work, task, shape).await;
            assert_eq!(service.tuning.tunings(), before + 1, "{kind}");
            let log = log(&run);
            let count = |line: &str| log.iter().filter(|l| **l == line).count();
            assert_eq!(count(BUDGET_S), 1, "{kind}: {log:?}");
            assert_eq!(count(REFIT_S), usize::from(n == 0), "{kind}: {log:?}");
            assert_eq!(
                log.iter().position(|l| *l == BUDGET_S),
                Some(if n == 0 { 2 } else { 0 })
            );
            if task {
                let t1 = run.task("t1").unwrap();
                assert_eq!(
                    (t1.budget.tool_calls, t1.budget.minutes),
                    (55, 18),
                    "{kind}"
                );
            }
        }
    }

    /// Task M9.6.3 (decision 3): a planned build decides the design mode and freezes
    /// the design limits from the start's settings read; a later config, as a second
    /// start sees it, changes neither, and the run keeps both across a save and load.
    #[tokio::test]
    async fn the_mode_is_frozen_at_start() {
        let tmp = tempfile::tempdir().unwrap();
        let (work, _repo_dir, service) = rig(tmp.path());
        let build = |config: config::Orchestrator| {
            let once = super::super::TuneOnce::with_config(config);
            let (service, work) = (service.clone(), work.clone());
            async move {
                let delivery = crate::run::driver::delivery::DeliveryStart::Resolve(None);
                let flags = (false, false, true);
                let built = service
                    .build_delivered(plan(false), work, flags, planned(true), delivery, &once)
                    .await;
                built.unwrap_or_else(|e| panic!("{}", e.text()))
            }
        };
        let mut first = config::Orchestrator::default();
        first.design.phase_minutes = 30;
        let mut run = build(first).await;
        assert_eq!(run.design_mode, proto::DesignMode::Full);
        assert_eq!(run.limits.orch.design.phase_minutes, 30);

        // The config changes: the flow is off by default and the budget is longer.
        let mut later = config::Orchestrator::default();
        later.design.default = proto::DesignMode::Off;
        later.design.phase_minutes = 90;
        let other = build(later).await;
        assert_eq!(other.design_mode, proto::DesignMode::Off);
        assert_eq!(other.limits.orch.design.phase_minutes, 90);

        // The first run keeps its own, as written and read back by a restart.
        run.data_dir = crate::run::journal::runs_dir(&tmp.path().join("saved")).join(&run.id);
        crate::run::journal::save_run(&run).unwrap();
        let (mut runs, problems) = crate::run::journal::load_all(&tmp.path().join("saved"));
        assert!(problems.is_empty(), "{problems:?}");
        let loaded = runs.remove(0).0;
        assert_eq!(loaded.design_mode, proto::DesignMode::Full);
        assert_eq!(loaded.limits.orch.design.phase_minutes, 30);
    }

    /// Ruling T12-1: a design run whose documents folder goes through a symbolic link
    /// tracked at the base is refused at its start with the commit's own text, before
    /// any run exists; with `docs_dir = ""` (nothing is committed) it starts.
    #[tokio::test]
    async fn a_design_start_through_a_symlinked_docs_dir_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let (work, _repo_dir, service) = rig(tmp.path());
        std::fs::create_dir_all(work.join("elsewhere")).unwrap();
        std::fs::write(work.join("elsewhere/keep"), "k\n").unwrap();
        std::os::unix::fs::symlink("elsewhere", work.join("docs")).unwrap();
        git(&work, &["add", "-A"]);
        git(&work, &["commit", "-q", "-m", "a link"]);
        let build = |config: config::Orchestrator| {
            let once = super::super::TuneOnce::with_config(config);
            let (service, work) = (service.clone(), work.clone());
            async move {
                let delivery = crate::run::driver::delivery::DeliveryStart::Resolve(None);
                let flags = (false, false, true);
                (service.build_delivered(plan(false), work, flags, planned(true), delivery, &once))
                    .await
                    .map_err(|e| e.text())
            }
        };
        let refused = build(config::Orchestrator::default()).await;
        assert_eq!(
            refused.err().as_deref(),
            Some(
                "design flow: the documents folder docs/anthrex goes through a symlink in the repository; change [orchestrator.design].docs_dir"
            )
        );
        let mut none = config::Orchestrator::default();
        none.design.docs_dir = String::new();
        let run = build(none).await.unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(run.design_mode, proto::DesignMode::Full);
    }

    /// Decision 3: a planned build refuses `--design full` for a goal DF §1 puts off,
    /// with the exact text, and builds nothing.
    #[tokio::test]
    async fn a_planned_build_refuses_full_for_a_research_goal() {
        let tmp = tempfile::tempdir().unwrap();
        let (work, _repo_dir, service) = rig(tmp.path());
        let Shape::Planned(mut research) = planned(true) else {
            unreachable!()
        };
        let triage = research.triage.as_mut().unwrap();
        triage.kinds = vec![proto::TaskKind::Research];
        research.design = Some(proto::DesignMode::Full);
        let built = service
            .build_plan(
                plan(false),
                work,
                false,
                false,
                true,
                Shape::Planned(research),
            )
            .await;
        let Err(error) = built else {
            panic!("a research goal built with the design flow");
        };
        assert_eq!(
            error.text(),
            "the design flow runs only for planned code or docs goals; this goal is research"
        );
    }
}
