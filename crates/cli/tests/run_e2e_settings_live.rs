//! Review focus 3: a settings save reaches new runs only (decision 29), end to end.

mod support;

use proto::models::{ModelRef, Role, RoleChoice};
use proto::{RunReply, RunRequest, Runtime, SettingsReply, SettingsRequest};

use crate::support::run_harness::{REQUEST_WAIT, RUN_WAIT, RunHarness};
use crate::support::run_plans::{approve, commit, done, plan, task};
use serde_json::json;

#[test]
fn e2e_settings_save_applies_to_new_runs_only() {
    let h = RunHarness::new("max_writers = 3");
    // Run A holds its worker open, so it is running across the save.
    h.script(
        "worker-t1-1",
        &[commit("a.txt", "a\n"), json!({"hang": {}})],
    );
    let a = h.start(&plan("", &[task("t1", &["a.txt"], "")]), true);
    h.wait_run(&a, |r| r.state == proto::RunState::Running, RUN_WAIT);

    let RunReply::Settings { reply, .. } =
        h.tagged(1, RunRequest::Settings(SettingsRequest::Get), REQUEST_WAIT)
    else {
        panic!("no settings reply\n{}", h.log_tail())
    };
    let SettingsReply::Current { mut doc, .. } = *reply else {
        panic!("{reply:?}")
    };
    doc.limits.max_writers = 1;
    // M9.8.12: B's task (size S) takes the saved `implementer.small` row, a model the
    // built-in table never gives it.
    doc.roles.rows.insert(
        Role::ImplementerSmall,
        RoleChoice {
            model: ModelRef::parse("claude:claude-opus-5-5").unwrap(),
            effort: None,
            fallback: None,
        },
    );
    let RunReply::Settings { reply, .. } = h.tagged(
        2,
        RunRequest::Settings(SettingsRequest::Put {
            settings: doc.clone(),
        }),
        REQUEST_WAIT,
    ) else {
        panic!("no settings reply")
    };
    assert!(matches!(*reply, SettingsReply::Saved { .. }), "{reply:?}");

    // A keeps what it froze; B gets the new limits and role table.
    assert_eq!(h.run(&a).unwrap().max_writers, 3);
    h.script("worker-t9-1", &[commit("b.txt", "b\n"), done("added b")]);
    h.script("reviewer-t9-1", &[approve()]);
    let b = h.start(&plan("", &[task("t9", &["b.txt"], "")]), true);
    let info = h.wait_run(&b, |r| !r.tasks.is_empty(), RUN_WAIT);
    assert_eq!(info.max_writers, 1);
    let route = &info.tasks[0].route;
    assert_eq!(
        (route.runtime, route.model.as_str()),
        (Runtime::Claude, "claude-opus-5-5")
    );
    // The file holds the save, the harness's own lines kept.
    let text = std::fs::read_to_string(h.dir.path().join("config.toml")).unwrap();
    assert!(
        text.contains("git_timeout_secs = 5") && text.contains("[orchestrator.cache_dirs]"),
        "{text}"
    );
}
