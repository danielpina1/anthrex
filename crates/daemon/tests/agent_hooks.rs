use daemon::manager::{ManagerConfig, WindowManager};
use daemon::window::WindowEvent;
use proto::{HookSource, Runtime, Status, WindowInfo, WindowSpec};
use serde_json::{Value, json};
use std::os::unix::fs::PermissionsExt;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

struct Agent {
    manager: Arc<WindowManager>,
    events: mpsc::UnboundedReceiver<(u32, WindowEvent)>,
    id: u32,
    _dir: tempfile::TempDir,
}

impl Agent {
    fn new(output: bool) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let stub = dir.path().join("stub.sh");
        let body = if output {
            "#!/bin/sh\nwhile :; do echo tick; sleep 0.2; done\n"
        } else {
            "#!/bin/sh\nexec sleep 300\n"
        };
        std::fs::write(&stub, body).unwrap();
        std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755)).unwrap();
        let mut config = ManagerConfig::new(dir.path().join("daemon.sock"), "/bin/sh".into());
        config.claude_bin = stub.to_str().unwrap().into();
        let (manager, events) = WindowManager::new(config);
        let id = manager
            .create(
                WindowSpec {
                    name: None,
                    runtime: Runtime::Claude,
                    cwd: dir.path().into(),
                    worktree_branch: None,
                    model: Some("test-model".into()),
                    initial_prompt: None,
                },
                std::env::temp_dir(),
                None,
                80,
                24,
            )
            .unwrap()
            .id;
        Self {
            manager,
            events,
            id,
            _dir: dir,
        }
    }

    fn hook(&self, payload: Value) {
        self.manager
            .handle_hook(self.id, HookSource::Claude, &payload)
            .unwrap();
    }

    fn info(&self) -> WindowInfo {
        self.manager
            .list()
            .into_iter()
            .find(|w| w.id == self.id)
            .unwrap()
    }

    async fn output(&mut self) {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let (id, event) = self.events.recv().await.unwrap();
                let output = matches!(event, WindowEvent::Output);
                self.manager.handle_event(id, event);
                if output {
                    break;
                }
            }
        })
        .await
        .expect("stub produced no output");
    }
}

impl Drop for Agent {
    fn drop(&mut self) {
        let _ = self.manager.remove(self.id);
    }
}

#[test]
fn claude_hooks_drive_status_tool_and_session() {
    let a = Agent::new(false);
    a.hook(json!({"hook_event_name":"SessionStart", "session_id":"s1"}));
    assert_eq!(a.info().status, Status::Idle);
    assert_eq!(a.info().session_id.as_deref(), Some("s1"));
    assert_eq!(a.info().model.as_deref(), Some("test-model"));
    a.hook(json!({"hook_event_name":"UserPromptSubmit"}));
    assert_eq!(a.info().status, Status::Working);
    let mut changes = a.manager.watch();
    a.hook(json!({"hook_event_name":"PreToolUse", "tool_name":"Bash"}));
    assert_eq!(a.info().tool.as_deref(), Some("Bash"));
    assert!(
        changes.has_changed().unwrap(),
        "tool-only change must publish"
    );
    changes.borrow_and_update();
    a.hook(json!({"hook_event_name":"PostToolUse"}));
    assert_eq!(a.info().tool, None);
    assert!(changes.has_changed().unwrap());
    a.hook(json!({"hook_event_name":"PreToolUse", "tool_name":"Read"}));
    a.hook(json!({"hook_event_name":"Stop"}));
    assert_eq!(a.info().status, Status::Done);
    assert_eq!(a.info().tool, None);
}

#[test]
fn session_id_fills_once_then_latest_session_start_replaces_it() {
    let a = Agent::new(false);
    a.hook(json!({"hook_event_name":"UserPromptSubmit", "session_id":"early"}));
    assert_eq!(a.info().session_id.as_deref(), Some("early"));
    a.hook(json!({"hook_event_name":"PreToolUse", "session_id":"ignored"}));
    assert_eq!(a.info().session_id.as_deref(), Some("early"));
    a.hook(json!({"hook_event_name":"SessionStart", "session_id":"latest"}));
    assert_eq!(a.info().session_id.as_deref(), Some("latest"));
    assert_eq!(
        a.info().status,
        Status::Working,
        "context must precede flag mutation"
    );
}

#[test]
fn stop_on_a_viewed_window_is_idle() {
    let a = Agent::new(false);
    a.manager.focus(a.id);
    a.manager.focus(a.id);
    a.manager.unfocus(a.id);
    a.hook(json!({"hook_event_name":"Stop"}));
    assert_eq!(a.info().status, Status::Idle);
    a.manager.unfocus(a.id);
    a.manager.unfocus(a.id);
    a.hook(json!({"hook_event_name":"UserPromptSubmit"}));
    a.hook(json!({"hook_event_name":"Stop"}));
    assert_eq!(a.info().status, Status::Done);
}

#[test]
fn sub_agent_tool_events_do_not_touch_the_window_tool() {
    let a = Agent::new(false);
    a.hook(json!({"hook_event_name":"PreToolUse", "agent_id":"child", "tool_name":"Read"}));
    assert_eq!(a.info().tool, None);
    assert_eq!(a.info().status, Status::Working);
    a.hook(json!({"hook_event_name":"PreToolUse", "tool_name":"Bash"}));
    a.hook(json!({"hook_event_name":"PostToolUse", "agent_id":"child"}));
    assert_eq!(a.info().tool.as_deref(), Some("Bash"));
}

#[tokio::test]
async fn the_first_hook_disables_the_output_fallback() {
    let mut a = Agent::new(true);
    a.output().await;
    assert_eq!(a.info().status, Status::Working);
    a.hook(json!({"hook_event_name":"SessionStart"}));
    assert_eq!(a.info().status, Status::Idle);
    let until = Instant::now() + Duration::from_millis(800);
    while Instant::now() < until {
        a.output().await;
        assert_eq!(a.info().status, Status::Idle);
    }
}

#[test]
fn unknown_window_is_an_error() {
    let a = Agent::new(false);
    let error = a
        .manager
        .handle_hook(99, HookSource::Claude, &json!({}))
        .unwrap_err();
    assert_eq!(error.to_string(), "no window with id 99");
}

#[test]
fn wrong_source_is_ignored() {
    let a = Agent::new(false);
    let before = a.info();
    a.manager
        .handle_hook(
            a.id,
            HookSource::CodexNotify,
            &json!({"type":"agent-turn-complete"}),
        )
        .unwrap();
    assert_eq!(a.info(), before);
    a.manager.handle_event(a.id, WindowEvent::Output);
    assert_eq!(a.info().status, Status::Working);
}

#[test]
fn unparseable_payload_is_ignored() {
    let a = Agent::new(false);
    let before = a.info();
    a.hook(json!("invalid"));
    a.hook(json!({"hook_event_name":"future", "text":"é".repeat(2000)}));
    assert_eq!(a.info(), before);
    a.manager.handle_event(a.id, WindowEvent::Output);
    assert_eq!(a.info().status, Status::Working);
}

#[test]
fn bell_after_hooks_does_not_raise_attention() {
    let a = Agent::new(false);
    a.hook(json!({"hook_event_name":"SessionStart"}));
    a.manager.handle_event(a.id, WindowEvent::Bell);
    assert_eq!(a.info().status, Status::Idle);
}
