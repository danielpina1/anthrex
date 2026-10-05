//! Milestone 9.2's `anthrex run` commands (decision 40, Interfaces "CLI"): `run start
//! --delivery`, `run prs` (from the snapshot, no request of its own), `run deliver` and
//! `run watch` (decision 25's requests), `run status`'s delivery lines, and the hidden
//! `run fake-github` (decision 14), which edits a fake GitHub's `github.json` and talks
//! to no daemon and never to GitHub.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use clap::{Args, Subcommand, ValueEnum};
use daemon::host::fake::{CiRule, FakeGithubCtl, MergeMethodArg};
use daemon::host::{HOST_READ_TIMEOUT, RepoPermission};
use daemon::subprocess::Outcome;
use proto::{CiState, DeliveryMode, PrState, RunInfo, RunRequest, RunState, StageInfo};

use super::Runs;

/// `--delivery`'s values, as clap shows them.
#[derive(Clone, Copy, Debug, ValueEnum)]
enum DeliveryArg {
    Pr,
    Local,
}

/// `--delivery`'s value. clap takes it as text, so a wrong one is checked here with
/// clap's own parser and message, and exits 1 like every other refusal (Interfaces
/// "CLI"), before the daemon is asked anything.
pub(super) fn mode(given: Option<&str>) -> anyhow::Result<Option<DeliveryMode>> {
    let Some(given) = given else {
        return Ok(None);
    };
    let arg = clap::Arg::new("delivery")
        .long("delivery")
        .value_name("pr|local")
        .value_parser(clap::value_parser!(DeliveryArg));
    let command = clap::Command::new("anthrex run start").arg(arg);
    // One `--delivery=<value>` word, so a value that starts with `-` is still a value.
    let word = format!("--delivery={given}");
    let matches = command.try_get_matches_from(["anthrex run start", word.as_str()])?;
    Ok(match matches.get_one::<DeliveryArg>("delivery") {
        Some(DeliveryArg::Pr) => Some(DeliveryMode::Pr),
        Some(DeliveryArg::Local) | None => Some(DeliveryMode::Local),
    })
}

/// `run prs <run> [--json]`.
#[derive(Args, Debug)]
pub(super) struct PrsArgs {
    run: String,
    /// Print the stages' pull requests as JSON
    #[arg(long)]
    json: bool,
}

/// `run deliver <run> --stage <n>`.
#[derive(Args, Debug)]
pub(super) struct DeliverArgs {
    run: String,
    #[arg(long)]
    stage: u16,
}

/// `run watch <run> (--off | --on)`: exactly one of the two.
#[derive(Args, Debug)]
#[command(group(clap::ArgGroup::new("switch").required(true).args(["off", "on"])))]
pub(super) struct WatchArgs {
    run: String,
    #[arg(long)]
    off: bool,
    #[arg(long)]
    on: bool,
}

/// `run prs`: the snapshot's stages; a local run's one line.
pub(super) async fn prs(runs: &mut Runs, args: PrsArgs) -> anyhow::Result<()> {
    let info = runs.resolve_info(&args.run).await?;
    let text = match (&info.delivery, args.json) {
        (None, false) => format!(
            "run {} is delivered locally; it has no pull requests\n",
            info.run_id
        ),
        (None, true) => "[]\n".to_string(),
        (Some(_), false) => prs_table(&info),
        (Some(_), true) => {
            let prs: Vec<_> = info.stages.iter().map(|s| s.pr.as_ref()).collect();
            format!("{}\n", serde_json::to_string_pretty(&prs)?)
        }
    };
    print!("{}", super::status::printable(&text));
    Ok(())
}

/// `run deliver`: the reply, or the refusal and exit 1.
pub(super) async fn deliver(runs: &mut Runs, args: DeliverArgs) -> anyhow::Result<()> {
    let run_id = runs.resolve(&args.run).await?;
    let stage = args.stage;
    runs.done(RunRequest::Deliver { run_id, stage }).await
}

/// `run watch`: the reply, or the refusal and exit 1.
pub(super) async fn watch(runs: &mut Runs, args: WatchArgs) -> anyhow::Result<()> {
    let run_id = runs.resolve(&args.run).await?;
    let on = args.on && !args.off;
    runs.done(RunRequest::Watch { run_id, on }).await
}

/// `run prs`'s table: one line per stage (Interfaces "CLI").
pub fn prs_table(run: &RunInfo) -> String {
    let mut out = row(["STAGE", "PR", "STATE", "CI", "THREADS", "FIX TASKS"]);
    let of = run.stages.len();
    for stage in &run.stages {
        let n = format!("{}/{of}", stage.n);
        let state = stage_state(run, stage);
        let Some(pr) = &stage.pr else {
            out.push_str(&row([&n, "–", state, "–", "–", "–"]));
            continue;
        };
        let t = &pr.threads;
        let threads = format!("{} new, {} tasked, {} replied", t.new, t.tasked, t.replied);
        let fixes: Vec<String> = pr.fix_tasks.iter().map(|f| fix_task(f)).collect();
        let fixes = if fixes.is_empty() {
            "–".to_string()
        } else {
            fixes.join(", ")
        };
        let number = format!("#{}", pr.number);
        out.push_str(&row([&n, &number, state, ci(pr.ci), &threads, &fixes]));
    }
    out
}

/// A stage's `STATE`: its PR's (`paused` while a lower stage's PR is closed), else
/// `skipped` (no changes) or `waiting` (no PR yet).
fn stage_state(run: &RunInfo, stage: &StageInfo) -> &'static str {
    match &stage.pr {
        Some(pr) if pr.state == PrState::Open && pr.paused => "paused",
        Some(pr) => match pr.state {
            PrState::Open => "open",
            PrState::Merged => "merged",
            PrState::Closed => "closed",
        },
        None if run
            .delivery
            .as_ref()
            .is_some_and(|d| d.skipped_stages.contains(&stage.n)) =>
        {
            "skipped"
        }
        None => "waiting",
    }
}

fn ci(state: CiState) -> &'static str {
    match state {
        CiState::None => "none",
        CiState::Pending => "pending",
        CiState::Green => "green",
        CiState::Red => "red",
    }
}

/// `StagePrInfo.fix_tasks`'s `"<id> <origin> <state>"` as the table shows it: `<id>
/// <state>`.
fn fix_task(text: &str) -> String {
    let words: Vec<&str> = text.split(' ').collect();
    match words.as_slice() {
        [id, _origin, state] => format!("{id} {state}"),
        _ => text.to_string(),
    }
}

/// The table's columns, padded as [`super::status`]'s are: a cell as wide as its
/// column or wider ends in one space.
fn row(cells: [&str; 6]) -> String {
    const WIDTHS: [usize; 5] = [7, 7, 9, 9, 29];
    let mut line = String::new();
    for (cell, width) in cells.iter().zip(WIDTHS) {
        let len = cell.chars().count();
        line.push_str(cell);
        line.push_str(&" ".repeat(if len < width { width - len } else { 1 }));
    }
    line.push_str(cells[5]);
    format!("{}\n", line.trim_end())
}

/// Decision 36: a running `pr` run whose every stage has a PR and one is open.
pub fn delivering(run: &RunInfo) -> bool {
    run.state == RunState::Running && run.delivery.as_ref().is_some_and(|d| d.delivering)
}

/// `run status`'s lines for a `pr` run: `delivery: …`, and `prs: …` once a stage has a
/// PR. Empty for a local run.
pub fn status_lines(run: &RunInfo) -> String {
    let Some(d) = &run.delivery else {
        return String::new();
    };
    let watching = if d.watching {
        format!("watching every {} s", d.poll_secs)
    } else {
        "not watching".to_string()
    };
    let mut out = format!("  delivery: pr to {} ({}), {watching}\n", d.remote, d.repo);
    let prs: Vec<String> = (run.stages.iter())
        .filter_map(|s| s.pr.as_ref().map(|pr| (s, pr)))
        .map(|(stage, pr)| {
            let state = stage_state(run, stage);
            if pr.state != PrState::Open {
                return format!("#{} {state}", pr.number);
            }
            // Decision 42's count: the threads still to address, new and tasked.
            let threads = pr.threads.new.saturating_add(pr.threads.tasked);
            let threads = match threads {
                0 => String::new(),
                1 => ", 1 thread".to_string(),
                k => format!(", {k} threads"),
            };
            format!("#{} {state} (ci {}{threads})", pr.number, ci(pr.ci))
        })
        .collect();
    if !prs.is_empty() {
        out.push_str(&format!("  prs: {}\n", prs.join(", ")));
    }
    out
}

/// `anthrex run fake-github --dir <dir> <verb> …` (hidden; decision 14).
#[derive(Args, Debug)]
pub(super) struct FakeGithubArgs {
    /// The fake GitHub's directory (`ANTHREX_FAKE_HOST_DIR`); it must exist
    #[arg(long)]
    dir: PathBuf,
    #[command(subcommand)]
    verb: FakeVerb,
}

#[derive(Subcommand, Debug)]
enum FakeVerb {
    /// The repository `<owner>/<name>`, backed by the bare repository `bare`
    CreateRepo {
        owner: String,
        name: String,
        bare: PathBuf,
        #[arg(long, default_value = "main")]
        default_branch: String,
    },
    /// `gh auth status` succeeds for `host`
    LogIn {
        host: String,
    },
    SetPermission {
        user: String,
        permission: PermissionArg,
    },
    /// Replace the CI rules with `rules`, a JSON array of rules
    SetCi {
        rules: String,
    },
    /// A conversation comment; prints its id
    Comment {
        pr: u64,
        user: String,
        body: String,
    },
    /// A new review thread on `path`'s `line`; prints its first comment's id
    ReviewComment {
        pr: u64,
        user: String,
        path: String,
        line: u32,
        body: String,
    },
    /// The user merges `pr` on GitHub
    Merge {
        pr: u64,
        #[arg(long, value_enum)]
        method: MethodArg,
        #[arg(long)]
        delete_branch: bool,
    },
    Close {
        pr: u64,
    },
    /// A user's commit on the remote `branch`; prints its sha
    Commit {
        branch: String,
        path: String,
        content: String,
        message: String,
    },
    /// The pull requests, as JSON
    Prs,
    /// Every ask to land something, as JSON; must stay empty
    Forbidden,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum PermissionArg {
    Admin,
    Maintain,
    Write,
    Triage,
    Read,
    None,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum MethodArg {
    Merge,
    Squash,
    Rebase,
}

/// Runs one verb on `<dir>/github.json`. `FakeGithubCtl` panics on what it cannot do
/// (it is test setup); here that is the command's error.
pub(super) fn fake_github(args: FakeGithubArgs) -> anyhow::Result<()> {
    if !args.dir.is_dir() {
        anyhow::bail!("fake-github: {} is not a directory", args.dir.display());
    }
    let rules: Option<Vec<CiRule>> = match &args.verb {
        FakeVerb::SetCi { rules } => Some(
            serde_json::from_str(rules).map_err(|e| anyhow::anyhow!("fake-github: rules: {e}"))?,
        ),
        _ => None,
    };
    if let FakeVerb::CreateRepo { bare, .. } = &args.verb {
        bare_under(&args.dir, bare, "git".as_ref(), HOST_READ_TIMEOUT)?;
    }
    // Ruling m3: the hook in place before is put back, not the default.
    let before = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let ran =
        std::panic::catch_unwind(move || verb(&FakeGithubCtl::open(&args.dir), args.verb, rules));
    std::panic::set_hook(before);
    match ran {
        Ok(out) => {
            print!("{}", super::status::printable(&out));
            Ok(())
        }
        Err(panic) => {
            let text = (panic.downcast_ref::<String>().map(String::as_str))
                .or_else(|| panic.downcast_ref::<&str>().copied())
                .unwrap_or("panicked");
            anyhow::bail!("fake-github: {text}")
        }
    }
}

/// Ruling m2: `create-repo`'s `bare` must be a bare repository (its own git directory)
/// inside `dir`, so the later verbs never write objects or move branches in a
/// repository the fake does not own. A local `git rev-parse`, never a remote, run as
/// `program` under `bound` (decision 15, rulings R2 and R2a: given, not from `PATH`).
fn bare_under(dir: &Path, bare: &Path, program: &OsStr, bound: Duration) -> anyhow::Result<()> {
    let refuse = || {
        anyhow::anyhow!(
            "fake-github: {} is not a bare repository under {}",
            bare.display(),
            dir.display()
        )
    };
    let (Ok(dir), Ok(path)) = (dir.canonicalize(), bare.canonicalize()) else {
        return Err(refuse());
    };
    if !path.starts_with(&dir) || path == dir {
        return Err(refuse());
    }
    let mut git = std::process::Command::new(program);
    git.arg("-C").arg(&path).args([
        "--no-optional-locks",
        "rev-parse",
        "--is-bare-repository",
        "--absolute-git-dir",
    ]);
    // AGENTS.md rule 11 (W2 re-review N1): every inherited git variable dropped.
    daemon::subprocess::scrub_inherited_git(&mut git);
    // `run` kills and reaps the child's group at the bound; `Complete` means a zero exit.
    let stdout = match daemon::subprocess::run(&mut git, 64 * 1024, bound) {
        Outcome::Complete(stdout) => stdout,
        Outcome::TimedOut(_) => anyhow::bail!(
            "fake-github: git did not answer within {}s",
            bound.as_secs()
        ),
        Outcome::Failed | Outcome::Truncated(_) => return Err(refuse()),
    };
    let text = String::from_utf8_lossy(&stdout);
    let mut lines = text.lines();
    let bare_repo = lines.next() == Some("true");
    let own = lines
        .next()
        .and_then(|d| Path::new(d).canonicalize().ok())
        .is_some_and(|d| d == path);
    if !(bare_repo && own) {
        return Err(refuse());
    }
    Ok(())
}

/// What the verb prints.
fn verb(ctl: &FakeGithubCtl, verb: FakeVerb, rules: Option<Vec<CiRule>>) -> String {
    let json = |v: serde_json::Result<String>| format!("{}\n", v.expect("serialisable"));
    match verb {
        FakeVerb::CreateRepo {
            owner,
            name,
            bare,
            default_branch,
        } => ctl.create_repo(&owner, &name, &bare, &default_branch),
        FakeVerb::LogIn { host } => ctl.log_in(&host),
        FakeVerb::SetPermission { user, permission } => {
            ctl.set_permission(&user, permission.into())
        }
        FakeVerb::SetCi { .. } => ctl.set_ci(rules.unwrap_or_default()),
        FakeVerb::Comment { pr, user, body } => {
            return format!("{}\n", ctl.comment(pr, &user, &body));
        }
        FakeVerb::ReviewComment {
            pr,
            user,
            path,
            line,
            body,
        } => return format!("{}\n", ctl.review_comment(pr, &user, &path, line, &body)),
        FakeVerb::Merge {
            pr,
            method,
            delete_branch,
        } => ctl.merge(pr, method.into(), delete_branch),
        FakeVerb::Close { pr } => ctl.close(pr),
        FakeVerb::Commit {
            branch,
            path,
            content,
            message,
        } => return format!("{}\n", ctl.commit(&branch, &path, &content, &message)),
        FakeVerb::Prs => return json(serde_json::to_string_pretty(&ctl.prs())),
        FakeVerb::Forbidden => return json(serde_json::to_string(&ctl.forbidden())),
    }
    String::new()
}

impl From<PermissionArg> for RepoPermission {
    fn from(p: PermissionArg) -> Self {
        match p {
            PermissionArg::Admin => RepoPermission::Admin,
            PermissionArg::Maintain => RepoPermission::Maintain,
            PermissionArg::Write => RepoPermission::Write,
            PermissionArg::Triage => RepoPermission::Triage,
            PermissionArg::Read => RepoPermission::Read,
            PermissionArg::None => RepoPermission::None,
        }
    }
}

impl From<MethodArg> for MergeMethodArg {
    fn from(m: MethodArg) -> Self {
        match m {
            MethodArg::Merge => MergeMethodArg::Merge,
            MethodArg::Squash => MergeMethodArg::Squash,
            MethodArg::Rebase => MergeMethodArg::Rebase,
        }
    }
}

#[cfg(test)]
#[path = "delivery_tests.rs"]
mod tests;
