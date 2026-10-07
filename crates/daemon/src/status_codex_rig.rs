//! Tests only: a `codex` stand-in shell script (never an agent) that draws what a control
//! file holds, and the screens it draws, shared by `status_codex_tests.rs` and the
//! driver's `wake_report_tests.rs` (milestone 9.5 decision 40).

use std::path::Path;
use std::time::{Duration, Instant};

use proto::{AgentRole, Effort, RunRef, Runtime, WindowSpec};

use crate::headless::McpTarget;
use crate::launch::role::RoleLaunch;
use crate::manager::WindowManager;

const FIXTURE: &str = include_str!("../tests/fixtures/screens/codex-question-footer.txt");

/// Slack on top of each derived bound (a process start, a few 20 ms polls).
pub(crate) const SLACK: Duration = Duration::from_secs(5);

/// The fixture's screen: every line but its `# ` notes.
pub(crate) fn fixture_screen() -> String {
    FIXTURE
        .lines()
        .filter(|l| !l.starts_with("# "))
        .collect::<Vec<_>>()
        .join("\n")
}

/// What the stand-in draws: the fixture with its footer counting `n` questions, or,
/// with `None`, a screen with no footer. Each starts with a line naming itself, which
/// [`shown`] waits for.
pub(crate) fn screen(questions: Option<u32>) -> String {
    match questions {
        None => "screen: no footer\n\n› Explain this codebase\n  gpt-6.1-sol high".into(),
        Some(1) => format!("screen: 1\n{}", fixture_screen()),
        Some(n) => format!(
            "screen: {n}\n{}",
            fixture_screen().replace("? 1 question", &format!("? {n} questions"))
        ),
    }
}

/// `codex` in `dir`: `--version` answered at once (`docs/timing-budgets.md`), no echo,
/// a `Ready` title (its signal, as a Codex session with no hooks gives), then the screen
/// redrawn each time the control file in `dir` changes: the one its environment's
/// `ANTHREX_TEST_CTL` names (the role's `env`), else `plain.ctl`. It exits once `dir`
/// is gone.
pub(crate) fn stand_in(dir: &Path) -> String {
    let codex = dir.join("codex");
    let script = format!(
        "#!/bin/sh\n[ \"$1\" = --version ] && {{ echo codex-cli 0.0.0; exit 0; }}\n\
         stty -echo 2>/dev/null\nprintf '\\033]0;Ready\\007'\n\
         ctl={dir}/\"${{ANTHREX_TEST_CTL:-plain.ctl}}\"\nlast=''\nwhile :; do\n  \
         [ -d \"{dir}\" ] || exit 0\n  c=$(cat \"$ctl\" 2>/dev/null)\n  \
         if [ \"$c\" != \"$last\" ]; then\n    printf '\\033[2J\\033[H%s' \"$c\"\n    \
         last=$c\n  fi\n  sleep 0.05\ndone\n",
        dir = dir.display()
    );
    testexec::write_executable(&codex, script);
    codex.to_str().unwrap().into()
}

/// Run `run_id`'s orchestrator role, as the engine launches it; `ctl` names the control
/// file its stand-in draws.
pub(crate) fn role(run_id: &str, ctl: &str) -> RoleLaunch {
    RoleLaunch {
        run_ref: RunRef {
            run_id: run_id.into(),
            task_id: None,
            role: AgentRole::Orchestrator,
            session: 1,
            lane: None,
        },
        mcp: McpTarget {
            role: AgentRole::Orchestrator,
            run_id: run_id.into(),
            task_id: None,
            scout_id: None,
            epic: None,
            chain: None,
            lane: None,
            agent_label: None,
        },
        instructions: "the orchestrator contract".into(),
        effort: Effort::High,
        claude_allowed_tools: Vec::new(),
        claude_disallowed_tools: Vec::new(),
        env: vec![("ANTHREX_TEST_CTL".into(), ctl.into())],
        remove_env: Vec::new(),
    }
}

pub(crate) fn spec(dir: &Path, name: &str) -> WindowSpec {
    WindowSpec {
        name: Some(name.into()),
        runtime: Runtime::Codex,
        cwd: dir.to_path_buf(),
        worktree_branch: None,
        model: None,
        initial_prompt: None,
    }
}

/// Writes `screen` to `dir/ctl` at once (a rename), for the stand-in to draw.
pub(crate) fn draw(dir: &Path, ctl: &str, screen: &str) {
    let path = dir.join(ctl);
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, screen).unwrap();
    std::fs::rename(tmp, path).unwrap();
}

/// Waits until `window`'s screen shows [`screen`]`(questions)`.
pub(crate) async fn shown(manager: &WindowManager, window: u32, questions: Option<u32>) {
    let token = match questions {
        None => "screen: no footer".to_string(),
        Some(n) => format!("screen: {n}"),
    };
    let deadline = Instant::now() + SLACK;
    loop {
        let (bytes, _, _) = manager.snapshot(window).unwrap();
        if String::from_utf8_lossy(&bytes).contains(&token) {
            return;
        }
        assert!(Instant::now() < deadline, "screen never {token:?}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}
