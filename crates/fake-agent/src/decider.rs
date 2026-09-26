//! The decider mode (M8b decision 36): a headless call whose prompt's first line is
//! `[anthrex decider] <kind> v1` answers from `$FAKE_AGENT_DECIDER_DIR/<kind>-<n>.json`,
//! claimed in order of `n`, and records every call in `calls.jsonl` there (and, with
//! `FAKE_AGENT_ENV_FILE` set, its environment's names in that file). Its output has only
//! the shapes of M8b.1's decider recordings.

use std::fs::{self, OpenOptions};
use std::io::{self, ErrorKind, Read, Write};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde_json::{Map, Value, json};

use crate::headless::Events;
use crate::script::Usage;
use crate::stream_claude::{self, Claude, Line};
use crate::stream_codex::Codex;

const HEADER: &str = "[anthrex decider] ";
const VERSION: &str = " v1";

/// The kind a decider prompt names on its first line, if it is one.
pub fn kind(prompt: &str) -> Option<String> {
    let first = prompt.lines().next()?;
    let kind = first.strip_prefix(HEADER)?.strip_suffix(VERSION)?;
    let word = |c: char| c.is_ascii_lowercase() || c == '_';
    (!kind.is_empty() && kind.chars().all(word)).then(|| kind.to_owned())
}

/// How a headless Claude start goes on after [`claude`].
pub enum Start {
    /// It was a decider call, which exited with this code.
    Answered(i32),
    /// It is an ordinary session, whose first stdin line (if any) was read already.
    Session(Option<String>),
}

/// A Codex start: a decider call when the prompt, the last argument, names a kind.
pub fn codex(args: &[String], thread: &str) -> Result<Option<i32>> {
    let Some(prompt) = args.last() else {
        return Ok(None);
    };
    let Some(kind) = kind(prompt) else {
        return Ok(None);
    };
    drain_stdin()?;
    let answerer = Answerer::Codex {
        thread: thread.to_owned(),
    };
    run(answerer, &kind, args, prompt).map(Some)
}

/// A Claude start: reads the first stdin line, and answers when its user message names
/// a kind.
pub fn claude(
    args: &[String],
    session: &str,
    model: Option<&str>,
    mode: Option<&str>,
) -> Result<Start> {
    let first = stream_claude::read_first_line();
    let message = first.as_deref().and_then(stream_claude::parse_line);
    let Some(Line::Message(prompt)) = message else {
        return Ok(Start::Session(first));
    };
    let Some(kind) = kind(&prompt) else {
        return Ok(Start::Session(first));
    };
    let answerer = Answerer::Claude(Claude::new(session, model, mode, false));
    run(answerer, &kind, args, &prompt).map(Start::Answered)
}

/// What a script makes the call do.
#[derive(Debug, Clone, PartialEq)]
enum Reply {
    /// A structured answer, in the fixture's form.
    Answer(Value),
    /// Raw assistant text.
    Text(String),
    /// A failed turn with this error category.
    FailTurn(String),
    /// Exit with this code before answering.
    Exit(i32),
    /// No output until killed.
    Hang,
}

#[derive(Debug, Clone, PartialEq)]
struct Scripted {
    reply: Reply,
    usage: Usage,
}

fn parse(value: Value) -> Result<Scripted> {
    let Value::Object(mut object) = value else {
        bail!("a decider script must be a JSON object");
    };
    let usage = match object.remove("usage") {
        Some(usage) => serde_json::from_value(usage).context("invalid usage")?,
        None => Usage::default(),
    };
    let [(key, value)]: [(String, Value); 1] = object
        .into_iter()
        .collect::<Vec<_>>()
        .try_into()
        .map_err(|_| anyhow::anyhow!("a decider script needs exactly one reply key"))?;
    let reply = match key.as_str() {
        "answer" => Reply::Answer(value),
        "text" => Reply::Text(serde_json::from_value(value).context("invalid text")?),
        "fail_turn" => Reply::FailTurn(serde_json::from_value(value).context("invalid fail_turn")?),
        "exit" => Reply::Exit(serde_json::from_value(value).context("invalid exit")?),
        "hang" if value == json!(true) => Reply::Hang,
        other => bail!("unknown decider reply {other:?}"),
    };
    Ok(Scripted { reply, usage })
}

/// Which runtime is answering, with what its output needs.
enum Answerer {
    Claude(Claude),
    Codex { thread: String },
}

/// Runs one decider call of `kind`: records it, claims its script and answers. Exits 2
/// when no script is left, as the caller then falls back.
fn run(answerer: Answerer, kind: &str, args: &[String], prompt: &str) -> Result<i32> {
    record_env_keys()?;
    // A directory that does not exist has no script either (decision 36: exit 2).
    let dir = std::env::var_os("FAKE_AGENT_DECIDER_DIR")
        .map(PathBuf::from)
        .filter(|dir| dir.is_dir());
    if let Some(dir) = &dir {
        record(dir, kind, args, prompt)?;
    }
    let claimed = match &dir {
        Some(dir) => claim(dir, kind)?,
        None => None,
    };
    let Some(path) = claimed else {
        eprintln!("fake-agent: no scripted decider answer for {kind}");
        return Ok(2);
    };
    let text = fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    let value = serde_json::from_str(&text).with_context(|| format!("parse {}", path.display()))?;
    let scripted = parse(value).with_context(|| format!("in {}", path.display()))?;
    match answerer {
        Answerer::Claude(claude) => answer_claude(claude, scripted),
        Answerer::Codex { thread } => answer_codex(&thread, scripted),
    }
}

/// Claude: the `system/init`, then the reply; stdin (already closed by the daemon) is
/// read to its end before exiting, as the real CLI does.
fn answer_claude(claude: Claude, scripted: Scripted) -> Result<i32> {
    let mut claude = claude.decider();
    match scripted.reply {
        Reply::Exit(code) => return Ok(code),
        Reply::Hang => hang(),
        Reply::Answer(answer) => {
            claude.turn_started()?;
            claude.structured_answer(&answer, scripted.usage)?;
        }
        Reply::Text(text) => {
            claude.turn_started()?;
            claude.text(&text)?;
            claude.turn_completed(scripted.usage)?;
        }
        Reply::FailTurn(error) => {
            claude.turn_started()?;
            claude.turn_failed(&error, scripted.usage)?;
        }
    }
    io::copy(&mut io::stdin().lock(), &mut io::sink()).context("read stdin to EOF")?;
    Ok(0)
}

/// Codex: stdin was read to its end first (the real `codex exec` does); the answer is
/// the final `agent_message` text.
fn answer_codex(thread: &str, scripted: Scripted) -> Result<i32> {
    let mut codex = Codex::new(thread);
    match scripted.reply {
        Reply::Exit(code) => return Ok(code),
        Reply::Hang => hang(),
        Reply::Answer(answer) => {
            codex.thread_started()?;
            codex.turn_started()?;
            codex.text(&answer.to_string())?;
            codex.turn_completed(scripted.usage)?;
        }
        Reply::Text(text) => {
            codex.thread_started()?;
            codex.turn_started()?;
            codex.text(&text)?;
            codex.turn_completed(scripted.usage)?;
        }
        Reply::FailTurn(error) => {
            codex.thread_started()?;
            codex.turn_started()?;
            codex.turn_failed(&error, scripted.usage)?;
            return Ok(1);
        }
    }
    Ok(0)
}

/// Writes nothing and never returns: the caller's timeout kills the process.
fn hang() -> ! {
    loop {
        thread::sleep(Duration::from_secs(1));
    }
}

/// Codex reads its piped stdin to EOF before anything else (carry T17-C1).
fn drain_stdin() -> Result<()> {
    let mut ignored = Vec::new();
    io::stdin()
        .read_to_end(&mut ignored)
        .context("read stdin to EOF")?;
    Ok(())
}

/// With `FAKE_AGENT_ENV_FILE` set, writes the names (never the values) of the decider's
/// environment variables there, sorted, one per line (M8b.7: a decider sees no API
/// credential).
fn record_env_keys() -> Result<()> {
    let Some(path) = std::env::var_os("FAKE_AGENT_ENV_FILE") else {
        return Ok(());
    };
    let mut keys: Vec<String> = std::env::vars_os()
        .map(|(key, _)| key.to_string_lossy().into_owned())
        .collect();
    keys.sort();
    let text: String = keys.iter().map(|key| format!("{key}\n")).collect();
    fs::write(&path, text).with_context(|| format!("write {}", path.to_string_lossy()))
}

/// Appends `{"kind","argv","prompt"}` to `<dir>/calls.jsonl`.
fn record(dir: &Path, kind: &str, args: &[String], prompt: &str) -> Result<()> {
    let mut entry = Map::new();
    entry.insert("kind".into(), kind.into());
    entry.insert("argv".into(), json!(args));
    entry.insert("prompt".into(), prompt.into());
    let path = dir.join("calls.jsonl");
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("open {}", path.display()))?;
    file.write_all(format!("{}\n", Value::Object(entry)).as_bytes())
        .with_context(|| format!("append to {}", path.display()))
}

/// Claims `<kind>-<n>.json` with the smallest `n` whose `.claimed` does not exist, by
/// creating that file with `create_new` (as M8a.20's role scripts are claimed).
fn claim(dir: &Path, kind: &str) -> Result<Option<PathBuf>> {
    let prefix = format!("{kind}-");
    let mut candidates: Vec<(u64, PathBuf)> = Vec::new();
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).with_context(|| format!("read {}", dir.display())),
    };
    for entry in entries {
        let path = entry.context("read the decider directory")?.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let n = name
            .strip_suffix(".json")
            .and_then(|stem| stem.strip_prefix(&prefix))
            .and_then(|n| n.parse().ok());
        if let Some(n) = n {
            candidates.push((n, path));
        }
    }
    candidates.sort();
    for (_, path) in candidates {
        let mut claim = path.clone().into_os_string();
        claim.push(".claimed");
        match OpenOptions::new().write(true).create_new(true).open(&claim) {
            Ok(_) => return Ok(Some(path)),
            Err(error) if error.kind() == ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error).context("claim a decider script"),
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::{Reply, kind, parse};
    use crate::script::Usage;
    use serde_json::json;

    #[test]
    fn the_kind_is_read_from_the_prompts_first_line() {
        assert_eq!(
            kind("[anthrex decider] triage v1\nGoal"),
            Some("triage".into())
        );
        assert_eq!(
            kind("[anthrex decider] size_check v1"),
            Some("size_check".into())
        );
        for prompt in [
            "",
            "hello",
            "Goal\n[anthrex decider] triage v1",
            "[anthrex decider] triage v2",
            "[anthrex decider]  v1",
            "[anthrex decider] Tri age v1",
        ] {
            assert_eq!(kind(prompt), None, "{prompt:?}");
        }
    }

    #[test]
    fn parses_every_reply_with_its_usage() {
        let usage = json!({"input": 1, "output": 2, "cache_read": 3, "cache_write": 4});
        let scripted = parse(json!({"answer": {"a": 1}, "usage": usage})).unwrap();
        assert_eq!(scripted.reply, Reply::Answer(json!({"a": 1})));
        assert_eq!(scripted.usage.cache_write, 4);
        for (value, reply) in [
            (json!({"text": "t"}), Reply::Text("t".into())),
            (
                json!({"fail_turn": "rate_limit"}),
                Reply::FailTurn("rate_limit".into()),
            ),
            (json!({"exit": 7}), Reply::Exit(7)),
            (json!({"hang": true}), Reply::Hang),
        ] {
            let scripted = parse(value).unwrap();
            assert_eq!((scripted.reply, scripted.usage), (reply, Usage::default()));
        }
        for bad in [
            json!([]),
            json!({}),
            json!({"text": "a", "answer": 1}),
            json!({"hang": false}),
            json!({"wat": 1}),
            json!({"text": "a", "usage": {"input": 1}}),
        ] {
            assert!(parse(bad.clone()).is_err(), "{bad}");
        }
    }
}
