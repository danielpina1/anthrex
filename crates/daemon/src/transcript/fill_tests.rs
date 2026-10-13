//! Pins the shapes of the checked-in fixtures that milestone 9.16 reads, so a fixture edited to
//! suit the code fails here first. Every assertion reads only checked-in files.

use serde_json::Value;

macro_rules! fixture {
    ($path:literal) => {
        include_str!(concat!("../../tests/fixtures/", $path))
    };
}

fn lines(text: &str) -> Vec<Value> {
    text.lines()
        .map(|l| serde_json::from_str(l).expect("fixture line is JSON"))
        .collect()
}

fn of_type<'a>(all: &'a [Value], ty: &str) -> Vec<&'a Value> {
    all.iter().filter(|v| v["type"] == ty).collect()
}

fn rate_limit_infos(text: &str) -> Vec<Value> {
    lines(text)
        .iter()
        .filter(|v| v["type"] == "rate_limit_event")
        .map(|v| v["rate_limit_info"].clone())
        .collect()
}

/// Every `result.modelUsage` entry in the file as `(key, contextWindow)`.
fn context_windows(text: &str) -> Vec<(String, Option<u64>)> {
    let mut out = Vec::new();
    for line in lines(text) {
        if line["type"] != "result" {
            continue;
        }
        if let Some(map) = line["modelUsage"].as_object() {
            for (key, usage) in map {
                out.push((key.clone(), usage["contextWindow"].as_u64()));
            }
        }
    }
    out
}

/// `(line number, payload)` of every Codex `token_count` event, 1-based like `sed -n`.
fn token_counts(text: &str) -> Vec<(usize, Value)> {
    lines(text)
        .into_iter()
        .enumerate()
        .filter(|(_, v)| v["payload"]["type"] == "token_count")
        .map(|(i, v)| (i + 1, v["payload"].clone()))
        .collect()
}

#[test]
fn existing_fixtures_have_the_shapes_this_milestone_reads() {
    // `rate_limit_event` line counts, per file.
    let counts: [(&str, &str, usize); 10] = [
        (
            "headless/claude-2.1.278-documented",
            fixture!("headless/claude-2.1.278-documented.jsonl"),
            0,
        ),
        (
            "headless/claude-2.1.278-project-settings",
            fixture!("headless/claude-2.1.278-project-settings.jsonl"),
            1,
        ),
        (
            "headless/claude-2.1.278-sandbox",
            fixture!("headless/claude-2.1.278-sandbox.jsonl"),
            2,
        ),
        (
            "headless/claude-2.1.278-stream",
            fixture!("headless/claude-2.1.278-stream.jsonl"),
            1,
        ),
        (
            "headless/claude-2.1.280-filter-hook",
            fixture!("headless/claude-2.1.280-filter-hook.jsonl"),
            1,
        ),
        (
            "headless/claude-2.1.280-scout",
            fixture!("headless/claude-2.1.280-scout.jsonl"),
            1,
        ),
        (
            "headless/claude-2.1.280-scout-deny-cwd",
            fixture!("headless/claude-2.1.280-scout-deny-cwd.jsonl"),
            1,
        ),
        (
            "deciders/claude-2.1.280-decider",
            fixture!("deciders/claude-2.1.280-decider.jsonl"),
            1,
        ),
        (
            "deciders/claude-2.1.280-decider-triage",
            fixture!("deciders/claude-2.1.280-decider-triage.jsonl"),
            1,
        ),
        (
            "headless/claude-2.1.280-rate-limit",
            fixture!("headless/claude-2.1.280-rate-limit.jsonl"),
            1,
        ),
    ];
    for (name, text, want) in counts {
        assert_eq!(rate_limit_infos(text).len(), want, "{name}");
    }

    // 2.1.278: `allowed`, five_hour, both windows under `unifiedWindows`, no top-level utilization.
    for text in [
        fixture!("headless/claude-2.1.278-stream.jsonl"),
        fixture!("headless/claude-2.1.278-project-settings.jsonl"),
    ] {
        let info = &rate_limit_infos(text)[0];
        assert_eq!(info["status"], "allowed");
        assert_eq!(info["rateLimitType"], "five_hour");
        assert!(info["unifiedWindows"]["five_hour"]["utilization"].is_f64());
        assert!(info["unifiedWindows"]["seven_day"]["utilization"].is_f64());
        assert!(info.get("utilization").is_none());
    }
    let stream = &rate_limit_infos(fixture!("headless/claude-2.1.278-stream.jsonl"))[0];
    assert_eq!(stream["unifiedWindows"]["five_hour"]["utilization"], 0.05);
    assert_eq!(stream["unifiedWindows"]["seven_day"]["utilization"], 0.53);

    // 2.1.280: the warning case (the only `allowed_warning` fixtures).
    for text in [
        fixture!("headless/claude-2.1.280-scout.jsonl"),
        fixture!("headless/claude-2.1.280-scout-deny-cwd.jsonl"),
        fixture!("headless/claude-2.1.280-filter-hook.jsonl"),
        fixture!("deciders/claude-2.1.280-decider.jsonl"),
        fixture!("deciders/claude-2.1.280-decider-triage.jsonl"),
    ] {
        let info = &rate_limit_infos(text)[0];
        assert_eq!(info["status"], "allowed_warning");
        assert_eq!(info["rateLimitType"], "seven_day");
        assert_eq!(info["utilization"], 0.94);
        assert_eq!(info["surpassedThreshold"], 0.75);
        assert_eq!(info["unifiedWindows"]["seven_day"]["utilization"], 0.94);
        assert_eq!(info["unifiedWindows"]["five_hour"]["utilization"], 0.14);
    }

    // `contextWindow: 200000` in 11 `result.modelUsage` lines, all Haiku.
    let all_headless_and_deciders = [
        fixture!("headless/claude-2.1.278-documented.jsonl"),
        fixture!("headless/claude-2.1.278-project-settings.jsonl"),
        fixture!("headless/claude-2.1.278-sandbox.jsonl"),
        fixture!("headless/claude-2.1.278-stream.jsonl"),
        fixture!("headless/claude-2.1.280-filter-hook.jsonl"),
        fixture!("headless/claude-2.1.280-scout.jsonl"),
        fixture!("headless/claude-2.1.280-scout-deny-cwd.jsonl"),
        fixture!("deciders/claude-2.1.280-decider.jsonl"),
        fixture!("deciders/claude-2.1.280-decider-triage.jsonl"),
    ];
    let with_window: Vec<_> = all_headless_and_deciders
        .into_iter()
        .flat_map(context_windows)
        .filter(|(_, w)| w.is_some())
        .collect();
    assert_eq!(with_window.len(), 11);
    assert!(
        with_window
            .iter()
            .all(|(k, w)| k.starts_with("claude-haiku-4-5") && *w == Some(200_000)),
        "{with_window:?}"
    );

    // Codex 0.155.0 rollout: three `token_count` lines (17, 22, 26), each with the same window
    // and weekly limit (pre-flight ruling C1; the brief's "exactly two" was wrong).
    let codex = token_counts(fixture!("transcripts/codex-0.155.0.jsonl"));
    assert_eq!(
        codex.iter().map(|(n, _)| *n).collect::<Vec<_>>(),
        [17, 22, 26]
    );
    for (n, p) in &codex {
        assert_eq!(p["info"]["model_context_window"], 258_400, "line {n}");
        assert_eq!(
            p["rate_limits"]["primary"]["window_minutes"], 10_080,
            "line {n}"
        );
        assert_eq!(
            p["rate_limits"]["primary"]["used_percent"], 99.0,
            "line {n}"
        );
        assert!(p["rate_limits"]["secondary"].is_null(), "line {n}");
    }
    let totals: Vec<_> = codex
        .iter()
        .map(|(_, p)| {
            p["info"]["last_token_usage"]["total_tokens"]
                .as_u64()
                .unwrap()
        })
        .collect();
    assert_eq!(totals, [22_609, 23_193, 23_276]);

    // Claude 2.1.278 transcript: the last non-sidechain assistant line is line 46.
    let claude = lines(fixture!("transcripts/claude-2.1.278.jsonl"));
    let last = claude
        .iter()
        .enumerate()
        .filter(|(_, v)| v["type"] == "assistant" && v["isSidechain"] != true)
        .map(|(i, _)| i + 1)
        .next_back();
    assert_eq!(last, Some(46));
    let usage = &claude[45]["message"]["usage"];
    assert_eq!(usage["input_tokens"], 2);
    assert_eq!(usage["cache_creation_input_tokens"], 3189);
    assert_eq!(usage["cache_read_input_tokens"], 45_833);
    assert_eq!(usage["output_tokens"], 4);
}

#[test]
fn the_2_1_280_captures_pin_the_one_million_window_shapes() {
    // headless: `--model 'opus[1m]'`; `modelUsage` is keyed with the `[1m]` tag, and the same
    // tag is on `system/init.model`, but not on the assistant line's `message.model`.
    let headless = lines(fixture!("headless/claude-2.1.280-rate-limit.jsonl"));
    let info = &rate_limit_infos(fixture!("headless/claude-2.1.280-rate-limit.jsonl"))[0];
    // The map is ordered by key, so the expected list is alphabetical.
    let keys: Vec<_> = info
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        keys,
        [
            "isUsingOverage",
            "overageDisabledReason",
            "overageStatus",
            "rateLimitType",
            "resetsAt",
            "status",
            "unifiedWindows"
        ]
    );
    assert_eq!(info["status"], "allowed");
    assert_eq!(info["unifiedWindows"]["five_hour"]["utilization"], 0.16);
    assert_eq!(info["unifiedWindows"]["seven_day"]["utilization"], 0.36);
    assert_eq!(
        context_windows(fixture!("headless/claude-2.1.280-rate-limit.jsonl")),
        [("claude-opus-5-5[1m]".to_string(), Some(1_000_000))]
    );
    let result = of_type(&headless, "result")[0];
    assert_eq!(
        result["modelUsage"]["claude-opus-5-5[1m]"]["canonicalModel"],
        "claude-opus-5-5"
    );
    assert_eq!(
        of_type(&headless, "system")
            .iter()
            .find(|v| v["subtype"] == "init")
            .unwrap()["model"],
        "claude-opus-5-5[1m]"
    );
    let assistant = of_type(&headless, "assistant")[0];
    assert_eq!(assistant["message"]["model"], "claude-opus-5-5");

    // transcript: two assistant lines (22 and 39), each with the four token fields, never
    // sidechain; `message.model` carries no `[1m]` tag.
    let transcript = lines(fixture!("transcripts/claude-2.1.280-usage.jsonl"));
    let assistants: Vec<_> = transcript
        .iter()
        .enumerate()
        .filter(|(_, v)| v["type"] == "assistant")
        .map(|(i, v)| (i + 1, v))
        .collect();
    assert_eq!(
        assistants.iter().map(|(n, _)| *n).collect::<Vec<_>>(),
        [22, 39]
    );
    let want = [(22_795, 21_175), (27_395, 23_356)];
    for ((n, v), (create, read)) in assistants.iter().zip(want) {
        let usage = &v["message"]["usage"];
        assert_eq!(usage["input_tokens"], 2, "line {n}");
        assert_eq!(usage["cache_creation_input_tokens"], create, "line {n}");
        assert_eq!(usage["cache_read_input_tokens"], read, "line {n}");
        assert_eq!(usage["output_tokens"], 4, "line {n}");
        assert_eq!(v["message"]["model"], "claude-opus-5-5", "line {n}");
        assert_eq!(v["isSidechain"], false, "line {n}");
    }
    assert!(transcript.iter().all(|v| v["isSidechain"] != true));

    // Codex 0.160.1 rollout: one `token_count` (line 14), a 475000-token window, a weekly
    // primary window, no secondary, and the same `rate_limits` key set as 0.155.0.
    let codex = token_counts(fixture!("transcripts/codex-0.160.1-rollout-limits.jsonl"));
    assert_eq!(codex.len(), 1);
    let (n, p) = &codex[0];
    assert_eq!(*n, 14);
    assert_eq!(p["info"]["model_context_window"], 475_000);
    let last = &p["info"]["last_token_usage"];
    assert_eq!(last["input_tokens"], 23_016);
    assert_eq!(last["cached_input_tokens"], 13_184);
    assert_eq!(last["cache_write_input_tokens"], 0);
    assert_eq!(last["output_tokens"], 5);
    assert_eq!(last["total_tokens"], 23_021);
    let limits = &p["rate_limits"];
    assert_eq!(limits["primary"]["window_minutes"], 10_080);
    assert_eq!(limits["primary"]["used_percent"], 0.0);
    assert!(limits["secondary"].is_null());
    let keys: Vec<_> = limits
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        keys,
        [
            "credits",
            "individual_limit",
            "limit_id",
            "limit_name",
            "plan_type",
            "primary",
            "rate_limit_reached_type",
            "secondary",
            "spend_control_reached"
        ]
    );
    let old = token_counts(fixture!("transcripts/codex-0.155.0.jsonl"));
    let old_keys: Vec<_> = old[0].1["rate_limits"]
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        old_keys, keys,
        "0.160.1 added no rate_limits key over 0.155.0"
    );
}

// Milestone 9.16 decisions 5, 22 and 24: the pure readers over these fixtures.

mod readers {
    use super::super::{Fill, claude_fill, codex_fill, codex_limits, fill_percent, handoff_due};
    use crate::headless::quota::QuotaSample;
    use proto::brakes::Permille;
    use proto::brakes::QuotaWindow::{FiveHour, SevenDay};
    use serde_json::{Value, json};

    const CLAUDE_278: &str = fixture!("transcripts/claude-2.1.278.jsonl");
    const CLAUDE_280: &str = fixture!("transcripts/claude-2.1.280-usage.jsonl");
    const CODEX_155: &str = fixture!("transcripts/codex-0.155.0.jsonl");
    const CODEX_160: &str = fixture!("transcripts/codex-0.160.1-rollout-limits.jsonl");

    /// The fixture's last `token_count` line with its payload changed by `edit`.
    fn token_count_with(fixture: &str, edit: impl FnOnce(&mut Value)) -> String {
        let line = fixture
            .lines()
            .rfind(|l| l.contains(r#""type":"token_count""#))
            .unwrap();
        let mut value: Value = serde_json::from_str(line).unwrap();
        edit(&mut value["payload"]);
        value.to_string()
    }

    #[test]
    fn claude_fill_counts_cache_writes_and_output() {
        // Line 46: 2 + 45,833 + 3,189 + 4.
        assert_eq!(
            claude_fill(CLAUDE_278),
            Some(Fill {
                tokens: 49_028,
                window: None,
                model: Some("claude-opus-5".into()),
            })
        );
        // 2.1.280, line 39: 2 + 23,356 + 27,395 + 4; the model has no `[1m]` tag.
        assert_eq!(
            claude_fill(CLAUDE_280),
            Some(Fill {
                tokens: 50_757,
                window: None,
                model: Some("claude-opus-5-5".into()),
            })
        );
    }

    #[test]
    fn claude_fill_skips_sidechain_and_cut_lines() {
        let lines: Vec<&str> = CLAUDE_278.lines().collect();
        let mut sidechain: Value = serde_json::from_str(lines[45]).unwrap();
        sidechain["isSidechain"] = json!(true);
        sidechain["message"]["usage"]["cache_read_input_tokens"] = json!(900_000);
        // The tail starts in the middle of line 44 and ends with a sub-agent's line.
        let start = CLAUDE_278.find(lines[43]).unwrap() + lines[43].len() / 2;
        let tail = format!("{}\n{sidechain}\n", &CLAUDE_278[start..]);
        assert!(serde_json::from_str::<Value>(tail.lines().next().unwrap()).is_err());
        assert_eq!(claude_fill(&tail).map(|f| f.tokens), Some(49_028));

        // A line that does not parse, after the last assistant line, is skipped.
        let broken = format!("{CLAUDE_278}{{\"type\":\"assistant\",\"message\":{{\n");
        assert_eq!(claude_fill(&broken).map(|f| f.tokens), Some(49_028));

        // No main-chain assistant line, or one whose usage lacks a field: no reading.
        assert_eq!(claude_fill(&format!("{sidechain}\n")), None);
        assert_eq!(claude_fill(""), None);
        for field in [
            "input_tokens",
            "cache_creation_input_tokens",
            "cache_read_input_tokens",
            "output_tokens",
        ] {
            let mut last: Value = serde_json::from_str(lines[45]).unwrap();
            last["message"]["usage"][field] = json!("x");
            assert_eq!(
                claude_fill(&format!("{CLAUDE_278}{last}\n")),
                None,
                "{field}"
            );
        }
    }

    #[test]
    fn codex_fill_reads_last_token_usage_and_window() {
        // Line 26, not line 22's 23,193 (decision 22): input + output, cached inside input.
        assert_eq!(
            codex_fill(CODEX_155),
            Some(Fill {
                tokens: 23_276,
                window: Some(258_400),
                model: None,
            })
        );
        // The window is the line's own: 0.160.1 says 475,000.
        assert_eq!(
            codex_fill(CODEX_160),
            Some(Fill {
                tokens: 23_021,
                window: Some(475_000),
                model: None,
            })
        );
        // No window on the line: the tokens still count, the window is unknown.
        let line = token_count_with(CODEX_155, |p| {
            p["info"]
                .as_object_mut()
                .unwrap()
                .remove("model_context_window");
        });
        assert_eq!(
            codex_fill(&line),
            Some(Fill {
                tokens: 23_276,
                window: None,
                model: None,
            })
        );
        // A `token_count` with no usage yet (`info: null`) leaves the last one standing.
        let empty = token_count_with(CODEX_155, |p| p["info"] = Value::Null);
        assert_eq!(
            codex_fill(&format!("{CODEX_155}{empty}\n")).map(|f| f.tokens),
            Some(23_276)
        );
        assert_eq!(codex_fill(&format!("{empty}\n")), None);
        // A changed usage shape is no reading.
        let changed = token_count_with(CODEX_155, |p| {
            p["info"]["last_token_usage"]["output_tokens"] = json!("5");
        });
        assert_eq!(codex_fill(&changed), None);
        assert_eq!(codex_fill(CLAUDE_278), None);
    }

    #[test]
    fn codex_limits_maps_by_window_minutes() {
        assert_eq!(
            codex_limits(CODEX_155),
            Some(QuotaSample {
                windows: vec![(SevenDay, Permille(990), Some(1_790_332_405))],
                rejected: None,
            })
        );
        // Mapped by length, never by position.
        let five = json!({"used_percent": 12.5, "window_minutes": 300, "resets_at": 100});
        let seven = json!({"used_percent": 40.0, "window_minutes": 10080, "resets_at": 200});
        let want = QuotaSample {
            windows: vec![
                (FiveHour, Permille(125), Some(100)),
                (SevenDay, Permille(400), Some(200)),
            ],
            rejected: None,
        };
        for (primary, secondary) in [(&five, &seven), (&seven, &five)] {
            let line = token_count_with(CODEX_155, |p| {
                p["rate_limits"]["primary"] = primary.clone();
                p["rate_limits"]["secondary"] = secondary.clone();
            });
            assert_eq!(codex_limits(&line), Some(want.clone()), "{line}");
        }
        // No limits, an unknown window, or an unreadable percentage: no reading.
        for edit in [
            json!(null),
            json!({"primary": null, "secondary": null}),
            json!({"primary": {"used_percent": 50.0, "window_minutes": 60}, "secondary": null}),
            json!({"primary": {"used_percent": "50", "window_minutes": 300}, "secondary": null}),
            json!({"primary": {"used_percent": -1.0, "window_minutes": 300}, "secondary": null}),
        ] {
            let line = token_count_with(CODEX_155, |p| p["rate_limits"] = edit.clone());
            assert_eq!(codex_limits(&line), None, "{line}");
        }
        assert_eq!(codex_limits(CLAUDE_278), None);
    }

    #[test]
    fn handoff_due_whichever_first() {
        type Case = (u64, Option<u64>, u8, u64);
        let cases: [(Case, bool); 7] = [
            ((240_000, Some(1_000_000), 80, 300_000), false),
            ((300_000, Some(1_000_000), 80, 300_000), true),
            ((160_000, Some(200_000), 80, 300_000), true),
            ((159_999, Some(200_000), 80, 300_000), false),
            ((400_000, None, 80, 300_000), true),
            ((400_000, None, 80, 0), false),
            ((999_999, Some(200_000), 0, 0), false),
        ];
        for ((tokens, window, percent, cap), want) in cases {
            assert_eq!(
                handoff_due(tokens, window, percent, cap),
                want,
                "{tokens} {window:?} {percent} {cap}"
            );
        }
        // No overflow at the extremes.
        assert!(handoff_due(u64::MAX, Some(u64::MAX), 100, 0));
        assert_eq!(fill_percent(160_000, 200_000), 80);
        assert_eq!(fill_percent(23_276, 258_400), 9);
        assert_eq!(fill_percent(1, 0), 0);
    }
}
