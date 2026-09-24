use std::io::BufRead;

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

#[derive(Debug, Clone, PartialEq)]
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
        expect_error: bool,
    },
    Transcript(Value),
    // The headless steps (M8a.20).
    ReadMessage {
        timeout_ms: Option<u64>,
        expect: Option<String>,
    },
    EndTurn,
    Sh(String),
    Capture {
        name: String,
        sh: String,
    },
    ApiRetry {
        error: String,
        delay_ms: u64,
        times: u32,
    },
    FailTurn(String),
    Deny {
        tool: String,
        reason: String,
    },
    Usage(Usage),
    Hang,
}

/// The token usage a turn end reports (the `usage` step).
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Usage {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
}

impl Default for Usage {
    fn default() -> Self {
        Self {
            input: 10,
            output: 5,
            cache_read: 0,
            cache_write: 0,
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct McpCallStep {
    tool: String,
    args: Value,
    #[serde(default)]
    expect_error: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReadMessageStep {
    timeout_ms: Option<u64>,
    expect: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ShStep {
    cmd: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CaptureStep {
    name: String,
    sh: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ApiRetryStep {
    error: String,
    delay_ms: u64,
    times: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FailTurnStep {
    error: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DenyStep {
    tool: String,
    reason: String,
}

/// `{}` for a step with no arguments (`end_turn`, `hang`).
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NoArgs {}

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
            let call: McpCallStep = field(object, "mcp_call")?;
            Ok(Step::McpCall {
                tool: call.tool,
                args: call.args,
                expect_error: call.expect_error,
            })
        }
        Some("read_message") if has_keys(object, &["read_message"]) => {
            let step: ReadMessageStep = field(object, "read_message")?;
            Ok(Step::ReadMessage {
                timeout_ms: step.timeout_ms,
                expect: step.expect,
            })
        }
        Some("end_turn") if has_keys(object, &["end_turn"]) => {
            let NoArgs {} = field(object, "end_turn")?;
            Ok(Step::EndTurn)
        }
        Some("sh") if has_keys(object, &["sh"]) => {
            let ShStep { cmd } = field(object, "sh")?;
            Ok(Step::Sh(cmd))
        }
        Some("capture") if has_keys(object, &["capture"]) => {
            let CaptureStep { name, sh } = field(object, "capture")?;
            Ok(Step::Capture { name, sh })
        }
        Some("api_retry") if has_keys(object, &["api_retry"]) => {
            let step: ApiRetryStep = field(object, "api_retry")?;
            Ok(Step::ApiRetry {
                error: step.error,
                delay_ms: step.delay_ms,
                times: step.times,
            })
        }
        Some("fail_turn") if has_keys(object, &["fail_turn"]) => {
            let FailTurnStep { error } = field(object, "fail_turn")?;
            Ok(Step::FailTurn(error))
        }
        Some("deny") if has_keys(object, &["deny"]) => {
            let DenyStep { tool, reason } = field(object, "deny")?;
            Ok(Step::Deny { tool, reason })
        }
        Some("usage") if has_keys(object, &["usage"]) => Ok(Step::Usage(field(object, "usage")?)),
        Some("hang") if has_keys(object, &["hang"]) => {
            let NoArgs {} = field(object, "hang")?;
            Ok(Step::Hang)
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
    use super::{Step, Usage, parse_script};
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
                    expect_error: false,
                },
                Step::Transcript(json!({"type": "x"})),
            ]
        );
    }

    #[test]
    fn parses_every_headless_step() {
        let input = concat!(
            "{\"mcp_call\":{\"tool\":\"task_done\",\"args\":{},\"expect_error\":true}}\n",
            "{\"read_message\":{\"timeout_ms\":50,\"expect\":\"go\"}}\n",
            "{\"read_message\":{}}\n",
            "{\"end_turn\":{}}\n",
            "{\"sh\":{\"cmd\":\"true\"}}\n",
            "{\"capture\":{\"name\":\"red\",\"sh\":\"git rev-parse HEAD\"}}\n",
            "{\"api_retry\":{\"error\":\"rate_limit\",\"delay_ms\":10,\"times\":2}}\n",
            "{\"fail_turn\":{\"error\":\"rate_limit\"}}\n",
            "{\"deny\":{\"tool\":\"Write\",\"reason\":\"no\"}}\n",
            "{\"usage\":{\"input\":1,\"output\":2,\"cache_read\":3,\"cache_write\":4}}\n",
            "{\"hang\":{}}\n",
        );

        let steps = parse_script(Cursor::new(input)).unwrap();

        assert_eq!(
            steps,
            vec![
                Step::McpCall {
                    tool: "task_done".into(),
                    args: json!({}),
                    expect_error: true,
                },
                Step::ReadMessage {
                    timeout_ms: Some(50),
                    expect: Some("go".into()),
                },
                Step::ReadMessage {
                    timeout_ms: None,
                    expect: None,
                },
                Step::EndTurn,
                Step::Sh("true".into()),
                Step::Capture {
                    name: "red".into(),
                    sh: "git rev-parse HEAD".into(),
                },
                Step::ApiRetry {
                    error: "rate_limit".into(),
                    delay_ms: 10,
                    times: 2,
                },
                Step::FailTurn("rate_limit".into()),
                Step::Deny {
                    tool: "Write".into(),
                    reason: "no".into(),
                },
                Step::Usage(Usage {
                    input: 1,
                    output: 2,
                    cache_read: 3,
                    cache_write: 4,
                }),
                Step::Hang,
            ]
        );
    }

    #[test]
    fn headless_steps_reject_unknown_keys() {
        for line in [
            "{\"sh\":{\"cmd\":\"true\",\"extra\":1}}\n",
            "{\"hang\":{\"x\":1}}\n",
            "{\"mcp_call\":{\"tool\":\"t\",\"args\":{},\"retry\":true}}\n",
        ] {
            let error = parse_script(Cursor::new(line)).unwrap_err();
            assert!(error.to_string().contains("line 1"), "{error:#}");
        }
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
