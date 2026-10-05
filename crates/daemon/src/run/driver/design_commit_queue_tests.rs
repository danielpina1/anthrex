//! Milestone 9.6 task M9.6.12, split out of `design_commit_tests.rs` (the 400-line rule
//! for a new file): the documents commit through the project's git queue, every call
//! with `--no-optional-locks`, the protect flags and the driver's own index (hard rules
//! 2, 10 and 11 and the task's addendum).

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::tests::{Rig, T};
use crate::run::engine::{OpKind, OpResult};

/// A `git` stand-in that logs each call's arguments and `GIT_` variables, then runs
/// the real git.
pub(super) fn recording(dir: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let script = dir.join("recording-git");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\n\
             log='{log}'\n\
             {{ printf 'argv'; for a in \"$@\"; do printf '\\t%s' \"$a\"; done; printf '\\n'; }} >> \"$log\"\n\
             env | grep '^GIT_' | while IFS= read -r l; do printf 'env\\t%s\\n' \"$l\"; done >> \"$log\"\n\
             exec git \"$@\"\n",
            log = dir.join("git.log").display(),
        ),
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    script
}

/// Hard rules 2, 10 and 11 and the addendum: the commit waits for the project's git
/// queue (no git runs while another write holds it), every call carries
/// `--no-optional-locks` and the protect flags, and only the index commands see
/// `GIT_INDEX_FILE`, set to the driver's own index in the run's data directory.
#[test]
fn the_commit_uses_the_queue_no_optional_locks_and_the_scrubbed_env() {
    let scripts = tempfile::tempdir().unwrap();
    let program = recording(scripts.path());
    let rig = Rig::new(Some(program));
    let spec = rig.spec_and_plan();
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    let log_path = scripts.path().join("git.log");
    let result = rt.block_on(async {
        let (held_tx, held_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
        let release_rx = std::sync::Mutex::new(release_rx);
        let queue = rig.service.queue.clone();
        let project = rig.ctx.project.clone();
        let holder = tokio::spawn(async move {
            queue
                .write(&project, move || {
                    let _ = held_tx.send(());
                    let _ = crate::lock(&release_rx).recv_timeout(T);
                    Ok(())
                })
                .await
        });
        held_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        let (service, ctx) = (rig.service.clone(), rig.ctx.clone());
        let op = tokio::spawn(async move {
            let kind = OpKind::CommitDesignDocs(Box::new(spec));
            super::super::ops::run(&service, &ctx, 1, kind).await
        });
        // Ruling T12-3 (m4): the commit is parked on the held queue, or a git call ran
        // without it, whichever comes first; a deadline only guards a hang.
        let deadline = Instant::now() + Duration::from_secs(10);
        while rig.service.queue.waiters() == 0 && !log_path.exists() {
            assert!(
                Instant::now() < deadline,
                "the commit neither waited nor ran"
            );
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert!(
            !log_path.exists(),
            "a git call ran while the queue was held"
        );
        assert_eq!(
            rig.service.queue.waiters(),
            1,
            "the commit waits for the queue"
        );
        assert!(!op.is_finished());
        release_tx.send(()).unwrap();
        holder.await.unwrap().unwrap();
        op.await.unwrap()
    });
    assert!(
        matches!(result, OpResult::DocsCommitted { .. }),
        "{result:?}"
    );
    let log = std::fs::read_to_string(scripts.path().join("git.log")).unwrap();
    let index = format!(
        "GIT_INDEX_FILE={}",
        rig.ctx.data_dir.join("design/commit.index").display()
    );
    let mut calls = 0;
    let mut indexed = Vec::new();
    let mut last: Vec<String> = Vec::new();
    let mut check = |argv: &[String], env: &[String]| {
        if argv.is_empty() {
            return;
        }
        assert_eq!(
            (argv[0].as_str(), argv[2].as_str()),
            ("-C", "--no-optional-locks")
        );
        let joined = argv.join(" ");
        assert!(
            joined.contains("-c core.protectHFS=true -c core.protectNTFS=true"),
            "{argv:?}"
        );
        let index_call = ["read-tree", "update-index", "write-tree"]
            .iter()
            .find(|sub| argv.iter().any(|a| a == *sub));
        match index_call {
            Some(sub) => {
                assert!(env.contains(&index), "{argv:?}: {env:?}");
                indexed.push(sub.to_string());
            }
            None => assert!(
                !env.iter().any(|e| e.starts_with("GIT_INDEX_FILE=")),
                "{argv:?}: {env:?}"
            ),
        }
        calls += 1;
    };
    let mut env: Vec<String> = Vec::new();
    for line in log.lines() {
        let mut fields = line.split('\t');
        match fields.next() {
            Some("argv") => {
                check(&last, &env);
                last = fields.map(String::from).collect();
                env.clear();
            }
            Some("env") => env.push(fields.collect()),
            other => panic!("{other:?}"),
        }
    }
    check(&last, &env);
    assert!(calls >= 8, "{log}");
    assert_eq!(
        indexed,
        ["read-tree", "update-index", "update-index", "write-tree"]
    );
}
