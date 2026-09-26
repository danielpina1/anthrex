//! Milestone 8b decision 28: the output-filter hook in a Claude worker's settings.

use super::*;
use crate::output_filter::{FilterHook, hook_command};
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
    // Codex has no settings hook: even a spec that carried one puts nothing on the argv.
    let mut codex_worker = worker(Runtime::Codex);
    codex_worker.output_filter = Some(filter());
    let argv = codex(&codex_worker, &new_session(), "go", &CLI_CAPS);
    assert!(
        !argv
            .iter()
            .any(|a| a.contains("filter-hook") || a.contains("filter-run")),
        "{argv:?}"
    );
    assert_eq!(
        argv,
        codex(&worker(Runtime::Codex), &new_session(), "go", &CLI_CAPS)
    );
}
