//! Milestone 9.16 decisions 2–4: a Claude `rate_limit_event` read through the stream parser,
//! on the checked-in captures. A test that needs another shape edits a copy of a fixture
//! line here, never the fixture.

use super::*;
use crate::headless::SessionEvent;
use crate::headless::claude_stream::ClaudeStream;
use crate::headless::test_support::{CLAUDE_STREAM, line_type, lines};
use proto::brakes::QuotaWindow::{FiveHour, SevenDay};
use serde_json::{Value, json};

const SCOUT_280: &str = include_str!("../../tests/fixtures/headless/claude-2.1.280-scout.jsonl");
const CAPTURE_280: &str =
    include_str!("../../tests/fixtures/headless/claude-2.1.280-rate-limit.jsonl");

/// The fixture's one `rate_limit_event` line.
fn rate_limit_line(fixture: &str) -> &str {
    lines(fixture)
        .into_iter()
        .find(|line| line_type(line) == "rate_limit_event")
        .expect("the fixture has a rate_limit_event")
}

/// The line through the stream parser, which must give exactly one `Quota`.
fn quota(line: &str) -> QuotaSample {
    match ClaudeStream::default().parse_line(line).as_slice() {
        [SessionEvent::Quota(sample)] => sample.clone(),
        other => panic!("{other:#?}"),
    }
}

/// `line` with its `rate_limit_info` changed by `edit`.
fn edited(line: &str, edit: impl FnOnce(&mut Value)) -> String {
    let mut value: Value = serde_json::from_str(line).unwrap();
    edit(&mut value["rate_limit_info"]);
    value.to_string()
}

#[test]
fn rate_limit_event_yields_per_mille_not_percent() {
    // 0.05 and 0.53 in the 2.1.278 capture: 50 ‰ and 530 ‰, not 53 (a percentage) nor 0
    // (a truncated fraction).
    assert_eq!(
        quota(rate_limit_line(CLAUDE_STREAM)),
        QuotaSample {
            windows: vec![
                (FiveHour, Permille(50), Some(1_790_154_600)),
                (SevenDay, Permille(530), Some(1_790_456_400)),
            ],
            rejected: None,
        }
    );
    // 2.1.280 (`--model 'opus[1m]'`) still carries `unifiedWindows`.
    assert_eq!(
        quota(rate_limit_line(CAPTURE_280)),
        QuotaSample {
            windows: vec![
                (FiveHour, Permille(160), Some(1_791_930_600)),
                (SevenDay, Permille(360), Some(1_792_270_800)),
            ],
            rejected: None,
        }
    );
}

#[test]
fn allowed_warning_is_read_like_allowed() {
    assert_eq!(
        quota(rate_limit_line(SCOUT_280)),
        QuotaSample {
            windows: vec![
                (FiveHour, Permille(140), Some(1_790_375_400)),
                (SevenDay, Permille(940), Some(1_790_456_400)),
            ],
            rejected: None,
        }
    );
    // The warning's top-level `utilization` and `rateLimitType` alone, without
    // `unifiedWindows`, still read as that window.
    let line = edited(rate_limit_line(SCOUT_280), |info| {
        info.as_object_mut().unwrap().remove("unifiedWindows");
    });
    assert_eq!(
        quota(&line),
        QuotaSample {
            windows: vec![(SevenDay, Permille(940), Some(1_790_456_400))],
            rejected: None,
        }
    );
}

#[test]
fn rejected_status_counts_its_window_as_full() {
    let line = edited(rate_limit_line(SCOUT_280), |info| {
        info["status"] = json!("rejected");
    });
    assert_eq!(
        quota(&line),
        QuotaSample {
            windows: vec![
                (FiveHour, Permille(140), Some(1_790_375_400)),
                (SevenDay, Permille(940), Some(1_790_456_400)),
            ],
            rejected: Some(SevenDay),
        }
    );
    // A rejection with no utilization at all is still a reading: its window is full until
    // the line's `resetsAt`.
    let bare = r#"{"type":"rate_limit_event","rate_limit_info":{"status":"rejected","rateLimitType":"five_hour","resetsAt":1790154600}}"#;
    assert_eq!(
        quota(bare),
        QuotaSample {
            windows: vec![(FiveHour, Permille::FULL, Some(1_790_154_600))],
            rejected: Some(FiveHour),
        }
    );
}

#[test]
fn rate_limit_event_without_windows_stays_other() {
    let other = vec![SessionEvent::Other {
        kind: "rate_limit_event".into(),
    }];
    for line in [
        r#"{"type":"rate_limit_event","rate_limit_info":{"status":"allowed"}}"#,
        r#"{"type":"rate_limit_event"}"#,
        r#"{"type":"rate_limit_event","rate_limit_info":null}"#,
        r#"{"type":"rate_limit_event","rate_limit_info":{"status":"allowed","unifiedWindows":{"seven_day_opus":{"utilization":0.5}}}}"#,
        // A rejection naming a window this milestone does not know is no reading.
        r#"{"type":"rate_limit_event","rate_limit_info":{"status":"rejected","rateLimitType":"overage"}}"#,
    ] {
        assert_eq!(ClaudeStream::default().parse_line(line), other, "{line}");
    }

    // A utilization that is not a number, or is negative, drops that window: never 0.
    for bad in [json!("x"), json!(-1), json!(null)] {
        let line = edited(rate_limit_line(CLAUDE_STREAM), |info| {
            info["unifiedWindows"]["five_hour"]["utilization"] = bad.clone();
        });
        assert_eq!(
            quota(&line).windows,
            vec![(SevenDay, Permille(530), Some(1_790_456_400))],
            "{bad}"
        );
        let both = edited(&line, |info| {
            info["unifiedWindows"]["seven_day"]["utilization"] = bad.clone();
        });
        assert_eq!(ClaudeStream::default().parse_line(&both), other, "{bad}");
    }
    // A missing or unreadable `resetsAt` keeps the window, with no reset time.
    let line = edited(rate_limit_line(CLAUDE_STREAM), |info| {
        info["unifiedWindows"]["seven_day"]["resetsAt"] = json!("soon");
    });
    assert_eq!(
        quota(&line).windows[1],
        (SevenDay, Permille(530), None),
        "{line}"
    );
}

/// The patterns decision 2 forbids outside `proto/src/brakes.rs`: a usage field scaled by
/// hand. Built from pieces so this file does not match itself.
fn unit_violations(text: &str) -> Vec<String> {
    let scales = [
        ["* ", "1000.0"].concat(),
        ["*", "1000.0"].concat(),
        ["* ", "10.0"].concat(),
        ["/ ", "100.0"].concat(),
    ];
    let fields = [
        ["utili", "zation"].concat(),
        ["used", "_percent"].concat(),
        ["us", "ed"].concat(),
    ];
    text.lines()
        .filter(|line| scales.iter().any(|s| line.contains(s.as_str())))
        .filter(|line| fields.iter().any(|f| has_word(line, f)))
        .map(str::to_owned)
        .collect()
}

/// Whether `word` appears in `line` as a whole identifier (or a field access's last part).
fn has_word(line: &str, word: &str) -> bool {
    line.match_indices(word).any(|(i, _)| {
        let ident = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() || c == '_');
        !ident(line[..i].chars().next_back()) && !ident(line[i + word.len()..].chars().next())
    })
}

#[test]
fn units_convert_in_one_place() {
    // The check itself sees a hand-scaled field.
    let scaled = ["let p = info.utili", "zation * 1", "000.0;"].concat();
    assert_eq!(unit_violations(&scaled), std::slice::from_ref(&scaled));
    let percent = ["let p = limits.used", "_percent / 1", "00.0;"].concat();
    assert_eq!(unit_violations(&percent).len(), 1);
    assert!(unit_violations("let x = reused * 10.0;").is_empty());

    let crates = testexec::checkout_dir(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("the daemon crate sits in crates/")
        .to_path_buf();
    let mut stack = vec![crates.clone()];
    let mut seen = 0;
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).expect("a readable directory") {
            let path = entry.expect("an entry").path();
            if path.is_dir() {
                if path.file_name().is_some_and(|n| n == "target") {
                    continue;
                }
                stack.push(path);
                continue;
            }
            if path.extension().is_none_or(|e| e != "rs") {
                continue;
            }
            seen += 1;
            let rel = path
                .strip_prefix(&crates)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            if rel == "proto/src/brakes.rs" {
                continue;
            }
            let text = std::fs::read_to_string(&path).expect("a source file");
            let found = unit_violations(&text);
            assert!(found.is_empty(), "{rel} scales a usage field: {found:?}");
        }
    }
    assert!(seen > 300, "the walk found the sources ({seen})");
}
