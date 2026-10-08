//! Milestone 9.8 decision 40 (M9.8.12), end to end: a Settings save writes the role
//! table as `[models]`, removes the old keys it replaced, keeps `config.toml.bak`, and
//! a run started afterwards launches its worker on the saved row.

mod support;

use std::time::{Duration, Instant};

use proto::models::{ModelRef, Role, RoleChoice};
use proto::{RunReply, RunRequest, SettingsReply, SettingsRequest};

use crate::support::run_harness::{REQUEST_WAIT, RUN_WAIT, RunHarness};
use crate::support::run_plans::{approve, commit, done, plan, task};

/// The old keys: an `s` list naming Sonnet, and a roster entry.
const OLD_KEYS: &str = r#"[orchestrator.routes.s]
candidates = [{ runtime = "claude", model = "claude-sonnet-5" }]

[[orchestrator.models]]
runtime = "codex"
model = "gpt-6-sol"
strength = "standard"
"#;

fn settings(h: &RunHarness, id: u64, request: SettingsRequest) -> SettingsReply {
    let RunReply::Settings { reply, .. } =
        h.tagged(id, RunRequest::Settings(request), REQUEST_WAIT)
    else {
        panic!("no settings reply\n{}", h.log_tail())
    };
    *reply
}

#[test]
fn saving_models_rewrites_the_config_and_new_runs_use_them() {
    let h = RunHarness::with_config("", OLD_KEYS, &[]);
    let config = h.dir.path().join("config.toml");
    let before = std::fs::read_to_string(&config).unwrap();

    let SettingsReply::Current { mut doc, .. } = settings(&h, 1, SettingsRequest::Get) else {
        panic!("no Current reply")
    };
    // The migrated row, from the old `s` list.
    let small = &doc.roles.rows[&Role::ImplementerSmall];
    assert_eq!(
        small.model,
        ModelRef::parse("claude:claude-sonnet-5").unwrap()
    );
    doc.roles.rows.insert(
        Role::ImplementerSmall,
        RoleChoice {
            model: ModelRef::parse("claude:claude-opus-5-5").unwrap(),
            effort: Some("high".into()),
            fallback: None,
        },
    );
    let reply = settings(&h, 2, SettingsRequest::Put { settings: doc });
    assert!(matches!(reply, SettingsReply::Saved { .. }), "{reply:?}");

    let text = std::fs::read_to_string(&config).unwrap();
    assert!(
        text.contains(
            "[models.implementer.small]\nmodel = \"claude:claude-opus-5-5\"\neffort = \"high\"\n"
        ),
        "{text}"
    );
    for gone in ["[orchestrator.routes.s]", "[[orchestrator.models]]"] {
        assert!(!text.contains(gone), "{gone} is still there:\n{text}");
    }
    assert!(
        text.contains("git_timeout_secs = "),
        "the harness's lines stay:\n{text}"
    );
    let bak = std::fs::read_to_string(h.dir.path().join("config.toml.bak")).unwrap();
    assert_eq!(bak, before);

    // A new run's S worker runs on the saved row.
    h.script("worker-t1-1", &[commit("a.txt", "a\n"), done("added a")]);
    h.script("reviewer-t1-1", &[approve()]);
    let _run = h.start(&plan("", &[task("t1", &["a.txt"], "")]), true);
    let deadline = Instant::now() + RUN_WAIT;
    let argv = loop {
        let lines = h.io_lines("worker-t1-1", "args");
        let first = lines
            .first()
            .and_then(|l| serde_json::from_str::<Vec<String>>(l).ok());
        if let Some(argv) = first {
            break argv;
        }
        assert!(
            Instant::now() < deadline,
            "the worker never started\n{}",
            h.log_tail()
        );
        std::thread::sleep(Duration::from_millis(50));
    };
    let has = |pair: [&str; 2]| argv.windows(2).any(|w| w == pair);
    assert!(has(["--model", "claude-opus-5-5"]), "{argv:?}");
    assert!(has(["--effort", "high"]), "{argv:?}");
}
