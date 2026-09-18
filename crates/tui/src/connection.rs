//! Async client for the daemon socket: one reader task, one writer task, channels in between.

use proto::{ClientKind, ClientMsg, DaemonMsg, PROTO_VERSION, WindowInfo, read_frame, write_frame};
use std::path::Path;
use tokio::net::UnixStream;
use tokio::sync::mpsc;

#[derive(Debug)]
pub struct Connection {
    tx: mpsc::Sender<ClientMsg>,
    rx: mpsc::Receiver<DaemonMsg>,
    reader: tokio::task::JoinHandle<()>,
    pub windows: Vec<WindowInfo>,
    pub daemon_version: String,
}

impl Connection {
    pub async fn connect(socket: &Path) -> anyhow::Result<Self> {
        let stream = UnixStream::connect(socket)
            .await
            .map_err(|e| anyhow::anyhow!("cannot connect to the daemon at {}: {e}", socket.display()))?;
        let (mut rd, mut wr) = stream.into_split();
        write_frame(&mut wr, &ClientMsg::Hello { proto_version: PROTO_VERSION, client: ClientKind::Tui }).await?;
        let (daemon_version, windows) = match read_frame::<_, DaemonMsg>(&mut rd).await? {
            Some(DaemonMsg::Welcome { daemon_version, windows }) => (daemon_version, windows),
            Some(DaemonMsg::Error { message, .. }) => anyhow::bail!(message),
            Some(other) => anyhow::bail!("unexpected handshake reply: {other:?}"),
            None => anyhow::bail!("the daemon closed the connection during the handshake"),
        };

        let (tx, mut out_rx) = mpsc::channel::<ClientMsg>(256);
        tokio::spawn(async move {
            while let Some(msg) = out_rx.recv().await {
                if write_frame(&mut wr, &msg).await.is_err() {
                    break;
                }
            }
        });

        let (in_tx, rx) = mpsc::channel::<DaemonMsg>(1024);
        let reader = tokio::spawn(async move {
            loop {
                match read_frame::<_, DaemonMsg>(&mut rd).await {
                    Ok(Some(msg)) => {
                        if in_tx.send(msg).await.is_err() {
                            break;
                        }
                    }
                    Ok(None) | Err(_) => break,
                }
            }
            // Dropping in_tx closes the channel; recv() then yields None.
        });

        Ok(Self { tx, rx, reader, windows, daemon_version })
    }

    /// Returns false if the connection is gone.
    pub async fn send(&self, msg: ClientMsg) -> bool {
        self.tx.send(msg).await.is_ok()
    }

    /// Returns None once the daemon has closed the connection.
    pub async fn recv(&mut self) -> Option<DaemonMsg> {
        self.rx.recv().await
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        // The reader task otherwise blocks in `read_frame` on its `OwnedReadHalf` until the
        // daemon sends something, keeping the task and the socket fd alive past this `Connection`.
        self.reader.abort();
    }
}
