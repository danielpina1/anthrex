//! Milestone 9.1 task M9.1.4: the `[testing]` table.

use super::super::*;

fn keys(problems: &[Problem]) -> Vec<String> {
    let mut keys: Vec<String> = problems.iter().map(|p| p.key.clone()).collect();
    keys.sort();
    keys
}

#[test]
fn testing_defaults() {
    let (config, problems) = parse("");
    assert!(problems.is_empty());
    let t = &config.testing;
    assert_eq!(t.test_slots, None);
    assert_eq!(t.full_idle_secs, 120);
    assert_eq!(t.test_cache_days, 14);
    assert_eq!(t.flaky_quarantine_after, 3);
    assert_eq!(t.flaky_window_days, 14);
    assert_eq!(t.bisect_fix_max, 2);
    assert_eq!(*t, Testing::default());

    // An M9 config, which has no `[testing]` table, loads exactly as before with the
    // defaults and no problem.
    let m9 = "prefix = \"C-a\"\n[orchestrator]\nmax_writers = 4\n[git]\npoll_secs = 60\n";
    let (config, problems) = parse(m9);
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(config.testing, Testing::default());
    assert_eq!(config.orchestrator.max_writers, 4);
}

#[test]
fn testing_keys_are_read_and_ranged() {
    // Every key at both ends of its range is read.
    let low = "[testing]\ntest_slots = 1\nfull_idle_secs = 10\ntest_cache_days = 0\n\
               flaky_quarantine_after = 1\nflaky_window_days = 1\nbisect_fix_max = 0\n";
    let (config, problems) = parse(low);
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(
        config.testing,
        Testing {
            test_slots: Some(1),
            full_idle_secs: 10,
            test_cache_days: 0,
            flaky_quarantine_after: 1,
            flaky_window_days: 1,
            bisect_fix_max: 0,
        }
    );
    let high = "[testing]\ntest_slots = 256\nfull_idle_secs = 86400\ntest_cache_days = 365\n\
                flaky_quarantine_after = 100\nflaky_window_days = 365\nbisect_fix_max = 10\n";
    let (config, problems) = parse(high);
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(
        config.testing,
        Testing {
            test_slots: Some(256),
            full_idle_secs: 86_400,
            test_cache_days: 365,
            flaky_quarantine_after: 100,
            flaky_window_days: 365,
            bisect_fix_max: 10,
        }
    );

    // Each key just outside its range is a problem in M6's format, the default kept.
    let out = "[testing]\ntest_slots = 0\nfull_idle_secs = 9\ntest_cache_days = 366\n\
               flaky_quarantine_after = 0\nflaky_window_days = 366\nbisect_fix_max = 11\n";
    let (config, problems) = parse(out);
    assert_eq!(config.testing, Testing::default());
    let texts: Vec<String> = problems.iter().map(|p| p.to_string()).collect();
    for expected in [
        "testing.test_slots: must be between 1 and 256 (using logical cores minus 2, at least 1)",
        "testing.full_idle_secs: must be between 10 and 86400 (using 120)",
        "testing.test_cache_days: must be between 0 and 365 (using 14)",
        "testing.flaky_quarantine_after: must be between 1 and 100 (using 3)",
        "testing.flaky_window_days: must be between 1 and 365 (using 14)",
        "testing.bisect_fix_max: must be between 0 and 10 (using 2)",
    ] {
        assert!(
            texts.iter().any(|t| t == expected),
            "{expected} in {texts:?}"
        );
    }
    assert_eq!(problems.len(), 6, "{texts:?}");

    // Values too large for the field's type, and negatives, are out of range too.
    let (config, problems) =
        parse("[testing]\ntest_slots = 4294967297\nbisect_fix_max = 266\nfull_idle_secs = -1\n");
    assert_eq!(config.testing, Testing::default());
    assert_eq!(
        keys(&problems),
        [
            "testing.bisect_fix_max",
            "testing.full_idle_secs",
            "testing.test_slots"
        ]
    );

    // Each wrong type is a problem, the default kept.
    let wrong = "[testing]\ntest_slots = \"4\"\nfull_idle_secs = 1.5\ntest_cache_days = true\n\
                 flaky_quarantine_after = [3]\nflaky_window_days = \"x\"\nbisect_fix_max = {}\n";
    let (config, problems) = parse(wrong);
    assert_eq!(config.testing, Testing::default());
    assert_eq!(
        keys(&problems),
        [
            "testing.bisect_fix_max",
            "testing.flaky_quarantine_after",
            "testing.flaky_window_days",
            "testing.full_idle_secs",
            "testing.test_cache_days",
            "testing.test_slots",
        ]
    );

    // One bad key costs that key alone.
    let (config, problems) = parse("[testing]\ntest_slots = 0\nbisect_fix_max = 5\n");
    assert_eq!(keys(&problems), ["testing.test_slots"]);
    assert_eq!(config.testing.bisect_fix_max, 5);
    assert_eq!(config.testing.test_slots, None);

    // An unknown key is reported as the other tables report theirs.
    let (config, problems) = parse("[testing]\nfrobnicate = 1\ntest_slots = 8\n");
    let texts: Vec<String> = problems.iter().map(|p| p.to_string()).collect();
    assert_eq!(
        texts,
        ["testing.frobnicate: unknown key, ignored (using nothing)"]
    );
    assert_eq!(problems[0].message, "unknown key, ignored");
    assert_eq!(config.testing.test_slots, Some(8));

    // `testing` that is not a table is one problem, not an unknown key per entry.
    let (config, problems) = parse("testing = 5\n");
    assert_eq!(config.testing, Testing::default());
    let texts: Vec<String> = problems.iter().map(|p| p.to_string()).collect();
    assert_eq!(
        texts,
        ["testing: expected a table (using table of defaults)"]
    );
}
