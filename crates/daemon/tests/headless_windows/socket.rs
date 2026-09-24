//! Decision 49 over a real socket: a client can read a headless window but never drive
//! it, and no client message can create one.

use super::support::headless::*;
use super::support::{Client, start_daemon_configured};
use proto::{ClientMsg, DaemonMsg, PROTO_VERSION, Runtime, WindowKind, WindowSpec};

fn alive(pid: u32) -> bool {
    // SAFETY: signal 0 only checks that the pid exists.
    unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
}

#[tokio::test]
async fn client_control_of_a_headless_window_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let claude = script(dir.path(), "claude", "exec sleep 30");
    let d =
        start_daemon_configured(false, |c| c.claude_bin = claude.to_str().unwrap().into()).await;
    let info = create(&d.manager, "w", spec(Runtime::Claude, dir.path()), "hi").await;
    let id = info.id;
    wait_until("the session process", || {
        d.manager.child_pid(id).unwrap().is_some()
    })
    .await;
    let pid = d.manager.child_pid(id).unwrap().unwrap();
    let (mut client, _) = Client::connect(&d, PROTO_VERSION).await;

    let engine_only = format!(
        "window {id} is a headless session of run {RUN_ID}; only the engine drives it. Use anthrex run cancel to stop it"
    );
    let cases = [
        (
            ClientMsg::Subscribe {
                window_id: id,
                cols: 80,
                rows: 24,
            },
            "subscribe",
            format!("window {id} is a headless session; open its conversation with C-b m"),
        ),
        (
            ClientMsg::Input {
                window_id: id,
                bytes: b"rm -rf /\n".to_vec(),
            },
            "input",
            engine_only.clone(),
        ),
        (
            ClientMsg::Kill { window_id: id },
            "kill",
            engine_only.clone(),
        ),
        (
            ClientMsg::Remove {
                window_id: id,
                remove_worktree: false,
                force: false,
            },
            "remove",
            engine_only.clone(),
        ),
        (
            ClientMsg::Remove {
                window_id: id,
                remove_worktree: true,
                force: false,
            },
            "remove",
            engine_only.clone(),
        ),
        (
            ClientMsg::Restart { window_id: id },
            "restart",
            engine_only.clone(),
        ),
    ];
    for (msg, request, message) in cases {
        client.send(msg.clone()).await;
        let reply = client
            .recv_until(|m| matches!(m, DaemonMsg::Error { .. }))
            .await;
        assert_eq!(
            reply,
            DaemonMsg::Error {
                request: request.into(),
                message: message.clone(),
            },
            "{msg:?}"
        );
        assert!(alive(pid), "{msg:?} left the process running");
        assert_eq!(d.manager.child_pid(id).unwrap(), Some(pid));
    }

    // `Resize` is accepted and ignored: no reply, so a `ListWindows` after it is the
    // next thing answered.
    client
        .send(ClientMsg::Resize {
            window_id: id,
            cols: 100,
            rows: 30,
        })
        .await;
    client.send(ClientMsg::ListWindows).await;
    let listed = client
        .recv_until(|m| !matches!(m, DaemonMsg::Git { .. }))
        .await;
    let DaemonMsg::WindowsChanged { windows } = listed else {
        panic!("the Resize got a reply: {listed:?}");
    };
    let window = windows.iter().find(|w| w.id == id).expect("listed");
    assert_eq!(window.kind, WindowKind::Headless);
    assert_eq!(window.run, Some(run_ref()));
    assert!(alive(pid));
    d.manager.remove(id).unwrap();
}

#[tokio::test]
async fn a_client_cannot_create_a_headless_window() {
    let dir = tempfile::tempdir().unwrap();
    let stub = script(dir.path(), "agent", "exec sleep 30");
    let d = start_daemon_configured(false, |c| {
        c.claude_bin = stub.to_str().unwrap().into();
        c.codex_bin = stub.to_str().unwrap().into();
    })
    .await;
    let (mut client, _) = Client::connect(&d, PROTO_VERSION).await;
    for runtime in [Runtime::Shell, Runtime::Claude, Runtime::Codex] {
        let spec = WindowSpec {
            name: Some(format!("c-{}", runtime.label())),
            runtime,
            cwd: std::env::temp_dir(),
            worktree_branch: None,
            model: None,
            initial_prompt: None,
        };
        // Exhaustive on purpose: a field added to `WindowSpec` fails to compile here,
        // and whoever adds it must decide whether it could ask for a headless window.
        let WindowSpec {
            name,
            runtime: spec_runtime,
            cwd,
            worktree_branch,
            model,
            initial_prompt,
        } = spec.clone();
        assert!(name.is_some() && cwd.is_dir() && spec_runtime == runtime);
        assert!(worktree_branch.is_none() && model.is_none() && initial_prompt.is_none());

        client
            .send(ClientMsg::CreateWindow {
                spec,
                cols: 80,
                rows: 24,
            })
            .await;
        let created = client
            .recv_until(|m| matches!(m, DaemonMsg::Created { .. } | DaemonMsg::Error { .. }))
            .await;
        let DaemonMsg::Created { window_id } = created else {
            panic!("{created:?}");
        };
        let window = find(&d.manager, window_id);
        assert_eq!(window.kind, WindowKind::Pty, "{runtime:?}");
        assert_eq!(window.run, None, "{runtime:?}");
        assert_eq!(d.manager.headless_spec(window_id), None);
    }
}
