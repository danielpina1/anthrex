//! `anthrex lessons …` (milestone 9.15 decision 20): list a repository's lessons, and
//! approve, reject or forget them. Every request waits [`RUN_REQUEST_TIMEOUT`]; every
//! refusal prints the daemon's message and exits 1. There is no `add` (L5).

use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};

use clap::{Args, Subcommand};
use proto::safe_text::one_line;
use proto::{ClientMsg, DaemonMsg, LessonInfo, LessonStatus, LessonsRequest, RunReply, RunRequest};

use crate::client::CliClient;
use crate::run_cmd::RUN_REQUEST_TIMEOUT;

/// Decision 20: without `--yes`, a decision is asked on a terminal only.
pub const NOT_A_TERMINAL: &str = "stdin is not a terminal; pass --yes";
/// The column a lesson's body is wrapped at.
pub const WRAP: usize = 100;
/// The body's and the files' indent.
const INDENT: &str = "    ";

#[derive(Args, Debug)]
pub struct LessonsArgs {
    #[command(subcommand)]
    command: LessonsCommand,
}

#[derive(Subcommand, Debug, PartialEq, Eq)]
enum LessonsCommand {
    /// List this repository's lessons
    List {
        /// Also list rejected and retired lessons
        #[arg(long)]
        all: bool,
        /// Print the lessons as pretty JSON
        #[arg(long)]
        json: bool,
    },
    /// Approve proposed or on-trial lessons, so prompts may use them
    Approve {
        /// Lesson ids, or unique prefixes of at least six characters
        #[arg(required = true)]
        ids: Vec<String>,
        /// Do not ask first
        #[arg(long)]
        yes: bool,
    },
    /// Reject proposed or on-trial lessons; a rejected lesson is never proposed again
    Reject {
        #[arg(required = true)]
        ids: Vec<String>,
        #[arg(long)]
        yes: bool,
    },
    /// Forget lessons: retire a distilled one, dismiss a known failure
    Forget {
        #[arg(required = true)]
        ids: Vec<String>,
        #[arg(long)]
        yes: bool,
    },
}

/// Runs one `anthrex lessons` command; any error is printed as it is and exits 1.
pub async fn main(args: LessonsArgs, socket: PathBuf, dir: Option<PathBuf>) -> anyhow::Result<()> {
    if let Err(error) = dispatch(args.command, &socket, dir).await {
        eprintln!("{}", crate::run_cmd::printable(&error.to_string()));
        std::process::exit(1);
    }
    Ok(())
}

async fn dispatch(
    command: LessonsCommand,
    socket: &Path,
    dir: Option<PathBuf>,
) -> anyhow::Result<()> {
    let dir = crate::resolve_dir(dir)?;
    let (verb, ids, yes, approve) = match command {
        LessonsCommand::List { all, json } => return list(socket, dir, all, json).await,
        LessonsCommand::Approve { ids, yes } => ("approve", ids, yes, Some(true)),
        LessonsCommand::Reject { ids, yes } => ("reject", ids, yes, Some(false)),
        LessonsCommand::Forget { ids, yes } => ("forget", ids, yes, None),
    };
    if !yes && !std::io::stdin().is_terminal() {
        anyhow::bail!(NOT_A_TERMINAL);
    }
    tui::spawn::ensure_daemon(&std::env::current_exe()?, socket).await?;
    let mut client = CliClient::connect(socket).await?;
    let ids = if yes {
        ids
    } else {
        let all = LessonsRequest::List {
            dir: dir.clone(),
            all: true,
        };
        let known = lessons_of(request(&mut client, all).await?)?;
        let mut chosen = Vec::new();
        for id in ids {
            if ask(&prompt(verb, &id, &known)).await? {
                chosen.push(id);
            }
        }
        chosen
    };
    if ids.is_empty() {
        return Ok(());
    }
    let req = match approve {
        Some(approve) => LessonsRequest::Decide { dir, ids, approve },
        None => LessonsRequest::Forget { dir, ids },
    };
    match request(&mut client, req).await? {
        RunReply::Done { message, .. } => {
            println!("{}", crate::run_cmd::printable(&message));
            Ok(())
        }
        other => anyhow::bail!("unexpected reply: {other:?}"),
    }
}

async fn list(socket: &Path, dir: PathBuf, all: bool, json: bool) -> anyhow::Result<()> {
    tui::spawn::ensure_daemon(&std::env::current_exe()?, socket).await?;
    let mut client = CliClient::connect(socket).await?;
    let req = LessonsRequest::List {
        dir: dir.clone(),
        all,
    };
    let lessons = lessons_of(request(&mut client, req).await?)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&lessons)?);
    } else {
        let project = daemon::project::resolve_roots(dir).await.project;
        print!("{}", list_text(&project.display().to_string(), &lessons));
    }
    Ok(())
}

/// One request; a refusal is the error, in the daemon's words.
async fn request(client: &mut CliClient, req: LessonsRequest) -> anyhow::Result<RunReply> {
    let msg = ClientMsg::Run(RunRequest::Lessons(req));
    match client
        .request_with_timeout(msg, RUN_REQUEST_TIMEOUT)
        .await?
    {
        DaemonMsg::Run(RunReply::Refused { message, .. }) => anyhow::bail!(message),
        DaemonMsg::Run(reply) => Ok(reply),
        DaemonMsg::Error { message, .. } => anyhow::bail!(message),
        other => anyhow::bail!("unexpected reply: {other:?}"),
    }
}

fn lessons_of(reply: RunReply) -> anyhow::Result<Vec<LessonInfo>> {
    match reply {
        RunReply::Lessons { lessons, .. } => Ok(lessons),
        other => anyhow::bail!("unexpected reply: {other:?}"),
    }
}

/// Decision 20's question for `id`: `<verb> <id> "<title>"? [y/N] `, the full id and
/// title of the one lesson `id` names; just the id when it names none or several (the
/// daemon then refuses it).
fn prompt(verb: &str, id: &str, known: &[LessonInfo]) -> String {
    let exact = known.iter().find(|l| l.id == id);
    let mut prefixed = known.iter().filter(|l| l.id.starts_with(id));
    let one = exact.or_else(|| match (prefixed.next(), prefixed.next()) {
        (Some(one), None) => Some(one),
        _ => None,
    });
    match one {
        Some(lesson) => format!(
            "{verb} {} \"{}\"? [y/N] ",
            one_line(&lesson.id),
            one_line(&lesson.title)
        ),
        None => format!("{verb} {}? [y/N] ", one_line(id)),
    }
}

/// Asks `question` on stderr and reads one line of stdin; yes only for `y`/`yes`.
async fn ask(question: &str) -> anyhow::Result<bool> {
    eprint!("{question}");
    let _ = std::io::stderr().flush();
    let line = tokio::task::spawn_blocking(|| {
        let mut line = String::new();
        std::io::stdin().read_line(&mut line).map(|_| line)
    })
    .await??;
    Ok(matches!(
        line.trim().to_ascii_lowercase().as_str(),
        "y" | "yes"
    ))
}

/// Decision 20's `list` output for the lessons of `project`.
pub fn list_text(project: &str, lessons: &[LessonInfo]) -> String {
    let project = one_line(project);
    if lessons.is_empty() {
        return format!("no lessons for {project} yet\n");
    }
    let count = |f: fn(LessonStatus) -> bool| lessons.iter().filter(|l| f(l.status)).count();
    let active = count(|s| {
        matches!(
            s,
            LessonStatus::OnTrial
                | LessonStatus::Kept
                | LessonStatus::Approved
                | LessonStatus::Known
        )
    });
    let review = count(|s| s == LessonStatus::Proposed);
    let mut out = format!("lessons for {project}: {active} active, {review} to review\n");
    for (n, lesson) in lessons.iter().enumerate() {
        if n > 0 {
            out.push('\n');
        }
        let usefulness = if lesson.outcomes == 0 {
            "–".to_string()
        } else {
            format!("{}/{}", lesson.green, lesson.outcomes)
        };
        out.push_str(&format!(
            "{}  {}  {}  {usefulness}  {}\n",
            one_line(&lesson.id),
            lesson.status.name(),
            lesson.layer.name(),
            one_line(&lesson.title)
        ));
        for line in wrap(&one_line(&lesson.body), WRAP - INDENT.len()) {
            out.push_str(&format!("{INDENT}{line}\n"));
        }
        if !lesson.files.is_empty() {
            let files: Vec<String> = lesson.files.iter().map(|f| one_line(f)).collect();
            out.push_str(&format!("{INDENT}files: {}\n", files.join(", ")));
        }
    }
    out
}

/// `text` in lines of at most `width` characters, broken at spaces; a word longer
/// than `width` is cut.
fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        let mut word: Vec<char> = word.chars().collect();
        while word.len() > width {
            if !line.is_empty() {
                lines.push(std::mem::take(&mut line));
            }
            lines.push(word.drain(..width).collect());
        }
        let word: String = word.into_iter().collect();
        if word.is_empty() {
            continue;
        }
        if !line.is_empty() && line.chars().count() + 1 + word.chars().count() > width {
            lines.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(&word);
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

#[cfg(test)]
#[path = "lessons_cmd_tests.rs"]
mod tests;
