//! Milestone 9.15 ruling C3: every end-to-end daemon a test starts runs with lessons
//! off (`[lessons] enabled = false`, `risk_note = false`), so no background `distill`
//! or `risk_note` decider call races a test's reads, unless the test opts in by
//! supplying its own `[lessons]` table, which replaces the default rather than adding a
//! second one.

use std::path::{Path, PathBuf};
use std::time::Duration;

use proto::{ClientMsg, RunReply, RunRequest, SettingsReply, SettingsRequest};

use super::run_harness::connect;
use super::runtime;

/// The harnesses' default `[lessons]` table.
pub const LESSONS_OFF: &str = "[lessons]\nenabled = false\nrisk_note = false\n";

/// Whether `toml` has its own `[lessons]` table.
pub fn has_lessons_table(toml: &str) -> bool {
    toml.lines().any(|l| l.trim() == "[lessons]")
}

/// `extra` appended to the config at `config`. When `extra` has its own `[lessons]`
/// table, the harness's default one is removed first, so the file has one table; so is
/// the default `[orchestrator.brakes]` (milestone 9.16 ruling C19).
pub fn append_toml(config: &Path, extra: &str) {
    let mut text = std::fs::read_to_string(config).unwrap();
    if super::brakes::has_brakes_table(extra) && text.contains(super::brakes::BRAKES_OFF) {
        text = text.replacen(super::brakes::BRAKES_OFF, "", 1);
    }
    if has_lessons_table(extra) {
        assert!(
            text.contains(LESSONS_OFF),
            "no default [lessons] to replace"
        );
        text = text.replacen(LESSONS_OFF, "", 1);
    }
    std::fs::write(config, format!("{text}\n{extra}\n")).unwrap();
}

/// The config file the daemon at `socket` reads, as its settings reply names it.
pub fn daemon_config_path(socket: &Path, wait: Duration) -> PathBuf {
    let reply = runtime().block_on(async move {
        let mut stream = connect(socket).await;
        let request = RunRequest::Settings(SettingsRequest::Get);
        proto::write_frame(&mut stream, &ClientMsg::RunTagged { id: 1, request })
            .await
            .unwrap();
        tokio::time::timeout(wait, async {
            loop {
                match proto::read_frame::<_, proto::DaemonMsg>(&mut stream).await {
                    Ok(Some(proto::DaemonMsg::Run(reply))) if reply.request_id() == Some(1) => {
                        return reply;
                    }
                    Ok(Some(_)) => {}
                    other => panic!("connection ended: {other:?}"),
                }
            }
        })
        .await
        .expect("the settings request timed out")
    });
    let RunReply::Settings { reply, .. } = reply else {
        panic!("{reply:?}")
    };
    let SettingsReply::Current { path, .. } = *reply else {
        panic!("{reply:?}")
    };
    path
}
