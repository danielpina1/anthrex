use crate::types::{ClientKind, WindowInfo, WindowSpec};
use serde::{Deserialize, Serialize};

/// Who produced a hook event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum HookSource {
    Claude,
    CodexNotify,
}

/// Client → daemon.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ClientMsg {
    Hello { proto_version: u32, client: ClientKind },
    ListWindows,
    CreateWindow { spec: WindowSpec, cols: u16, rows: u16 },
    Subscribe { window_id: u32, cols: u16, rows: u16 },
    Unsubscribe,
    Input {
        window_id: u32,
        #[serde(with = "serde_bytes")]
        bytes: Vec<u8>,
    },
    Resize { window_id: u32, cols: u16, rows: u16 },
    Kill { window_id: u32 },
    Remove { window_id: u32, remove_worktree: bool, force: bool },
    Restart { window_id: u32 },
    Rename { window_id: u32, name: String },
    HookEvent { window_id: u32, source: HookSource, payload: serde_json::Value },
    Shutdown,
}

/// Daemon → client.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum DaemonMsg {
    Welcome { daemon_version: String, windows: Vec<WindowInfo> },
    WindowsChanged { windows: Vec<WindowInfo> },
    Created { window_id: u32 },
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
    Ack { request: String },
    Error { request: String, message: String },
    Bye { reason: String },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ClientKind, Runtime, WindowSpec};

    #[test]
    fn input_bytes_survive_messagepack() {
        let msg = ClientMsg::Input { window_id: 3, bytes: vec![0x1b, b'[', b'A', 0xff] };
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
    fn every_daemon_message_round_trips() {
        let spec = WindowSpec {
            name: None,
            runtime: Runtime::Shell,
            cwd: "/tmp".into(),
            worktree_branch: None,
            model: None,
            initial_prompt: None,
        };
        let msgs = vec![
            ClientMsg::Hello { proto_version: 1, client: ClientKind::Tui },
            ClientMsg::CreateWindow { spec, cols: 80, rows: 24 },
            ClientMsg::Subscribe { window_id: 1, cols: 80, rows: 24 },
        ];
        for m in msgs {
            let back: ClientMsg = rmp_serde::from_slice(&rmp_serde::to_vec_named(&m).unwrap()).unwrap();
            assert_eq!(back, m);
        }
        let d = DaemonMsg::Snapshot { window_id: 1, cols: 80, rows: 24, bytes: b"\x1b[H\x1b[Jhi".to_vec() };
        let back: DaemonMsg = rmp_serde::from_slice(&rmp_serde::to_vec_named(&d).unwrap()).unwrap();
        assert_eq!(back, d);
    }
}
