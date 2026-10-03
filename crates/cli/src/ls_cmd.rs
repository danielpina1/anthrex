//! `anthrex ls`: the window list, as a table or as pretty JSON, and after the table each
//! project's idle orchestrator (milestone 9.3, KG §7).

use std::path::Path;
use std::time::Duration;

use proto::{IdleOrchestrator, WindowInfo, safe_text};

use crate::client;
use crate::run_cmd::{list_runs, printable};

pub async fn ls(socket: &Path, json: bool) -> anyhow::Result<()> {
    let (out, note) = output(socket, json, LS_RUNS_TIMEOUT).await?;
    print!("{out}");
    if let Some(note) = note {
        eprintln!("{note}");
    }
    Ok(())
}

/// How long `ls` waits for the runs list after its handshake. The daemon answers `List`
/// from its in-memory snapshot; a wait near this means the run service is busy or stuck.
pub const LS_RUNS_TIMEOUT: Duration = Duration::from_secs(10);

/// What `ls` prints to stdout, and the note it prints to stderr when the runs list did
/// not come within `wait`.
async fn output(
    socket: &Path,
    json: bool,
    wait: Duration,
) -> anyhow::Result<(String, Option<String>)> {
    let mut c = client::CliClient::connect(socket).await?;
    let windows = std::mem::take(&mut c.windows);
    // `--json` is the window list alone, as before: it asks for no runs.
    if json {
        return Ok((text(&windows, &[], json)?, None));
    }
    // The table does not depend on the run service: a refused or late runs list leaves
    // the idle lines out and says why, drawn safe (decision 33).
    let listed = tokio::time::timeout(wait, list_runs(c))
        .await
        .unwrap_or_else(|_| Err(anyhow::anyhow!("timed out waiting for the daemon")));
    let (idle, note) = match listed {
        Ok(runs) => (runs.idle_orchestrators, None),
        Err(e) => {
            let why = printable(&e.to_string());
            (
                Vec::new(),
                Some(format!("anthrex: no idle orchestrators listed: {why}")),
            )
        }
    };
    Ok((text(&windows, &idle, json)?, note))
}

/// What `ls` prints: the windows as pretty JSON, or the window table and then one line
/// per idle orchestrator whose session has not ended (design decision 31),
/// `<chain>  orchestrator  idle after <h4>`, each name on one line without hidden
/// carriers (decision 33). The line names no outcome, so a delivered `pr` run's (D17)
/// reads as any other.
fn text(windows: &[WindowInfo], idle: &[IdleOrchestrator], json: bool) -> anyhow::Result<String> {
    if json {
        return Ok(format!("{}\n", serde_json::to_string_pretty(windows)?));
    }
    let mut out = client::format_table(windows);
    for chain in idle.iter().filter(|c| !c.fresh) {
        let after = safe_text::one_line(&chain.after_run);
        let h4: String = {
            let chars: Vec<char> = after.chars().collect();
            chars[chars.len().saturating_sub(4)..].iter().collect()
        };
        let id = safe_text::one_line(&chain.chain);
        out.push_str(&format!("{id}  orchestrator  idle after {h4}\n"));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use proto::{IdleOrchestrator, RunState, Runtime, Status, WindowInfo};

    use std::time::Duration;

    use proto::{ClientMsg, DaemonMsg, RunRequest};
    use proto::{read_frame, write_frame};

    use super::{LS_RUNS_TIMEOUT, output, text};
    use crate::client::format_table;
    use crate::run_cmd::printable;

    fn win(id: u32, name: &str) -> WindowInfo {
        WindowInfo {
            id,
            name: name.into(),
            runtime: Runtime::Claude,
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
            kind: proto::WindowKind::Pty,
            run: None,
            signals_seen: false,
            placeholder: false,
        }
    }

    fn idle(chain: &str, after_run: &str, outcome: RunState, fresh: bool) -> IdleOrchestrator {
        IdleOrchestrator {
            chain: chain.into(),
            project: "/home/me/repo".into(),
            after_run: after_run.into(),
            outcome,
            runtime: Runtime::Claude,
            model: "claude-opus-5".into(),
            window_id: (!fresh).then_some(4),
            fresh,
            runs: 2,
        }
    }

    /// KG §7 and design decision 31: after the window table, one line per idle orchestrator
    /// whose session has not ended, exactly `<chain>  orchestrator  idle after <h4>`; an
    /// ended chain (`fresh`) has none. The line names no outcome, so a delivered `pr` run
    /// (D17: its idle orchestrator's outcome is `Complete`) reads like an accepted one.
    /// Chain ids and run ids are drawn on one line without hidden carriers. `--json` is the
    /// window list exactly as before.
    #[test]
    fn ls_prints_one_line_per_idle_orchestrator() {
        let windows = vec![win(4, "3f9a/orchestrator"), win(7, "api")];
        let table = format_table(&windows);
        let chains = vec![
            idle("o-3f9a", "add-reset-3f9a", RunState::Accepted, false),
            idle("o-77b0", "fix-login-77b0", RunState::Complete, false),
            idle("o-1c2d", "old-goal-1c2d", RunState::Discarded, true),
            idle(
                "o-9e\u{202E}8f",
                "x\u{200D}y-9e8f",
                RunState::Discarded,
                false,
            ),
        ];
        assert_eq!(
            text(&windows, &chains, false).unwrap(),
            format!(
                "{table}o-3f9a  orchestrator  idle after 3f9a\n\
                 o-77b0  orchestrator  idle after 77b0\n\
                 o-9e8f  orchestrator  idle after 9e8f\n"
            )
        );
        assert_eq!(text(&windows, &[], false).unwrap(), table);

        let json = serde_json::to_string_pretty(&windows).unwrap();
        assert_eq!(text(&windows, &chains, true).unwrap(), format!("{json}\n"));
    }

    /// A daemon on a real socket that greets with two windows, reads `ls`'s runs
    /// request, and answers `reply`, or nothing for 30 s.
    fn fake_daemon(socket: &std::path::Path, reply: Option<DaemonMsg>) -> Vec<WindowInfo> {
        let windows = vec![win(4, "api"), win(7, "web")];
        let listener = tokio::net::UnixListener::bind(socket).unwrap();
        let greeting = windows.clone();
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let (mut rd, mut wr) = stream.into_split();
            let hello = read_frame::<_, ClientMsg>(&mut rd).await.unwrap().unwrap();
            assert!(matches!(hello, ClientMsg::Hello { .. }));
            let welcome = DaemonMsg::Welcome {
                daemon_version: "test".into(),
                windows: greeting,
            };
            write_frame(&mut wr, &welcome).await.unwrap();
            let request = read_frame::<_, ClientMsg>(&mut rd).await.unwrap().unwrap();
            assert_eq!(request, ClientMsg::Run(RunRequest::List));
            match reply {
                Some(reply) => write_frame(&mut wr, &reply).await.unwrap(),
                None => tokio::time::sleep(Duration::from_secs(30)).await,
            }
        });
        windows
    }

    /// Final fix wave (task 8 m2): a refused runs list still prints the window table,
    /// and the daemon's error goes to stderr drawn through `printable`.
    #[tokio::test]
    async fn ls_prints_its_table_when_the_runs_list_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("d.sock");
        let refused = DaemonMsg::Error {
            request: "run".into(),
            message: "the run service is down\u{1b}[2J".into(),
        };
        let windows = fake_daemon(&socket, Some(refused));
        let bound = LS_RUNS_TIMEOUT + proto::HANDSHAKE_TIMEOUT + Duration::from_secs(2);
        let (out, note) = tokio::time::timeout(bound, output(&socket, false, LS_RUNS_TIMEOUT))
            .await
            .expect("answered")
            .expect("the table prints");
        assert_eq!(out, format_table(&windows));
        let note = note.expect("a note");
        assert_eq!(
            note,
            format!(
                "anthrex: no idle orchestrators listed: {}",
                printable("the run service is down\u{1b}[2J")
            )
        );
        assert!(!note.contains('\u{1b}'), "{note:?}");
    }

    /// Final fix wave (task 8 m2): a runs list that never comes is waited for `wait`
    /// only, then the table prints with a note.
    #[tokio::test]
    async fn ls_bounds_its_wait_for_the_runs_list() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("d.sock");
        let windows = fake_daemon(&socket, None);
        let wait = Duration::from_millis(300);
        let (out, note) =
            tokio::time::timeout(Duration::from_secs(5), output(&socket, false, wait))
                .await
                .expect("bounded")
                .expect("the table prints");
        assert_eq!(out, format_table(&windows));
        assert_eq!(
            note.as_deref(),
            Some("anthrex: no idle orchestrators listed: timed out waiting for the daemon")
        );
    }
}
