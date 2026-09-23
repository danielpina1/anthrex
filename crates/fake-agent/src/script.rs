use std::io::BufRead;

use anyhow::{Context, Result, bail};
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

#[derive(Debug, PartialEq)]
pub enum Step {
    Print(String),
    Hook {
        event: String,
        payload: Value,
    },
    Notify(Value),
    Title(String),
    Bell,
    WaitMs(u64),
    ReadLine,
    GitCommit {
        file: String,
        content: String,
        message: String,
    },
    Exit(i32),
    McpCall {
        tool: String,
        args: Value,
    },
    Transcript(Value),
}

pub fn parse_script(reader: impl BufRead) -> Result<Vec<Step>> {
    reader
        .lines()
        .enumerate()
        .try_fold(Vec::new(), |mut steps, (index, line)| {
            let line_number = index + 1;
            let line = line.with_context(|| format!("bad step on line {line_number}"))?;
            if line.trim().is_empty() {
                return Ok(steps);
            }
            let value = serde_json::from_str(&line)
                .with_context(|| format!("bad step on line {line_number}"))?;
            let step =
                parse_step(value).with_context(|| format!("bad step on line {line_number}"))?;
            steps.push(step);
            Ok(steps)
        })
}

fn parse_step(value: Value) -> Result<Step> {
    let object = value.as_object().context("step must be a JSON object")?;
    match object.keys().next().map(String::as_str) {
        Some("print") if has_keys(object, &["print"]) => Ok(Step::Print(field(object, "print")?)),
        Some("hook") if has_keys(object, &["hook", "payload"]) => Ok(Step::Hook {
            event: field(object, "hook")?,
            payload: object["payload"].clone(),
        }),
        Some("notify") if has_keys(object, &["notify"]) => {
            Ok(Step::Notify(object["notify"].clone()))
        }
        Some("title") if has_keys(object, &["title"]) => Ok(Step::Title(field(object, "title")?)),
        Some("bell") if has_keys(object, &["bell"]) => {
            let _: bool = field(object, "bell")?;
            Ok(Step::Bell)
        }
        Some("wait_ms") if has_keys(object, &["wait_ms"]) => {
            Ok(Step::WaitMs(field(object, "wait_ms")?))
        }
        Some("read_line") if has_keys(object, &["read_line"]) => {
            let _: bool = field(object, "read_line")?;
            Ok(Step::ReadLine)
        }
        Some("git_commit") if has_keys(object, &["git_commit"]) => {
            let commit = object["git_commit"]
                .as_object()
                .context("git_commit must be an object")?;
            if !has_keys(commit, &["file", "content", "message"]) {
                bail!("git_commit requires file, content, and message");
            }
            Ok(Step::GitCommit {
                file: field(commit, "file")?,
                content: field(commit, "content")?,
                message: field(commit, "message")?,
            })
        }
        Some("exit") if has_keys(object, &["exit"]) => Ok(Step::Exit(field(object, "exit")?)),
        Some("mcp_call") if has_keys(object, &["mcp_call"]) => {
            let call = object["mcp_call"]
                .as_object()
                .context("mcp_call must be an object")?;
            if !has_keys(call, &["tool", "args"]) {
                bail!("mcp_call requires tool and args");
            }
            Ok(Step::McpCall {
                tool: field(call, "tool")?,
                args: call["args"].clone(),
            })
        }
        Some("transcript") if has_keys(object, &["transcript"]) => {
            object["transcript"]
                .as_object()
                .context("transcript must be an object")?;
            Ok(Step::Transcript(object["transcript"].clone()))
        }
        _ => bail!("unknown or malformed step"),
    }
}

fn has_keys(object: &Map<String, Value>, expected: &[&str]) -> bool {
    object.len() == expected.len() && expected.iter().all(|key| object.contains_key(*key))
}

fn field<T: DeserializeOwned>(object: &Map<String, Value>, name: &str) -> Result<T> {
    serde_json::from_value(object[name].clone()).with_context(|| format!("invalid {name}"))
}

#[cfg(test)]
mod tests {
    use super::{Step, parse_script};
    use serde_json::json;
    use std::io::Cursor;

    #[test]
    fn parses_every_step_kind() {
        let input = concat!(
            "{\"print\":\"hello\"}\n",
            "{\"hook\":\"PreToolUse\",\"payload\":{\"tool_name\":\"Bash\"}}\n",
            "{\"notify\":{\"last-assistant-message\":\"done\"}}\n",
            "{\"title\":\"Working\"}\n",
            "{\"bell\":true}\n",
            "{\"wait_ms\":200}\n",
            "{\"read_line\":true}\n",
            "{\"git_commit\":{\"file\":\"a.txt\",\"content\":\"x\",\"message\":\"m\"}}\n",
            "{\"exit\":4}\n",
            "{\"mcp_call\":{\"tool\":\"report_done\",\"args\":{\"ok\":true}}}\n",
            "{\"transcript\":{\"type\":\"x\"}}\n",
        );

        let steps = parse_script(Cursor::new(input)).unwrap();

        assert_eq!(
            steps,
            vec![
                Step::Print("hello".into()),
                Step::Hook {
                    event: "PreToolUse".into(),
                    payload: json!({"tool_name": "Bash"}),
                },
                Step::Notify(json!({"last-assistant-message": "done"})),
                Step::Title("Working".into()),
                Step::Bell,
                Step::WaitMs(200),
                Step::ReadLine,
                Step::GitCommit {
                    file: "a.txt".into(),
                    content: "x".into(),
                    message: "m".into(),
                },
                Step::Exit(4),
                Step::McpCall {
                    tool: "report_done".into(),
                    args: json!({"ok": true}),
                },
                Step::Transcript(json!({"type": "x"})),
            ]
        );
    }

    #[test]
    fn transcript_step_requires_an_object() {
        let error = parse_script(Cursor::new("{\"transcript\":3}\n")).unwrap_err();

        assert!(error.to_string().contains("line 1"), "{error:#}");
    }

    #[test]
    fn transcript_step_rejects_extra_keys() {
        let error = parse_script(Cursor::new("{\"transcript\":{},\"extra\":1}\n")).unwrap_err();

        assert!(error.to_string().contains("line 1"), "{error:#}");
    }

    #[test]
    fn bad_step_reports_its_line() {
        let error = parse_script(Cursor::new(
            "{\"print\":\"one\"}\n{\"bell\":true}\n{\"wat\":false}\n",
        ))
        .unwrap_err();

        assert!(error.to_string().contains("line 3"), "{error:#}");
    }

    #[test]
    fn skips_leading_and_interior_blank_lines() {
        let steps = parse_script(Cursor::new(
            "\n  \t\n{\"print\":\"one\"}\n\t \n{\"exit\":0}\n",
        ))
        .unwrap();

        assert_eq!(steps, vec![Step::Print("one".into()), Step::Exit(0)]);
    }

    #[test]
    fn bad_step_line_counts_skipped_blank_lines() {
        let error = parse_script(Cursor::new(
            "\n{\"print\":\"one\"}\n  \n{\"bell\":true}\n{\"wat\":false}\n",
        ))
        .unwrap_err();

        assert!(error.to_string().contains("line 5"), "{error:#}");
    }
}
