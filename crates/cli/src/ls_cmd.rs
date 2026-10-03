//! `anthrex ls`: the window list, as a table or as pretty JSON, and after the table each
//! project's idle orchestrator (milestone 9.3, KG §7).

use std::path::Path;

use proto::{IdleOrchestrator, WindowInfo, safe_text};

use crate::client;
use crate::run_cmd::list_runs;

pub async fn ls(socket: &Path, json: bool) -> anyhow::Result<()> {
    let mut c = client::CliClient::connect(socket).await?;
    let windows = std::mem::take(&mut c.windows);
    // `--json` is the window list alone, as before: it asks for no runs.
    let idle = if json {
        Vec::new()
    } else {
        list_runs(c).await?.idle_orchestrators
    };
    print!("{}", text(&windows, &idle, json)?);
    Ok(())
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

    use super::text;
    use crate::client::format_table;

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
}
