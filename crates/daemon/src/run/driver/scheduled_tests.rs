//! Milestone 9.1 ruling C-12b: an M8a command waits for its slots in the daemon's test
//! scheduler, under no lock, and runs isolated with the slot variables. A real
//! scheduler, a real shell and a temporary git repository; no daemon, unconfined.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::manager::{ManagerConfig, WindowManager};
use crate::run::driver::{GitRoots, OpCtx, RunService};
use crate::run::engine::{OpKind, OpResult};
use crate::run::slots::{Priority, SlotRequest, Want};

struct NoRoots;
impl GitRoots for NoRoots {
    fn register(&self, _: PathBuf) {}
    fn unregister(&self, _: &Path) {}
}

const DEADLINE: Duration = Duration::from_secs(20);

fn service(top: &Path, slots: u32) -> Arc<RunService> {
    let config = ManagerConfig::for_tests("/tmp/ax-unused.sock".into(), "/bin/sh".into());
    let (manager, _events) = WindowManager::new(config);
    let testing = config::Testing {
        test_slots: Some(slots),
        ..config::Testing::default()
    };
    let ctx = crate::run::driver::RunContext::new(
        top.join("data"),
        manager.config(),
        config::Orchestrator::default(),
        Arc::new(NoRoots),
    )
    .with_testing(testing);
    RunService::new(manager, ctx)
}

#[tokio::test(flavor = "multi_thread")]
async fn an_m8a_check_waits_for_its_slots_then_runs_isolated() {
    let tmp = tempfile::tempdir().unwrap();
    let top = tmp.path().canonicalize().unwrap();
    let project = top.join("repo");
    std::fs::create_dir_all(&project).unwrap();
    let out = Command::new("git")
        .args(["init", "-q"])
        .current_dir(&project)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    let service = service(&top, 2);
    let ctx = OpCtx {
        run_id: "r1".into(),
        project: project.clone(),
        data_dir: top.join("data/runs/r1"),
        git_timeout: Duration::from_secs(30),
        check_timeout: Duration::from_secs(30),
        confine: None,
    };
    let log = top.join("check.log");
    let command = format!(
        "echo \"slots=$ANTHREX_TEST_SLOTS jobs=$CARGO_BUILD_JOBS load=$ANTHREX_TEST_LOAD tmp=$TMPDIR sock=$ANTHREX_SOCKET data=$ANTHREX_DATA_DIR\" >> '{}'",
        log.display()
    );
    // Every slot is held elsewhere: the final check waits for them.
    let busy = service
        .scheduler()
        .acquire(SlotRequest {
            priority: Priority::Candidate,
            critical: false,
            want: Want::All,
            exclusive: false,
            label: "busy".into(),
        })
        .await;
    let kind = OpKind::Check {
        dir: project.clone(),
        command,
        timeout_secs: 30,
        env: Vec::new(),
        scratch: None,
    };
    let (svc, c) = (service.clone(), ctx.clone());
    let check = tokio::spawn(async move { super::super::run(&svc, &c, 7, kind).await });
    let deadline = Instant::now() + DEADLINE;
    while service.scheduler().waiting() == 0 {
        assert!(Instant::now() < deadline, "the check never asked for slots");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(!log.exists(), "the check ran without its slots");
    drop(busy);
    let result = tokio::time::timeout(DEADLINE, check)
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(result, OpResult::Check { ok: true, .. }),
        "{result:?}"
    );
    let line = std::fs::read_to_string(&log).unwrap();
    // Decision 24: the final check is the run's completion, on every slot.
    assert!(line.starts_with("slots=2 jobs=2 load=1.0 tmp="), "{line}");
    let tmpdir = line
        .split(" tmp=")
        .nth(1)
        .unwrap()
        .split(' ')
        .next()
        .unwrap();
    assert!(tmpdir.ends_with("/s7-check"), "{line}");
    assert!(
        !tmpdir.starts_with(&project.display().to_string()),
        "{line}"
    );
    assert!(
        line.contains(&format!("sock={tmpdir}/d.sock data={tmpdir}/data")),
        "{line}"
    );
    assert!(!Path::new(tmpdir).exists(), "the step directory is removed");
}
