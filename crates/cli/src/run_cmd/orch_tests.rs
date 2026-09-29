use clap::Parser;
use clap::error::ErrorKind;
use proto::{MessageKind, MessageTarget, OrchestratorChoice, PlanEdit, Runtime};

use super::super::RunCommand;
use super::*;

#[derive(Parser, Debug)]
struct Cli {
    #[command(subcommand)]
    command: RunCommand,
}

fn parse(args: &[&str]) -> Result<RunCommand, ErrorKind> {
    let mut all = vec!["run"];
    all.extend_from_slice(args);
    Cli::try_parse_from(all)
        .map(|cli| cli.command)
        .map_err(|e| e.kind())
}

fn choice(runtime: Runtime, model: Option<&str>) -> OrchestratorChoice {
    OrchestratorChoice {
        runtime,
        model: model.map(str::to_string),
    }
}

#[test]
fn orchestrator_values_parse() {
    let cases = [
        ("claude", choice(Runtime::Claude, None)),
        ("codex", choice(Runtime::Codex, None)),
        (
            "claude:claude-opus-5",
            choice(Runtime::Claude, Some("claude-opus-5")),
        ),
        ("codex:gpt-5", choice(Runtime::Codex, Some("gpt-5"))),
        // `codex:` means the default model.
        ("codex:", choice(Runtime::Codex, None)),
    ];
    for (spec, want) in cases {
        assert_eq!(parse_orchestrator(spec).unwrap(), want, "{spec}");
    }
    for bad in ["", "gemini", ":x", "Claude", "claude-opus", " codex"] {
        assert_eq!(
            parse_orchestrator(bad).unwrap_err().to_string(),
            BAD_ORCHESTRATOR,
            "{bad:?}"
        );
    }
}

#[test]
fn orchestrator_applies_to_goals_only() {
    let plan = PathBuf::from("p.toml");
    assert_eq!(
        start_orchestrator(Some(&plan), Some("claude"))
            .unwrap_err()
            .to_string(),
        ORCHESTRATOR_WITH_PLAN
    );
    // The plan refusal comes first, whatever the value.
    assert_eq!(
        start_orchestrator(Some(&plan), Some("gemini"))
            .unwrap_err()
            .to_string(),
        ORCHESTRATOR_WITH_PLAN
    );
    assert_eq!(start_orchestrator(Some(&plan), None).unwrap(), None);
    assert_eq!(
        start_orchestrator(None, Some("codex")).unwrap(),
        Some(choice(Runtime::Codex, None))
    );
}

#[test]
fn message_targets_parse() {
    assert_eq!(parse_target("running").unwrap(), MessageTarget::Running);
    assert_eq!(parse_target("stage:2").unwrap(), MessageTarget::Stage(2));
    assert_eq!(
        parse_target("t1").unwrap(),
        MessageTarget::Tasks(vec!["t1".into()])
    );
    assert_eq!(
        parse_target("t1,t2").unwrap(),
        MessageTarget::Tasks(vec!["t1".into(), "t2".into()])
    );
    assert!(parse_target("stage:x").is_err());
    assert!(parse_target(",").is_err());
}

#[test]
fn message_text_is_the_words_joined_with_one_space() {
    let words: Vec<String> = ["look", "at", "the  README"]
        .iter()
        .map(|w| w.to_string())
        .collect();
    assert_eq!(
        message_edit("t1", KindArg::Change, &words).unwrap(),
        PlanEdit::Message {
            to: MessageTarget::Tasks(vec!["t1".into()]),
            text: "look at the  README".into(),
            kind: MessageKind::Change,
        }
    );
}

#[test]
fn message_kind_defaults_to_info_and_names_three_kinds() {
    let text = format!("{:?}", parse(&["message", "r", "t1", "hi"]).unwrap());
    assert!(text.contains("kind: Info"), "{text}");
    let text = format!(
        "{:?}",
        parse(&[
            "message",
            "r",
            "t1",
            "--kind",
            "stop_and_wait",
            "hi",
            "there"
        ])
        .unwrap()
    );
    assert!(text.contains("kind: StopAndWait"), "{text}");
    assert!(text.contains("[\"hi\", \"there\"]"), "{text}");
    assert_eq!(
        parse(&["message", "r", "t1", "--kind", "stop-and-wait", "hi"]).unwrap_err(),
        ErrorKind::InvalidValue
    );
    assert_eq!(
        parse(&["message", "r", "t1"]).unwrap_err(),
        ErrorKind::MissingRequiredArgument
    );
}

#[test]
fn edit_needs_a_file_or_submit() {
    assert_eq!(
        parse(&["edit", "r"]).unwrap_err(),
        ErrorKind::MissingRequiredArgument
    );
    for args in [
        &["edit", "r", "--submit"][..],
        &["edit", "r", "--file", "e.toml"],
        &["edit", "r", "--file", "e.toml", "--submit"],
    ] {
        assert!(parse(args).is_ok(), "{args:?}");
    }
}

#[test]
fn hold_flags_parse() {
    let text = format!(
        "{:?}",
        parse(&["approve", "r", "--hold", "promotion"]).unwrap()
    );
    assert!(text.contains("hold: Some(\"promotion\")"), "{text}");
    let text = format!("{:?}", parse(&["reject", "r", "--hold", "epic:c"]).unwrap());
    assert!(text.contains("hold: Some(\"epic:c\")"), "{text}");
    assert_eq!(
        parse(&["reject", "r", "--hold", "h", "--confirm", "r"]).unwrap_err(),
        ErrorKind::ArgumentConflict
    );
}
