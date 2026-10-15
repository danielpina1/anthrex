//! Task M9.15.3: the `[lessons]` table (decision 31).

use super::super::*;

#[test]
fn defaults_match_the_brief() {
    let (config, problems) = parse("");
    assert!(problems.is_empty(), "{problems:?}");
    let l = &config.lessons;
    assert!(l.enabled);
    assert!(l.distill);
    assert!(l.risk_note);
    assert_eq!(l.block_chars, 4000);
    assert_eq!(l.item_chars, 600);
    assert_eq!(l.max_items, 5);
    assert_eq!(l.dossier_chars, 12000);
    assert_eq!(l.episodic_trial_runs, 20);
    assert_eq!(l.known_min, 2);
    assert_eq!(l.known_window_days, 14);
    assert_eq!(l.known_clean_runs, 5);
    assert_eq!(l.expiry_days, 90);
    assert_eq!(l.max_distills_per_day, 20);
    assert_eq!(*l, Lessons::default());
    // An empty table is every default too.
    let (config, problems) = parse("[lessons]\n");
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(config.lessons, Lessons::default());
}

/// Every ranged key with its range's two ends.
const RANGES: &[(&str, u32, u32)] = &[
    ("block_chars", 500, 20000),
    ("item_chars", 100, 2000),
    ("max_items", 1, 10),
    ("dossier_chars", 2000, 50000),
    ("episodic_trial_runs", 1, 200),
    ("known_min", 2, 10),
    ("known_window_days", 1, 365),
    ("known_clean_runs", 1, 50),
    ("expiry_days", 1, 3650),
    ("max_distills_per_day", 0, 500),
];

fn field(l: &Lessons, key: &str) -> u32 {
    match key {
        "block_chars" => l.block_chars,
        "item_chars" => l.item_chars,
        "max_items" => l.max_items,
        "dossier_chars" => l.dossier_chars,
        "episodic_trial_runs" => l.episodic_trial_runs,
        "known_min" => l.known_min,
        "known_window_days" => l.known_window_days,
        "known_clean_runs" => l.known_clean_runs,
        "expiry_days" => l.expiry_days,
        "max_distills_per_day" => l.max_distills_per_day,
        other => panic!("no key {other}"),
    }
}

#[test]
fn each_key_reads_at_both_ends_of_its_range() {
    for &(key, low, high) in RANGES {
        for value in [low, high] {
            let (config, problems) = parse(&format!("[lessons]\n{key} = {value}\n"));
            assert!(problems.is_empty(), "{key} = {value}: {problems:?}");
            assert_eq!(field(&config.lessons, key), value, "{key}");
        }
    }
    let (config, problems) =
        parse("[lessons]\nenabled = false\ndistill = false\nrisk_note = false\n");
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(
        config.lessons,
        Lessons {
            enabled: false,
            distill: false,
            risk_note: false,
            ..Lessons::default()
        }
    );
}

#[test]
fn each_key_out_of_range_is_a_problem_and_keeps_its_default() {
    for &(key, low, high) in RANGES {
        let mut outside = vec![i64::from(high) + 1, -1];
        if low > 0 {
            outside.push(i64::from(low) - 1);
        }
        for value in outside {
            let (config, problems) = parse(&format!("[lessons]\n{key} = {value}\n"));
            assert_eq!(config.lessons, Lessons::default(), "{key} = {value}");
            assert_eq!(problems.len(), 1, "{key} = {value}: {problems:?}");
            assert_eq!(problems[0].key, format!("lessons.{key}"));
            assert_eq!(
                problems[0].message,
                format!("must be between {low} and {high}")
            );
            assert_eq!(
                problems[0].default,
                field(&Lessons::default(), key).to_string()
            );
        }
    }
    let (_, problems) = parse("[lessons]\nmax_items = 11\n");
    assert_eq!(
        problems[0].to_string(),
        "lessons.max_items: must be between 1 and 10 (using 5)"
    );
    // A wrong type is the same problem; a wrong-typed bool keeps its default.
    let (config, problems) = parse("[lessons]\nblock_chars = \"lots\"\nenabled = 1\n");
    assert_eq!(config.lessons, Lessons::default());
    let mut keys: Vec<&str> = problems.iter().map(|p| p.key.as_str()).collect();
    keys.sort();
    assert_eq!(keys, ["lessons.block_chars", "lessons.enabled"]);
    let enabled = problems
        .iter()
        .find(|p| p.key == "lessons.enabled")
        .unwrap();
    assert_eq!(
        enabled.to_string(),
        "lessons.enabled: must be true or false (using true)"
    );
}

#[test]
fn unknown_key_is_reported_and_ignored() {
    let (config, problems) = parse("[lessons]\nmax_items = 3\nfrobnicate = true\n");
    assert_eq!(config.lessons.max_items, 3);
    assert_eq!(problems.len(), 1, "{problems:?}");
    assert_eq!(problems[0].key, "lessons.frobnicate");
    assert_eq!(problems[0].message, "unknown key, ignored");
}

#[test]
fn not_a_table_is_a_problem() {
    let (config, problems) = parse("lessons = true\n");
    assert_eq!(config.lessons, Lessons::default());
    assert_eq!(problems.len(), 1, "{problems:?}");
    assert_eq!(problems[0].key, "lessons");
    assert_eq!(problems[0].message, "expected a table");
}
