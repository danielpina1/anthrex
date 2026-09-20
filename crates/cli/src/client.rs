//! A small blocking-style client for one-shot CLI commands.

use proto::{ClientKind, ClientMsg, DaemonMsg, PROTO_VERSION, WindowInfo, read_frame, write_frame};
use std::path::Path;
use std::time::Duration;
use tokio::net::UnixStream;
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
const CREATE_REPLY_ALLOWANCE: Duration = Duration::from_secs(2);
pub const CREATE_WINDOW_REPLY_TIMEOUT: Duration =
    daemon::project::DETECT_TIMEOUT.saturating_add(CREATE_REPLY_ALLOWANCE);

pub struct CliClient {
    rd: OwnedReadHalf,
    wr: OwnedWriteHalf,
    pub windows: Vec<WindowInfo>,
    pub daemon_version: String,
}

impl CliClient {
    pub async fn connect(socket: &Path) -> anyhow::Result<Self> {
        let stream = UnixStream::connect(socket)
            .await
            .map_err(|e| anyhow::anyhow!("cannot reach the daemon at {}: {e}", socket.display()))?;
        let (mut rd, mut wr) = stream.into_split();
        write_frame(
            &mut wr,
            &ClientMsg::Hello {
                proto_version: PROTO_VERSION,
                client: ClientKind::Cli,
            },
        )
        .await?;
        match read_frame::<_, DaemonMsg>(&mut rd).await? {
            Some(DaemonMsg::Welcome {
                daemon_version,
                windows,
            }) => Ok(Self {
                rd,
                wr,
                windows,
                daemon_version,
            }),
            Some(DaemonMsg::Error { message, .. }) => anyhow::bail!(message),
            Some(other) => anyhow::bail!("unexpected handshake reply: {other:?}"),
            None => anyhow::bail!("the daemon closed the connection during the handshake"),
        }
    }

    /// Writes one frame without waiting for a reply.
    pub async fn send(&mut self, msg: ClientMsg) -> anyhow::Result<()> {
        write_frame(&mut self.wr, &msg).await?;
        Ok(())
    }

    /// Sends one request and returns the first reply that is not a `WindowsChanged` broadcast.
    pub async fn request(&mut self, msg: ClientMsg) -> anyhow::Result<DaemonMsg> {
        self.request_with_timeout(msg, REQUEST_TIMEOUT).await
    }

    /// Sends one request with an explicit reply deadline.
    pub async fn request_with_timeout(
        &mut self,
        msg: ClientMsg,
        reply_timeout: Duration,
    ) -> anyhow::Result<DaemonMsg> {
        self.send(msg).await?;
        tokio::time::timeout(reply_timeout, async {
            loop {
                match read_frame::<_, DaemonMsg>(&mut self.rd).await? {
                    Some(DaemonMsg::WindowsChanged { .. }) => continue,
                    Some(reply) => return Ok(reply),
                    None => anyhow::bail!("the daemon closed the connection"),
                }
            }
        })
        .await
        .map_err(|_| anyhow::anyhow!("timed out waiting for the daemon"))?
    }

    /// Reads and discards frames until the daemon closes the connection, bounded by a 5 s timeout.
    pub async fn wait_close(&mut self) {
        let _ = tokio::time::timeout(Duration::from_secs(5), async {
            while let Ok(Some(_)) = read_frame::<_, DaemonMsg>(&mut self.rd).await {}
        })
        .await;
    }
}

/// `target` is a window id or a window name. Ids win when both match.
pub fn resolve_target(windows: &[WindowInfo], target: &str) -> anyhow::Result<u32> {
    if let Ok(id) = target.parse::<u32>()
        && windows.iter().any(|w| w.id == id)
    {
        return Ok(id);
    }
    if let Some(w) = windows.iter().find(|w| w.name == target) {
        return Ok(w.id);
    }
    anyhow::bail!("no window with id or name '{target}'")
}

pub fn format_table(windows: &[WindowInfo]) -> String {
    let name_w = windows
        .iter()
        .map(|w| w.name.len())
        .max()
        .unwrap_or(4)
        .max(4);
    let mut out = format!(
        "{:<4} {:<name_w$} {:<7} {:<10} DIR\n",
        "ID", "NAME", "RUNTIME", "STATUS"
    );
    for w in windows {
        out.push_str(&format!(
            "{:<4} {:<name_w$} {:<7} {:<10} {}\n",
            w.id,
            w.name,
            w.runtime.label(),
            w.status.label(),
            w.cwd.display()
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use proto::{Runtime, Status};

    fn win(id: u32, name: &str) -> WindowInfo {
        WindowInfo {
            id,
            name: name.into(),
            runtime: Runtime::Shell,
            cwd: "/home/me/repo".into(),
            project: "/home/me/repo".into(),
            worktree: None,
            branch: None,
            status: Status::Idle,
            tool: None,
            since_secs: 5,
            last_output_secs: 5,
            session_id: None,
            model: None,
            subagents: vec![],
            exit: None,
        }
    }

    #[test]
    fn resolve_by_id_then_by_name() {
        let ws = vec![win(1, "api"), win(2, "7"), win(7, "other")];
        assert_eq!(resolve_target(&ws, "1").unwrap(), 1);
        assert_eq!(resolve_target(&ws, "api").unwrap(), 1);
        assert_eq!(
            resolve_target(&ws, "7").unwrap(),
            7,
            "an id match wins even when another window's name is the same string"
        );
        assert!(
            resolve_target(&ws, "nope")
                .unwrap_err()
                .to_string()
                .contains("nope")
        );

        let ws_without_id_7 = vec![win(1, "api"), win(2, "7")];
        assert_eq!(
            resolve_target(&ws_without_id_7, "7").unwrap(),
            2,
            "a numeric name still resolves when no such id exists"
        );
    }

    #[test]
    fn table_has_header_and_one_row_per_window() {
        let out = format_table(&[win(1, "api"), win(2, "tests")]);
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), 3);
        assert!(lines[0].starts_with("ID"));
        assert!(
            lines[1].contains("api")
                && lines[1].contains("idle")
                && lines[1].contains("/home/me/repo")
        );
        assert!(lines[2].contains("tests"));
    }

    #[test]
    fn create_timeout_adds_allowance_without_changing_the_default() {
        assert_eq!(REQUEST_TIMEOUT, Duration::from_secs(5));
        assert_eq!(
            CREATE_WINDOW_REPLY_TIMEOUT,
            daemon::project::DETECT_TIMEOUT + CREATE_REPLY_ALLOWANCE,
        );
    }
}
