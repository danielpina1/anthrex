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
