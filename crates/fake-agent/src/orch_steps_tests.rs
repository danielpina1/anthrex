//! Unit tests of `orch_steps` (M9.12).

use super::{Match, Until, prompt, resumed, unframe};
use crate::script::{Step, parse_script};
use serde_json::json;
use std::io::Cursor;

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|v| (*v).to_owned()).collect()
}

#[test]
fn parses_the_m9_steps_and_rejects_unknown_keys() {
    let input = concat!(
        "{\"mcp_until\":{\"tool\":\"run_status\",\"args\":{\"wait_secs\":50},",
        "\"until\":{\"pointer\":\"/gate/state\",\"equals\":\"approved\"},\"timeout_ms\":9}}\n",
        "{\"capture_json\":{\"name\":\"sid\",\"pointer\":\"/scout_id\"}}\n",
        "{\"expect\":{\"pointer\":\"/n\",\"equals\":3}}\n",
        "{\"expect_error_contains\":{\"text\":\"closed\"}}\n",
    );
    let steps = parse_script(Cursor::new(input)).unwrap();
    let until = Match {
        pointer: "/gate/state".into(),
        equals: json!("approved"),
    };
    assert_eq!(
        steps,
        vec![
            Step::McpUntil(Until {
                tool: "run_status".into(),
                args: json!({"wait_secs": 50}),
                until,
                timeout_ms: 9,
            }),
            Step::CaptureJson {
                name: "sid".into(),
                pointer: "/scout_id".into(),
            },
            Step::Expect(Match {
                pointer: "/n".into(),
                equals: json!(3),
            }),
            Step::ExpectErrorContains("closed".into()),
        ]
    );
    for line in [
        "{\"expect\":{\"pointer\":\"/n\",\"equals\":3,\"x\":1}}\n",
        "{\"capture_json\":{\"name\":\"a\"}}\n",
        "{\"mcp_until\":{\"tool\":\"t\",\"args\":{},\"until\":{\"pointer\":\"/a\"},\"timeout_ms\":1}}\n",
    ] {
        assert!(parse_script(Cursor::new(line)).is_err(), "{line}");
    }
}

/// A host whose every call takes `call` and answers `{}`.
struct Slow {
    call: std::time::Duration,
    calls: usize,
    vars: crate::roles::Vars,
}

impl super::Host for Slow {
    fn call(&mut self, _: &str, _: &serde_json::Value) -> anyhow::Result<crate::mcp::Reply> {
        std::thread::sleep(self.call);
        self.calls += 1;
        Ok(crate::mcp::Reply {
            ok: true,
            text: "{}".into(),
        })
    }
    fn wait(&mut self, duration: std::time::Duration) -> Option<String> {
        std::thread::sleep(duration);
        None
    }
    fn vars(&mut self) -> &mut crate::roles::Vars {
        &mut self.vars
    }
    fn save_vars(&mut self) -> anyhow::Result<()> {
        Ok(())
    }
}

/// Review fix 4: no call starts once the deadline has passed, so the step ends at
/// most one call's time after it. A 50 ms call and a 120 ms timeout: the wait after
/// the first call reaches the deadline, and the step ends there.
#[test]
fn mcp_until_starts_no_call_after_its_deadline() {
    let mut host = Slow {
        call: std::time::Duration::from_millis(50),
        calls: 0,
        vars: Default::default(),
    };
    let step = Step::McpUntil(Until {
        tool: "run_status".into(),
        args: json!({}),
        until: Match {
            pointer: "/gate".into(),
            equals: json!("approved"),
        },
        timeout_ms: 120,
    });
    let start = std::time::Instant::now();
    let flow = super::run(&mut host, &step).unwrap();
    assert!(matches!(flow, super::Flow::Exit(4)));
    assert_eq!(host.calls, 1);
    assert!(start.elapsed() < std::time::Duration::from_millis(170 + 100));
}

#[test]
fn a_match_needs_json_and_the_exact_value() {
    let m = Match {
        pointer: "/a/b".into(),
        equals: json!(1),
    };
    assert!(m.holds(r#"{"a":{"b":1}}"#));
    assert!(!m.holds(r#"{"a":{"b":"1"}}"#));
    assert!(!m.holds(r#"{"a":{}}"#));
    assert!(!m.holds("not json"));
}

#[test]
fn unframe_drops_only_the_paste_brackets() {
    let raw = b"x \x1b[200~a\nb\x1b[201~ y\x1b[A";
    assert_eq!(unframe(raw), b"x a\nb y\x1b[A");
}

#[test]
fn resume_and_prompt_come_from_either_runtimes_argv() {
    let claude = strings(&["--name", "n", "--model", "opus", "--resume", "s-1"]);
    assert_eq!(resumed(&claude).as_deref(), Some("s-1"));
    assert_eq!(prompt(&claude), "");
    let codex = strings(&["-C", "/r", "-m", "gpt", "resume", "thr-1"]);
    assert_eq!(resumed(&codex).as_deref(), Some("thr-1"));
    let fresh = strings(&["--model", "opus", "--", "resume thr-1"]);
    assert_eq!(
        (resumed(&fresh), prompt(&fresh)),
        (None, "resume thr-1".into())
    );
}
