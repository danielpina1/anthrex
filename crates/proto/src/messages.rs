use crate::types::{ClientKind, WindowInfo, WindowSpec};
use serde::{Deserialize, Serialize};

/// Who produced a hook event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum HookSource {
    Claude,
    CodexNotify,
    CodexHook,
}

/// Client → daemon.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ClientMsg {
    Hello {
        proto_version: u32,
        client: ClientKind,
    },
    ListWindows,
    CreateWindow {
        spec: WindowSpec,
        cols: u16,
        rows: u16,
    },
    Subscribe {
        window_id: u32,
        cols: u16,
        rows: u16,
    },
    Unsubscribe,
    Input {
        window_id: u32,
        #[serde(with = "serde_bytes")]
        bytes: Vec<u8>,
    },
    Resize {
        window_id: u32,
        cols: u16,
        rows: u16,
    },
    Kill {
        window_id: u32,
    },
    Remove {
        window_id: u32,
        remove_worktree: bool,
        force: bool,
    },
    Restart {
        window_id: u32,
    },
    Rename {
        window_id: u32,
        name: String,
    },
    HookEvent {
        window_id: u32,
        source: HookSource,
        payload: serde_json::Value,
    },
    Shutdown,
}

/// Daemon → client.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum DaemonMsg {
    Welcome {
        daemon_version: String,
        windows: Vec<WindowInfo>,
    },
    WindowsChanged {
        windows: Vec<WindowInfo>,
    },
    Created {
        window_id: u32,
    },
    Snapshot {
        window_id: u32,
        cols: u16,
        rows: u16,
        #[serde(with = "serde_bytes")]
        bytes: Vec<u8>,
    },
    Output {
        window_id: u32,
        #[serde(with = "serde_bytes")]
        bytes: Vec<u8>,
    },
    Ack {
        request: String,
    },
    Error {
        request: String,
        message: String,
    },
    Bye {
        reason: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{
        ClientKind, Runtime, Status, SubagentInfo, SubagentState, WindowInfo, WindowSpec,
    };

    #[test]
    fn input_bytes_survive_messagepack() {
        let msg = ClientMsg::Input {
            window_id: 3,
            bytes: vec![0x1b, b'[', b'A', 0xff],
        };
        let packed = rmp_serde::to_vec_named(&msg).unwrap();
        let back: ClientMsg = rmp_serde::from_slice(&packed).unwrap();
        assert_eq!(back, msg);
    }

    #[test]
    fn hook_payload_survives_messagepack() {
        let msg = ClientMsg::HookEvent {
            window_id: 1,
            source: HookSource::Claude,
            payload: serde_json::json!({"hook_event_name": "Stop", "session_id": "abc"}),
        };
        let packed = rmp_serde::to_vec_named(&msg).unwrap();
        let back: ClientMsg = rmp_serde::from_slice(&packed).unwrap();
        assert_eq!(back, msg);
    }

    #[test]
    fn hook_source_serializes_kebab_case() {
        assert_eq!(
            serde_json::to_string(&HookSource::Claude).unwrap(),
            "\"claude\""
        );
        assert_eq!(
            serde_json::to_string(&HookSource::CodexNotify).unwrap(),
            "\"codex-notify\""
        );
        assert_eq!(
            serde_json::to_string(&HookSource::CodexHook).unwrap(),
            "\"codex-hook\""
        );
    }

    #[test]
    fn every_message_round_trips() {
        let spec = WindowSpec {
            name: Some("shell".into()),
            runtime: Runtime::Shell,
            cwd: "/tmp".into(),
            worktree_branch: Some("feat/protocol".into()),
            model: Some("opus".into()),
            initial_prompt: Some("hello".into()),
        };
        let window = WindowInfo {
            id: 1,
            name: "shell".into(),
            runtime: Runtime::Shell,
            cwd: "/tmp".into(),
            branch: Some("feat/protocol".into()),
            status: Status::Working,
            tool: Some("Read".into()),
            since_secs: 3,
            last_output_secs: 1,
            session_id: Some("s1".into()),
            model: Some("opus".into()),
            subagents: vec![SubagentInfo {
                id: "agent-1".into(),
                parent_id: Some("parent-1".into()),
                kind: "explore".into(),
                label: Some("Inspect protocol".into()),
                model: Some("haiku".into()),
                state: SubagentState::Running,
                tool: Some("Read".into()),
                started_secs: 2,
                ended_secs: Some(3),
                needs_permission: true,
            }],
            exit: None,
        };
        let client_messages = vec![
            ClientMsg::Hello {
                proto_version: 1,
                client: ClientKind::Tui,
            },
            ClientMsg::ListWindows,
            ClientMsg::CreateWindow {
                spec,
                cols: 80,
                rows: 24,
            },
            ClientMsg::Subscribe {
                window_id: 1,
                cols: 80,
                rows: 24,
            },
            ClientMsg::Unsubscribe,
            ClientMsg::Input {
                window_id: 1,
                bytes: vec![0, 0xff],
            },
            ClientMsg::Resize {
                window_id: 1,
                cols: 100,
                rows: 30,
            },
            ClientMsg::Kill { window_id: 1 },
            ClientMsg::Remove {
                window_id: 1,
                remove_worktree: true,
                force: true,
            },
            ClientMsg::Restart { window_id: 1 },
            ClientMsg::Rename {
                window_id: 1,
                name: "renamed".into(),
            },
            ClientMsg::HookEvent {
                window_id: 1,
                source: HookSource::CodexHook,
                payload: serde_json::json!({"event": "agent-turn-complete"}),
            },
            ClientMsg::Shutdown,
        ];
        for message in client_messages {
            let back: ClientMsg =
                rmp_serde::from_slice(&rmp_serde::to_vec_named(&message).unwrap()).unwrap();
            assert_eq!(back, message);
        }

        let daemon_messages = vec![
            DaemonMsg::Welcome {
                daemon_version: "0.1.0".into(),
                windows: vec![window.clone()],
            },
            DaemonMsg::WindowsChanged {
                windows: vec![window],
            },
            DaemonMsg::Created { window_id: 1 },
            DaemonMsg::Snapshot {
                window_id: 1,
                cols: 80,
                rows: 24,
                bytes: b"\x1b[H\x1b[Jhi".to_vec(),
            },
            DaemonMsg::Output {
                window_id: 1,
                bytes: vec![0, 0xff],
            },
            DaemonMsg::Ack {
                request: "kill".into(),
            },
            DaemonMsg::Error {
                request: "hello".into(),
                message: "mismatch".into(),
            },
            DaemonMsg::Bye {
                reason: "shutdown".into(),
            },
        ];
        for message in daemon_messages {
            let back: DaemonMsg =
                rmp_serde::from_slice(&rmp_serde::to_vec_named(&message).unwrap()).unwrap();
            assert_eq!(back, message);
        }
    }
}
