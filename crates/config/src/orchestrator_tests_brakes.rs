//! Milestone 9.16 decisions 10, 23 and 24: `[orchestrator.brakes]` (settings-owned, U11's
//! defaults) and the three handoff keys of `[orchestrator]` (U13's defaults).

use super::*;

fn problem<'a>(problems: &'a [Problem], key: &str) -> &'a Problem {
    problems
        .iter()
        .find(|p| p.key == key)
        .unwrap_or_else(|| panic!("no problem for {key}: {problems:?}"))
}

#[test]
fn brake_keys_default_parse_and_reject() {
    let defaults = BrakeKeys {
        quota_brake: true,
        pause_5h_percent: 90,
        pause_7d_percent: 95,
        fresh_secs: 600,
    };
    let (config, problems) = parse("");
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(config.orchestrator.brakes, defaults);
    assert_eq!(BrakeKeys::default(), defaults);

    let (config, problems) = parse(
        "[orchestrator.brakes]\nquota_brake = false\nquota_pause_5h_percent = 80\nquota_pause_7d_percent = 100\nquota_fresh_secs = 86400\n",
    );
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(
        config.orchestrator.brakes,
        BrakeKeys {
            quota_brake: false,
            pause_5h_percent: 80,
            pause_7d_percent: 100,
            fresh_secs: 86_400,
        }
    );

    let (config, problems) = parse("[orchestrator.brakes]\nquota_pause_5h_percent = 0\n");
    assert_eq!(problems.len(), 1, "{problems:?}");
    assert_eq!(
        problem(&problems, "orchestrator.brakes.quota_pause_5h_percent").to_string(),
        "orchestrator.brakes.quota_pause_5h_percent: must be between 1 and 100 (using 90)"
    );
    assert_eq!(config.orchestrator.brakes.pause_5h_percent, 90);

    let (config, problems) = parse("[orchestrator.brakes]\nquota_pause_7d_percent = 101\n");
    assert_eq!(problems.len(), 1, "{problems:?}");
    problem(&problems, "orchestrator.brakes.quota_pause_7d_percent");
    assert_eq!(config.orchestrator.brakes.pause_7d_percent, 95);

    let (config, problems) = parse("[orchestrator.brakes]\nquota_fresh_secs = 30\n");
    assert_eq!(problems.len(), 1, "{problems:?}");
    assert_eq!(
        problem(&problems, "orchestrator.brakes.quota_fresh_secs").to_string(),
        "orchestrator.brakes.quota_fresh_secs: must be between 60 and 86400 (using 600)"
    );
    assert_eq!(config.orchestrator.brakes.fresh_secs, 600);

    let (config, problems) = parse("[orchestrator.brakes]\nquota_brake = \"yes\"\n");
    assert_eq!(problems.len(), 1, "{problems:?}");
    problem(&problems, "orchestrator.brakes.quota_brake");
    assert!(config.orchestrator.brakes.quota_brake);

    let (config, problems) = parse("[orchestrator.brakes]\nquota_pause = 50\n");
    assert_eq!(problems.len(), 1, "{problems:?}");
    assert_eq!(
        problem(&problems, "orchestrator.brakes.quota_pause").message,
        "unknown key, ignored"
    );
    assert_eq!(config.orchestrator.brakes, defaults);

    let (config, problems) = parse("[orchestrator]\nbrakes = 3\n");
    assert_eq!(problems.len(), 1, "{problems:?}");
    assert_eq!(
        problem(&problems, "orchestrator.brakes").message,
        "expected a table"
    );
    assert_eq!(config.orchestrator.brakes, defaults);

    // The ranges are the ones the Settings screen's `validate` reads.
    assert_eq!(QUOTA_PAUSE_PERCENT_RANGE, 1..=100);
    assert_eq!(QUOTA_FRESH_SECS_RANGE, 60..=86_400);
}

#[test]
fn handoff_keys_default_and_ranges() {
    let (config, problems) = parse("");
    assert!(problems.is_empty(), "{problems:?}");
    let o = &config.orchestrator;
    assert_eq!(o.context_window_tokens, 0);
    assert_eq!(o.handoff_at_percent, 80);
    assert_eq!(o.handoff_at_tokens, 300_000);

    let (config, problems) = parse(
        "[orchestrator]\ncontext_window_tokens = 500000\nhandoff_at_percent = 0\nhandoff_at_tokens = 0\n",
    );
    assert!(problems.is_empty(), "{problems:?}");
    let o = &config.orchestrator;
    assert_eq!(o.context_window_tokens, 500_000);
    assert_eq!(o.handoff_at_percent, 0);
    assert_eq!(o.handoff_at_tokens, 0);

    let (config, problems) = parse("[orchestrator]\nhandoff_at_percent = 101\n");
    assert_eq!(problems.len(), 1, "{problems:?}");
    assert_eq!(
        problem(&problems, "orchestrator.handoff_at_percent").to_string(),
        "orchestrator.handoff_at_percent: must be between 0 and 100 (using 80)"
    );
    assert_eq!(config.orchestrator.handoff_at_percent, 80);

    let (config, problems) =
        parse("[orchestrator]\nhandoff_at_tokens = 10000001\ncontext_window_tokens = -1\n");
    assert_eq!(problems.len(), 2, "{problems:?}");
    problem(&problems, "orchestrator.handoff_at_tokens");
    problem(&problems, "orchestrator.context_window_tokens");
    assert_eq!(config.orchestrator.handoff_at_tokens, 300_000);
    assert_eq!(config.orchestrator.context_window_tokens, 0);
}
