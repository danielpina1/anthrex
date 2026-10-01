//! Milestone 9.2 task M9.2.3: the `[delivery]` table.

use super::super::*;

fn problem(text: &str) -> Vec<Problem> {
    parse(text).1
}

/// The one problem `text` gives, as `(key, message, default)`.
fn only(text: &str) -> (String, String, String) {
    let problems = problem(text);
    assert_eq!(problems.len(), 1, "{text}: {problems:?}");
    let p = &problems[0];
    (p.key.clone(), p.message.clone(), p.default.clone())
}

#[test]
fn delivery_defaults_when_absent() {
    let (config, problems) = parse("");
    assert!(problems.is_empty());
    let d = &config.delivery;
    assert_eq!(d.poll_secs, 60);
    assert_eq!(d.poll_max_secs, 300);
    assert_eq!(d.ci_log_max_bytes, 2_097_152);
    assert_eq!(d.ci_fix_max, 2);
    assert_eq!(d.review_fix_max, 3);
    assert_eq!(d.review_batch_secs, 120);
    assert!(d.reviewers.is_empty());
    assert!(d.reply_to_comments);
    assert_eq!(d.sync, SyncPolicy::OnConflict);
    assert!(!d.delete_merged_branches);
    assert_eq!(d.stage_target_lines, (300, 800));
    assert_eq!(*d, Delivery::default());

    // A 9.1 config, which has no `[delivery]` table, loads with the defaults and no
    // problem; an empty table too.
    let m9_1 = "[testing]\nfull_idle_secs = 60\n[orchestrator]\nmax_writers = 4\n";
    let (config, problems) = parse(m9_1);
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(config.delivery, Delivery::default());
    let (config, problems) = parse("[delivery]\n");
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(config.delivery, Delivery::default());
}

#[test]
fn every_delivery_key_is_read() {
    let text = "[delivery]\npoll_secs = 5\npoll_max_secs = 9\nci_log_max_bytes = 4096\n\
                ci_fix_max = 0\nreview_fix_max = 10\nreview_batch_secs = 0\n\
                reviewers = [\"alice\", \"bob\", \"renovate[bot]\"]\nreply_to_comments = false\n\
                sync = \"always\"\ndelete_merged_branches = true\nstage_target_lines = [50, 5000]\n";
    let (config, problems) = parse(text);
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(
        config.delivery,
        Delivery {
            poll_secs: 5,
            poll_max_secs: 9,
            ci_log_max_bytes: 4096,
            ci_fix_max: 0,
            review_fix_max: 10,
            review_batch_secs: 0,
            reviewers: vec!["alice".into(), "bob".into(), "renovate[bot]".into()],
            reply_to_comments: false,
            sync: SyncPolicy::Always,
            delete_merged_branches: true,
            stage_target_lines: (50, 5000),
        }
    );
}

#[test]
fn each_delivery_range_is_enforced_with_the_exact_message() {
    // (key, low, high, default) of Interfaces "config".
    let ranged: [(&str, i64, i64, &str); 6] = [
        ("poll_secs", 1, 3600, "60"),
        ("poll_max_secs", 1, 3600, "300"),
        ("ci_log_max_bytes", 4096, 16_777_216, "2097152"),
        ("ci_fix_max", 0, 10, "2"),
        ("review_fix_max", 0, 10, "3"),
        ("review_batch_secs", 0, 3600, "120"),
    ];
    for (key, low, high, default) in ranged {
        for ok in [low, high] {
            let text = format!("[delivery]\n{key} = {ok}\n");
            assert!(problem(&text).is_empty(), "{text}");
        }
        for bad in [low - 1, high + 1] {
            let text = format!("[delivery]\n{key} = {bad}\n");
            let (k, message, using) = only(&text);
            assert_eq!(k, format!("delivery.{key}"));
            assert_eq!(message, format!("must be between {low} and {high}"));
            assert_eq!(using, default);
            assert_eq!(parse(&text).0.delivery, Delivery::default());
        }
        let text = format!("[delivery]\n{key} = \"5\"\n");
        assert_eq!(
            only(&text).1,
            format!("must be between {low} and {high}"),
            "a wrong type"
        );
    }
    // `sync` takes only its two values.
    for ok in ["on_conflict", "always"] {
        assert!(problem(&format!("[delivery]\nsync = \"{ok}\"\n")).is_empty());
    }
    for bad in ["\"rebase\"", "\"\"", "1"] {
        let text = format!("[delivery]\nsync = {bad}\n");
        assert_eq!(
            only(&text),
            (
                "delivery.sync".to_string(),
                "must be on_conflict or always".to_string(),
                "on_conflict".to_string()
            ),
            "{text}"
        );
    }
    // `reviewers` is an array of GitHub logins.
    for bad in [
        "[1]",
        "\"alice\"",
        "[\"alice\", 2]",
        "[\"has space\"]",
        "[\"\"]",
        "[\"a234567890123456789012345678901234567890\"]",
    ] {
        let text = format!("[delivery]\nreviewers = {bad}\n");
        assert_eq!(
            only(&text),
            (
                "delivery.reviewers".to_string(),
                "must be an array of GitHub logins".to_string(),
                "[]".to_string()
            ),
            "{text}"
        );
        assert!(parse(&text).0.delivery.reviewers.is_empty());
    }
    // The booleans.
    for key in ["reply_to_comments", "delete_merged_branches"] {
        let text = format!("[delivery]\n{key} = \"yes\"\n");
        let (k, message, _) = only(&text);
        assert_eq!(
            (k.as_str(), message.as_str()),
            (format!("delivery.{key}").as_str(), "expected a boolean")
        );
    }
    // A `[delivery]` that is not a table.
    assert_eq!(
        only("delivery = 3\n"),
        (
            "delivery".to_string(),
            "expected a table".to_string(),
            "table of defaults".to_string()
        )
    );
    // Any other key is M8a's `unknown key, ignored`.
    assert_eq!(
        only("[delivery]\npoll = 3\n"),
        (
            "delivery.poll".to_string(),
            "unknown key, ignored".to_string(),
            "nothing".to_string()
        )
    );
}

#[test]
fn mode_and_remote_in_config_are_refused_with_the_profile_hint() {
    for (key, value) in [("mode", "\"pr\""), ("remote", "\"origin\"")] {
        let text = format!("[delivery]\n{key} = {value}\npoll_secs = 7\n");
        let (config, problems) = parse(&text);
        assert_eq!(problems.len(), 1, "{problems:?}");
        let p = &problems[0];
        // Decision 3's exact text: `delivery.<key> is set per repository in its profile
        // (anthrex profile edit), ignored`.
        assert_eq!(
            format!("{} {}", p.key, p.message),
            format!(
                "delivery.{key} is set per repository in its profile (anthrex profile edit), ignored"
            )
        );
        assert_eq!(p.default, "nothing");
        // The rest of the table is still read.
        assert_eq!(config.delivery.poll_secs, 7);
    }
}

#[test]
fn stage_target_lines_must_be_ascending() {
    let refused = (
        "delivery.stage_target_lines".to_string(),
        "must be two integers between 50 and 5000, the first below the second".to_string(),
        "[300, 800]".to_string(),
    );
    for bad in [
        "[800, 300]",
        "[400, 400]",
        "[49, 800]",
        "[300, 5001]",
        "[300]",
        "[300, 800, 900]",
        "[\"300\", 800]",
        "300",
    ] {
        let text = format!("[delivery]\nstage_target_lines = {bad}\n");
        assert_eq!(only(&text), refused, "{text}");
        assert_eq!(parse(&text).0.delivery.stage_target_lines, (300, 800));
    }
    let (config, problems) = parse("[delivery]\nstage_target_lines = [200, 201]\n");
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(config.delivery.stage_target_lines, (200, 201));
}
