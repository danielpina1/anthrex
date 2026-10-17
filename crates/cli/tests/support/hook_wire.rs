use std::time::Duration;

use proto::{ClientKind, ClientMsg, DaemonMsg};
use tokio::net::{UnixListener, UnixStream};

pub async fn accept(listener: &UnixListener) -> UnixStream {
    let (stream, _) = tokio::time::timeout(Duration::from_millis(1300), listener.accept())
        .await
        .expect("hook never connected")
        .unwrap();
    stream
}

pub async fn hello(stream: &mut UnixStream) {
    let msg = tokio::time::timeout(
        Duration::from_secs(1),
        proto::read_frame::<_, ClientMsg>(stream),
    )
    .await
    .expect("hook never sent Hello")
    .unwrap();
    assert_eq!(
        msg,
        Some(ClientMsg::Hello {
            proto_version: proto::PROTO_VERSION,
            client: ClientKind::Hook
        })
    );
}

pub async fn welcome(stream: &mut UnixStream) {
    proto::write_frame(
        stream,
        &DaemonMsg::Welcome {
            daemon_version: "test".into(),
            windows: vec![],
        },
    )
    .await
    .unwrap();
}
