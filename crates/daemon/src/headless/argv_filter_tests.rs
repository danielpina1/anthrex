//! Milestone 8b decision 28: the output-filter hook in a Claude worker's settings.

use super::*;
use crate::output_filter::{FilterHook, codex_filter_note, hook_command};
use proto::OutputFilter;

fn filter() -> FilterHook {
    FilterHook {
        mode: OutputFilter::FailuresOnly,
        prefixes: vec!["cargo test".into(), "cargo build".into()],
        log_dir: PathBuf::from("/tmp/ax/t1/anthrex-logs"),
    }
}

#[test]
fn claude_worker_settings_include_the_filter_hook() {
    let plain = worker(Runtime::Claude);
    let mut filtered = plain.clone();
    filtered.output_filter = Some(filter());
    let before = json_after(&claude(&plain, &new_session(), &CLI_CAPS), "--settings");
    let after = json_after(&claude(&filtered, &new_session(), &CLI_CAPS), "--settings");
    let groups = after["hooks"]["PreToolUse"].as_array().expect("groups");
    assert_eq!(groups.len(), 2, "{after}");
    assert_eq!(groups[0], before["hooks"]["PreToolUse"][0]);
    assert_eq!(
        groups[1],
        json!({"matcher": "Bash", "hooks": [{"type": "command",
            "command": hook_command(Path::new(EXE), &filter())}]})
    );
    // Only that group is new: the sandbox block, its pins and every other flag are
    // untouched.
    let mut expected = before.clone();
    expected["hooks"]["PreToolUse"] = json!([groups[0], groups[1]]);
    assert_eq!(after, expected);
    assert_eq!(after["sandbox"], sandbox_block());
    let strip = |argv: Vec<String>| {
        let at = argv.iter().position(|a| a == "--settings").unwrap();
        [&argv[..=at], &argv[at + 2..]].concat()
    };
    assert_eq!(
        strip(claude(&filtered, &new_session(), &CLI_CAPS)),
        strip(claude(&plain, &new_session(), &CLI_CAPS))
    );
}

#[test]
fn reviewers_scouts_and_codex_get_no_filter_hook() {
    // A spec without a hook has only M3's group.
    for spec in [worker(Runtime::Claude), reviewer(Runtime::Claude)] {
        let settings = json_after(&claude(&spec, &new_session(), &CLI_CAPS), "--settings");
        assert_eq!(
            settings["hooks"]["PreToolUse"].as_array().unwrap().len(),
            1,
            "{settings}"
        );
    }
    // Codex has no settings hook: a spec that carries one gets no `filter-hook` on
    // its argv (milestone 9.5 decision 28 gives it the instruction note instead).
    let mut codex_worker = worker(Runtime::Codex);
    codex_worker.output_filter = Some(filter());
    let argv = codex(&codex_worker, &new_session(), "go", &CLI_CAPS);
    assert!(!argv.iter().any(|a| a.contains("filter-hook")), "{argv:?}");
}

/// The `developer_instructions=` value of a Codex argv.
fn developer_instructions(argv: &[String]) -> String {
    let all: Vec<&str> = argv
        .iter()
        .filter_map(|a| a.strip_prefix("developer_instructions="))
        .collect();
    assert_eq!(all.len(), 1, "{argv:?}");
    all[0].to_string()
}

/// Milestone 9.5 decision 28: with `CLI_CAPS.codex_filter` `Instruction`, a Codex
/// spec that carries a filter hook gets `codex_filter_note` after its instructions, on
/// its first turn and on every resumed one; nothing else on the argv changes, and a
/// spec without a hook keeps its instructions as they are.
#[test]
fn codex_args_put_the_filter_note_after_the_instructions() {
    assert_eq!(CLI_CAPS.codex_filter, CodexFilter::Instruction);
    let plain = worker(Runtime::Codex);
    let mut filtered = plain.clone();
    filtered.output_filter = Some(filter());
    let expected = format!(
        "{}\n\n{}",
        plain.instructions,
        codex_filter_note(Path::new(EXE), &filter())
    );
    let resume = SessionArg::Resume {
        session_id: "s-1".into(),
    };
    for session in [new_session(), resume] {
        let before = codex(&plain, &session, "go", &CLI_CAPS);
        let after = codex(&filtered, &session, "go", &CLI_CAPS);
        assert_eq!(
            developer_instructions(&before),
            toml_string(&plain.instructions)
        );
        assert_eq!(developer_instructions(&after), toml_string(&expected));
        let rest = |argv: &[String]| -> Vec<String> {
            argv.iter()
                .filter(|a| !a.starts_with("developer_instructions="))
                .cloned()
                .collect()
        };
        assert_eq!(rest(&after), rest(&before));
    }
}
