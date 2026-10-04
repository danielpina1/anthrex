//! Task M9.5.18: a racer's launch spec as the headless launcher turns it into argv
//! (decision 19): its `anthrex mcp` server names its lane, on Claude's `--mcp-config`
//! and on Codex's `-c mcp_servers.anthrex.args`.

use std::path::Path;

use proto::{LaneState, RaceLane};
use serde_json::Value;

use super::racer_spec;
use crate::headless::SessionArg;
use crate::headless::argv::{CLI_CAPS, claude_args, codex_args};
use crate::run::test_support::{PROFILE, plan_with, race_of, run_ok, task_toml};

/// `--mcp-config`'s anthrex server arguments in a Claude argv.
fn claude_mcp(args: &[String]) -> Vec<String> {
    let at = args.iter().position(|a| a == "--mcp-config").unwrap();
    let config: Value = serde_json::from_str(&args[at + 1]).unwrap();
    serde_json::from_value(config["mcpServers"]["anthrex"]["args"].clone()).unwrap()
}

/// The `-c mcp_servers.anthrex.args=…` value in a Codex argv.
fn codex_mcp(args: &[String]) -> String {
    let value = args
        .iter()
        .find_map(|a| a.strip_prefix("mcp_servers.anthrex.args="))
        .unwrap();
    value.to_string()
}

#[test]
fn a_racer_mcp_config_names_its_lane() {
    let mut run = run_ok(&plan_with(
        PROFILE,
        &[task_toml("t1", "M", "[\"crates/a/**\"]", "")],
    ));
    let race = race_of(&run.tasks[0], [LaneState::Working, LaneState::Working]);
    run.tasks[0].race = Some(race.clone());
    let task = &run.tasks[0];
    let (exe, socket) = (Path::new("/x/anthrex"), Path::new("/s/d.sock"));
    let session = SessionArg::New { uuid: None };

    let a = racer_spec(&run, task, &race.lanes[0]);
    assert_eq!(a.runtime, proto::Runtime::Claude);
    let mcp = claude_mcp(&claude_args(&a, &session, exe, 4, socket, &CLI_CAPS));
    let at = mcp.iter().position(|a| a == "--lane").expect("a --lane");
    assert_eq!(mcp[at + 1], "a", "{mcp:?}");
    assert_eq!(&mcp[1..3], ["--role", "racer"], "{mcp:?}");

    let b = racer_spec(&run, task, &race.lanes[1]);
    assert_eq!(b.runtime, proto::Runtime::Codex);
    let mcp = codex_mcp(&codex_args(&b, &session, "go", exe, 5, socket, &CLI_CAPS));
    assert!(mcp.contains(r#""--lane","b""#), "{mcp}");
    assert!(mcp.contains(r#""--role","racer""#), "{mcp}");
    assert_eq!(
        (
            b.run_ref.as_ref().unwrap().lane,
            b.mcp.as_ref().unwrap().lane
        ),
        (Some(RaceLane::B), Some(RaceLane::B))
    );
}
