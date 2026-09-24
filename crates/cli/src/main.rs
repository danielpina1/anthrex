mod client;
mod hook;
mod mcp_cmd;
mod run_cmd;
mod tree_cmd;

use clap::{Parser, Subcommand, ValueEnum};
use proto::{ClientMsg, DaemonMsg, Runtime, WindowSpec};
use std::path::PathBuf;
use std::time::Duration;
use tokio::net::UnixStream;

#[derive(Parser)]
#[command(
    name = "anthrex",
    version,
    about = "A terminal multiplexer for coding agents"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,

    /// Directory new agents start in (default: the current directory)
    #[arg(long, global = true)]
    dir: Option<PathBuf>,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Attach to the daemon (the default when no command is given)
    Attach {
        /// Window id or name to focus
        target: Option<String>,
    },
    /// Manage the background daemon that owns the agents
    Daemon {
        #[command(subcommand)]
        action: DaemonAction,
    },
    /// Create a window and print its id
    New {
        /// Defaults to the config's `default_runtime` (decision 7)
        #[arg(long, value_enum)]
        runtime: Option<RuntimeArg>,
        #[arg(long)]
        name: Option<String>,
        /// Create a git worktree on this branch and start the window in it
        #[arg(long)]
        worktree: Option<String>,
        #[arg(long)]
        model: Option<String>,
        /// Initial prompt for claude or codex
        #[arg(long)]
        prompt: Option<String>,
    },
    /// List windows
    Ls {
        /// Print the window list as pretty JSON
        #[arg(long)]
        json: bool,
    },
    /// Print the project tree
    Tree {
        /// Show the project containing this directory
        #[arg(long)]
        project: Option<PathBuf>,
        /// Print the project tree as pretty JSON
        #[arg(long)]
        json: bool,
    },
    /// Kill a window's process group (SIGHUP, then SIGTERM, then SIGKILL)
    Kill { target: String },
    /// Kill and forget a window
    Rm {
        target: String,
        /// Also remove the window's git worktree; the branch is kept
        #[arg(long)]
        worktree: bool,
        /// Remove the worktree even if it has uncommitted or untracked changes
        #[arg(long, requires = "worktree")]
        force: bool,
    },
    /// Rename a window
    Rename { target: String, name: String },
    /// Restart a window, resuming its session when known
    Restart { target: String },
    /// Start, watch and finish orchestrated runs of a plan
    Run(run_cmd::RunArgs),
    /// Serve the anthrex MCP tools on stdin/stdout for one headless run agent
    /// (decision 4). Started by the daemon, never by hand; stdout is JSON-RPC only.
    #[command(hide = true)]
    Mcp(mcp_cmd::McpArgs),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
enum RuntimeArg {
    Claude,
    Codex,
    Shell,
}

impl From<RuntimeArg> for Runtime {
    fn from(r: RuntimeArg) -> Self {
        match r {
            RuntimeArg::Claude => Runtime::Claude,
            RuntimeArg::Codex => Runtime::Codex,
            RuntimeArg::Shell => Runtime::Shell,
        }
    }
}

fn expect_ack(reply: DaemonMsg) -> anyhow::Result<()> {
    match reply {
        DaemonMsg::Ack { .. } => Ok(()),
        DaemonMsg::Error { message, .. } => anyhow::bail!(message),
        other => anyhow::bail!("unexpected reply: {other:?}"),
    }
}

/// The hint printed after a dirty refusal, decision 39. `target` is echoed back exactly
/// as the user typed it, so the two commands it names are ones they can paste.
///
/// "discard it anyway" rather than decision 39's original "discard the changes": the
/// daemon's message above this line may have refused a paused rebase, a bisect or
/// unreachable commits, none of which are changes, and a hint that renamed them would
/// undo the work the message does.
fn dirty_hint(target: &str) -> String {
    format!(
        "run 'anthrex rm {target} --worktree --force' to discard it anyway, or 'anthrex rm {target}' to keep the worktree"
    )
}

/// Resolves the working directory for a new window: the given `--dir`, or the current
/// directory when none was given, canonicalized either way.
fn resolve_dir(dir: Option<PathBuf>) -> anyhow::Result<PathBuf> {
    let dir = match dir {
        Some(d) => d,
        None => std::env::current_dir()?,
    };
    dir.canonicalize()
        .map_err(|e| anyhow::anyhow!("cannot resolve directory {}: {e}", dir.display()))
}

#[derive(Subcommand, Debug)]
enum DaemonAction {
    /// Start the daemon (detached unless --foreground)
    Start {
        #[arg(long)]
        foreground: bool,
    },
    /// Stop the daemon and every agent it owns
    Stop,
    /// Show whether a daemon is running
    Status,
}

fn main() -> anyhow::Result<()> {
    let started = std::time::Instant::now();
    if std::env::args_os().nth(1).as_deref() == Some(std::ffi::OsStr::new("hook")) {
        hook::run(std::env::args_os().skip(2).collect(), started);
        std::process::exit(0);
    }
    run_cli()
}

#[tokio::main]
async fn run_cli() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let socket: PathBuf = proto::paths::socket_path();
    match cli.command {
        // Pitfall 17: nothing but JSON-RPC may reach stdout from here on, so this arm
        // prints nothing and installs no tracing subscriber.
        Some(Command::Mcp(args)) => mcp::serve_stdio(args.into_options(socket)).await,
        None => attach(socket, resolve_dir(cli.dir)?, None).await,
        Some(Command::Run(args)) => run_cmd::main(args, socket, cli.dir).await,
        Some(Command::Attach { target }) => attach(socket, resolve_dir(cli.dir)?, target).await,
        Some(Command::Daemon { action }) => daemon_command(action, socket).await,
        Some(Command::New {
            runtime,
            name,
            worktree,
            model,
            prompt,
        }) => {
            let dir = resolve_dir(cli.dir)?;
            // Decision 7: `new` is the only one-shot command that reads the config, to
            // fall back to `default_runtime` when `--runtime` was not given. Every
            // problem it finds is printed to stderr, never stdout — stdout is reserved
            // for the created window's id, which downstream scripts parse.
            let (loaded_config, config_problems) = config::load(&proto::paths::config_path());
            for problem in &config_problems {
                eprintln!("anthrex: config: {}: {}", problem.key, problem.message);
            }
            let runtime: Runtime = match runtime {
                Some(r) => r.into(),
                None => loaded_config.default_runtime,
            };
            tui::spawn::ensure_daemon(&std::env::current_exe()?, &socket).await?;
            let mut c = client::CliClient::connect(&socket).await?;
            // A worktree create can make the daemon run git (decision 3's 30 s
            // operation deadline), so it needs the longer budget; a plain create is
            // bounded only by project detection (decision 38).
            let reply_timeout = if worktree.is_some() {
                client::WORKTREE_REQUEST_TIMEOUT
            } else {
                client::CREATE_WINDOW_REPLY_TIMEOUT
            };
            let spec = WindowSpec {
                name,
                runtime,
                cwd: dir,
                worktree_branch: worktree,
                model,
                initial_prompt: prompt,
            };
            let (cols, rows) = crossterm::terminal::size().unwrap_or((80, 24));
            match c
                .request_with_timeout(ClientMsg::CreateWindow { spec, cols, rows }, reply_timeout)
                .await?
            {
                DaemonMsg::Created { window_id } => {
                    println!("{window_id}");
                    Ok(())
                }
                DaemonMsg::Error { message, .. } => anyhow::bail!(message),
                other => anyhow::bail!("unexpected reply: {other:?}"),
            }
        }
        Some(Command::Ls { json }) => {
            let c = client::CliClient::connect(&socket).await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&c.windows)?);
            } else {
                print!("{}", client::format_table(&c.windows));
            }
            Ok(())
        }
        Some(Command::Tree { project, json }) => {
            let c = client::CliClient::connect(&socket).await?;
            let requested = project.map(|path| resolve_dir(Some(path))).transpose()?;
            let normalized = match requested.as_ref() {
                Some(path) => Some(daemon::project::resolve_roots(path.clone()).await.project),
                None => None,
            };
            let project = requested
                .as_deref()
                .zip(normalized.as_deref())
                .map(|(requested, normalized)| tree_cmd::ProjectQuery::new(requested, normalized));
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&tree_cmd::tree_json(&c.windows, project))?
                );
            } else {
                print!("{}", tree_cmd::tree_text(&c.windows, project));
            }
            Ok(())
        }
        Some(Command::Kill { target }) => {
            let mut c = client::CliClient::connect(&socket).await?;
            let id = client::resolve_target(&c.windows, &target)?;
            expect_ack(c.request(ClientMsg::Kill { window_id: id }).await?)
        }
        Some(Command::Rename { target, name }) => {
            let mut c = client::CliClient::connect(&socket).await?;
            let id = client::resolve_target(&c.windows, &target)?;
            expect_ack(
                c.request(ClientMsg::Rename {
                    window_id: id,
                    name,
                })
                .await?,
            )
        }
        Some(Command::Restart { target }) => {
            let mut c = client::CliClient::connect(&socket).await?;
            let id = client::resolve_target(&c.windows, &target)?;
            expect_ack(
                c.request_with_timeout(
                    ClientMsg::Restart { window_id: id },
                    client::RESTART_REQUEST_TIMEOUT,
                )
                .await?,
            )
        }
        Some(Command::Rm {
            target,
            worktree,
            force,
        }) => {
            let mut c = client::CliClient::connect(&socket).await?;
            let id = client::resolve_target(&c.windows, &target)?;
            // Kept only to word the "kept worktree" notice below; the removal itself
            // is driven by `id`.
            let removed = c.windows.iter().find(|w| w.id == id).cloned();
            let remove_msg = ClientMsg::Remove {
                window_id: id,
                remove_worktree: worktree,
                force,
            };
            // A worktree removal can make the daemon run git and wait out the agent's
            // kill grace before it does (decision 38); a plain removal never touches
            // git and keeps the short default.
            let reply = if worktree {
                c.request_with_timeout(remove_msg, client::WORKTREE_REQUEST_TIMEOUT)
                    .await?
            } else {
                c.request(remove_msg).await?
            };
            match reply {
                DaemonMsg::Ack { .. } => {
                    // Decision 39: the agent is already gone by the time this prints,
                    // so it reports what was kept, not what is still running.
                    if !worktree
                        && let Some(w) = &removed
                        && let Some(branch) = &w.branch
                    {
                        eprintln!("kept worktree {} on branch {branch}", w.cwd.display());
                    }
                    Ok(())
                }
                // The id it carries is not needed here — a one-shot `rm` has exactly one
                // removal outstanding and `target` is what the user typed — but the
                // message is still matched by its own variant rather than by a `request`
                // string, so the CLI and the TUI recognise the refusal the same way.
                DaemonMsg::RemoveDirty { message, .. } => {
                    anyhow::bail!("{message}\n{}", dirty_hint(&target))
                }
                DaemonMsg::Error { message, .. } => anyhow::bail!(message),
                other => anyhow::bail!("unexpected reply: {other:?}"),
            }
        }
    }
}

/// Decision A5's locale precedence: the first of `lc_all`, `lc_ctype`, `lang` that is
/// `Some` and non-empty, in that order. Pure — `attach` is the only caller, and it is the
/// one that reads `std::env::var` (`AGENTS.md` hard rule 5 keeps `crates/tui` I/O-free;
/// this crate keeps the read and the precedence logic separate so the precedence itself
/// is unit-testable without touching the environment).
fn resolve_locale(
    lc_all: Option<&str>,
    lc_ctype: Option<&str>,
    lang: Option<&str>,
) -> Option<String> {
    [lc_all, lc_ctype, lang]
        .into_iter()
        .find_map(|v| v.filter(|s| !s.is_empty()).map(str::to_string))
}

async fn attach(socket: PathBuf, dir: PathBuf, target: Option<String>) -> anyhow::Result<()> {
    tui::spawn::ensure_daemon(&std::env::current_exe()?, &socket).await?;
    // The config is loaded exactly once, here, and turned into the client's resolved
    // `UiSettings` before `tui::run` starts: nothing under `crates/tui/src/app/` or
    // `crates/tui/src/ui/` does I/O (task M6.9's layering rule), so the CLI is the only
    // place `config::load` can run for the TUI. Every problem it found is formatted
    // (decision 7's `<key>: <message> (using <default>)`, `Problem`'s own `Display`)
    // and shown once, in a dismissable notice, instead of being lost to a log no one
    // watching the TUI would see.
    let (loaded_config, config_problems) = config::load(&proto::paths::config_path());
    // Decision 32: `C-b r` may need to start the daemon the same way this cold
    // attach just did, so it gets the same executable path `ensure_daemon` above
    // used.
    let daemon_exe = std::env::current_exe()?;
    // Decision A5: ASCII badges are chosen by `conversation.badges.force_ascii`
    // (already applied by `from_config`) or by the first set, non-empty value of
    // `LC_ALL`, `LC_CTYPE`, `LANG` not naming UTF-8. Reading the environment happens
    // only here, never in `crates/tui/src/ui/badge.rs` (`AGENTS.md` hard rule 5); the
    // precedence itself is `resolve_locale`, tested below without touching the
    // environment.
    let lc_all = std::env::var("LC_ALL").ok();
    let lc_ctype = std::env::var("LC_CTYPE").ok();
    let lang = std::env::var("LANG").ok();
    let locale = resolve_locale(lc_all.as_deref(), lc_ctype.as_deref(), lang.as_deref());
    let settings =
        tui::settings::UiSettings::from_config(&loaded_config).with_locale(locale.as_deref());
    tui::run(tui::TuiOptions {
        socket_path: socket,
        default_dir: dir,
        focus: target,
        settings,
        config_problems: config_problems.iter().map(ToString::to_string).collect(),
        daemon_exe: Some(daemon_exe),
    })
    .await
}

async fn daemon_command(action: DaemonAction, socket: PathBuf) -> anyhow::Result<()> {
    match action {
        DaemonAction::Start { foreground: true } => {
            daemon::run(daemon::DaemonOptions {
                socket_path: socket,
                data_dir: proto::paths::data_dir(),
                config_path: proto::paths::config_path(),
                lock_wait: daemon::LOCK_WAIT,
            })
            .await
        }
        DaemonAction::Start { foreground: false } => {
            tui::spawn::ensure_daemon(&std::env::current_exe()?, &socket).await?;
            println!("daemon running on {}", socket.display());
            Ok(())
        }
        DaemonAction::Stop => {
            let mut c = client::CliClient::connect(&socket)
                .await
                .map_err(|_| anyhow::anyhow!("no daemon is running"))?;
            c.send(ClientMsg::Shutdown).await?;
            c.wait_close().await;
            // Decision 27: only report the daemon stopped once its lifetime lock is
            // free, so an `anthrex` right after this never races a daemon that is still
            // tearing down.
            let lock_path = proto::paths::lock_path();
            let released = tokio::task::spawn_blocking(move || {
                daemon::lockfile::wait_released(&lock_path, Duration::from_secs(10))
            })
            .await?;
            if !released {
                anyhow::bail!(
                    "the daemon did not exit within 10 s; see {}",
                    proto::paths::log_path().display()
                );
            }
            println!("daemon stopped");
            Ok(())
        }
        DaemonAction::Status => match UnixStream::connect(&socket).await {
            Ok(_) => match client::CliClient::connect(&socket).await {
                Ok(c) => {
                    println!(
                        "running: version {}, {} window(s), socket {}",
                        c.daemon_version,
                        c.windows.len(),
                        socket.display()
                    );
                    Ok(())
                }
                Err(e) => {
                    println!("running but incompatible: {e}");
                    Ok(())
                }
            },
            Err(_) => {
                println!("not running (socket {})", socket.display());
                Ok(())
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::{Cli, Command, dirty_hint, resolve_locale};
    use clap::Parser;

    /// Decision A5's precedence, pairwise-distinct per case per the review's Finding 1:
    /// all three set (LC_ALL wins over both distinct others); LC_ALL empty with LC_CTYPE
    /// set (LC_CTYPE wins, not the empty LC_ALL); LC_ALL and LC_CTYPE both unset/empty
    /// with LANG set (LANG wins); everything empty or unset (None).
    #[test]
    fn resolve_locale_follows_lc_all_then_lc_ctype_then_lang() {
        assert_eq!(
            resolve_locale(Some("en_US.UTF-8"), Some("C"), Some("POSIX")),
            Some("en_US.UTF-8".to_string()),
            "LC_ALL must win when all three are set"
        );
        assert_eq!(
            resolve_locale(Some(""), Some("de_DE.UTF-8"), Some("POSIX")),
            Some("de_DE.UTF-8".to_string()),
            "an empty LC_ALL must not beat a set LC_CTYPE"
        );
        assert_eq!(
            resolve_locale(None, Some(""), Some("ja_JP.UTF-8")),
            Some("ja_JP.UTF-8".to_string()),
            "LANG must win once LC_ALL and LC_CTYPE are unset/empty"
        );
        assert_eq!(
            resolve_locale(Some(""), None, Some("")),
            None,
            "every variable empty or unset must resolve to None"
        );
    }

    #[test]
    fn rm_force_requires_worktree() {
        assert!(
            Cli::try_parse_from(["anthrex", "rm", "x", "--force"]).is_err(),
            "--force without --worktree must be a parse error, not a daemon round trip"
        );

        match Cli::try_parse_from(["anthrex", "rm", "x", "--worktree", "--force"])
            .expect("--worktree --force parses")
            .command
        {
            Some(Command::Rm {
                target,
                worktree,
                force,
            }) => {
                assert_eq!(target, "x");
                assert!(worktree);
                assert!(force);
            }
            other => panic!("expected Rm, got {other:?}"),
        }
    }

    #[test]
    fn new_accepts_a_worktree_branch() {
        match Cli::try_parse_from([
            "anthrex",
            "new",
            "--runtime",
            "shell",
            "--worktree",
            "feat/x",
        ])
        .expect("new --worktree <branch> parses")
        .command
        {
            Some(Command::New { worktree, .. }) => {
                assert_eq!(worktree, Some("feat/x".to_string()));
            }
            other => panic!("expected New, got {other:?}"),
        }
    }

    #[test]
    fn dirty_hint_names_both_commands() {
        assert_eq!(
            dirty_hint("api-worker"),
            "run 'anthrex rm api-worker --worktree --force' to discard it anyway, or \
             'anthrex rm api-worker' to keep the worktree"
        );
    }

    #[test]
    fn tree_accepts_optional_project_and_json_flags() {
        for args in [
            vec!["anthrex", "tree"],
            vec!["anthrex", "tree", "--project", "/r/shop/src"],
            vec!["anthrex", "tree", "--json"],
            vec!["anthrex", "tree", "--project", "/r/shop/src", "--json"],
        ] {
            assert!(Cli::try_parse_from(&args).is_ok(), "{args:?}");
        }
    }

    /// `rename` and `restart` parse into their new variants, and `new` with no
    /// `--runtime` parses with `runtime == None` — the flag now falls back to the
    /// config's `default_runtime` instead of defaulting to `RuntimeArg::Shell` at parse
    /// time.
    #[test]
    fn rename_and_restart_parse() {
        match Cli::try_parse_from(["anthrex", "rename", "3", "new name"])
            .expect("rename <target> <name> parses")
            .command
        {
            Some(Command::Rename { target, name }) => {
                assert_eq!(target, "3");
                assert_eq!(name, "new name");
            }
            other => panic!("expected Rename, got {other:?}"),
        }

        match Cli::try_parse_from(["anthrex", "restart", "api"])
            .expect("restart <target> parses")
            .command
        {
            Some(Command::Restart { target }) => assert_eq!(target, "api"),
            other => panic!("expected Restart, got {other:?}"),
        }

        match Cli::try_parse_from(["anthrex", "new"])
            .expect("new with no --runtime parses")
            .command
        {
            Some(Command::New { runtime, .. }) => assert_eq!(runtime, None),
            other => panic!("expected New, got {other:?}"),
        }
    }

    #[test]
    fn tree_help_describes_command_and_both_flags() {
        let error = Cli::try_parse_from(["anthrex", "tree", "--help"])
            .err()
            .expect("help exits instead of parsing a command");
        assert_eq!(error.kind(), clap::error::ErrorKind::DisplayHelp);
        let help = error.to_string();
        assert!(help.contains("Print the project tree"));
        assert!(help.contains("--project <PROJECT>"));
        assert!(help.contains("--json"));
    }
}
