//! Task M9.2.4: `open_pr`'s and `failed_logs`'s behaviour through `ScriptedRunner` and
//! a stand-in `gh` script the test writes itself. Split from `tests_parse.rs` for
//! AGENTS.md rule 8 (move-only).

use std::path::{Path, PathBuf};

use serde_json::Value;

use super::scripted::ScriptedRunner;
use super::tests::*;
use super::*;

#[test]
fn open_pr_returns_an_existing_pr_for_the_head() {
    let tmp = tempfile::tempdir().unwrap();
    let req = OpenPrReq {
        repo: repo(tmp.path()),
        run_id: "r1a2b".to_string(),
        base: "trunk".to_string(),
        head: format!("anthrex/{RUN}/stage-1"),
        title: "[anthrex r1a2b 1/1] Add the parser".to_string(),
        body_file: tmp.path().join("pr-1.md"),
    };
    // The repository's own PR for the head; the fork's open PR of the same branch name
    // (ruling R-10) is not ours, even though it is open and newer.
    let h = host(ScriptedRunner::new().ok(PR_LIST_HEAD_OWNER));
    assert_eq!(
        h.open_pr(&req).unwrap(),
        PrRef {
            number: 13940,
            url: "https://github.com/cli/cli/pull/13940".to_string(),
            state: PrState::Merged,
            existed: true,
            // Fix wave A1: the base `pr list` reports, for the engine to retarget.
            base: Some("trunk".to_string()),
        }
    );
    assert_eq!(h.runner().calls().len(), 1, "no gh pr create runs");
    assert!(
        !h.runner()
            .argvs()
            .iter()
            .any(|a| a.get(1).is_some_and(|c| c == "create"))
    );

    // Only the fork has one: a new PR is created.
    let mut rows: Value = serde_json::from_str(PR_LIST_HEAD_OWNER).unwrap();
    rows.as_array_mut().unwrap().remove(0);
    let h = host(
        ScriptedRunner::new()
            .ok(&rows.to_string())
            .ok("https://github.com/cli/cli/pull/14001\n"),
    );
    let made = h.open_pr(&req).unwrap();
    assert_eq!((made.number, made.existed), (14001, false));
    assert_eq!(
        made.base, None,
        "a created PR has the base the engine asked for"
    );
    assert_eq!(h.runner().argvs()[1][1], "create");

    // gh's answer without the owner fields (M9.2.1's recording predates R-10) is never
    // read as "none of ours".
    let h = host(ScriptedRunner::new().ok(PR_LIST_HEAD));
    assert_eq!(
        h.open_pr(&req),
        Err(HostError::Rejected(
            "gh output changed: headRepositoryOwner".to_string()
        ))
    );
    assert_eq!(h.runner().calls().len(), 1);
}

/// A stand-in `gh` that prints `log` `times` times (the only process this file starts).
fn stand_in_gh(dir: &Path, log: &Path, times: usize) -> PathBuf {
    let script = dir.join("gh");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\ni=0\nwhile [ \"$i\" -lt {times} ]; do cat '{}'; i=$((i + 1)); done\n",
            log.display()
        ),
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    script
}

#[test]
fn failed_logs_keeps_the_head_and_the_tail_under_the_cap() {
    let tmp = tempfile::tempdir().unwrap();
    let fixture = tmp.path().join("fixture.log");
    std::fs::write(&fixture, RUN_LOG_FAILED).unwrap();
    let repo = repo(tmp.path());
    let first_line = RUN_LOG_FAILED.lines().next().unwrap();
    let last_line = RUN_LOG_FAILED.lines().last().unwrap();

    // 1000 copies (about 700 KB) under a 256 KiB cap.
    let gh = stand_in_gh(tmp.path(), &fixture, 1000);
    let host = GhHost::new(SystemRunner::new(&gh, "git"), &gh, "git");
    let cap = 256 * 1024;
    let out = tmp.path().join("ci-1.log");
    let file = host.failed_logs(&repo, 28656994029, cap, &out).unwrap();
    let bytes = std::fs::read(&out).unwrap();
    assert!(file.truncated);
    assert_eq!(file.bytes, bytes.len() as u64);
    assert!(file.bytes <= cap, "{} > {cap}", file.bytes);
    assert!(file.bytes > cap - 128, "the cap is used: {}", file.bytes);
    let text = String::from_utf8_lossy(&bytes);
    assert!(text.starts_with(first_line), "the head is kept");
    assert!(text.trim_end().ends_with(last_line), "the tail is kept");
    let dropped = RUN_LOG_FAILED.len() as u64 * 1000 - (bytes.len() as u64)
        + text.lines().find(|l| l.contains("cut here")).unwrap().len() as u64
        + 2;
    assert!(
        text.contains(&format!("… {dropped} bytes of the log cut here …")),
        "{dropped}"
    );
    let head_end = text.find("\n… ").unwrap();
    assert_eq!(head_end, 64 * 1024);
    // Task M9.2.9: the answer's `tail` is the file's last 48 KiB, for the engine.
    let want = &bytes[bytes.len() - crate::decider::CI_SUMMARY_INPUT_BYTES..];
    assert_eq!(file.tail, String::from_utf8_lossy(want));
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(
        std::fs::metadata(&out).unwrap().permissions().mode() & 0o777,
        0o600
    );

    // Under the cap the log is kept whole.
    let gh = stand_in_gh(tmp.path(), &fixture, 3);
    let host = GhHost::new(SystemRunner::new(&gh, "git"), &gh, "git");
    let file = host.failed_logs(&repo, 1, cap, &out).unwrap();
    assert!(!file.truncated);
    assert_eq!(
        file.tail,
        RUN_LOG_FAILED.repeat(3),
        "a short log is its own tail"
    );
    assert_eq!(
        std::fs::read_to_string(&out).unwrap(),
        RUN_LOG_FAILED.repeat(3)
    );

    // A gh that is not there is `Missing`, naming the path.
    let host = GhHost::new(
        SystemRunner::new("/nonexistent/anthrex-test/gh", "git"),
        "/nonexistent/anthrex-test/gh",
        "git",
    );
    assert_eq!(
        host.failed_logs(&repo, 1, cap, &out),
        Err(HostError::Missing(
            "gh is not installed (looked for /nonexistent/anthrex-test/gh)".to_string()
        ))
    );
}
