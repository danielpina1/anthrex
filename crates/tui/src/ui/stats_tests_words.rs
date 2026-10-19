//! Milestone 9.19 decision 14: the stats page labels its aggregates in the CLI's words
//! (`anthrex run stats`), in lower case: each median column says it is one, the footer
//! says over what, and the rounds line names both of its numbers.

use common::labels::{STATS_FOOTER, STATS_HEADER};

use super::tests::{app_with, history, rows};
use crate::app::stats::StatsState;

/// The column words of a header drawn over two rows: each column's top word, then the
/// word under it (if any), joined by a space.
fn column_words(top: &str, bottom: &str) -> Vec<String> {
    let starts: Vec<usize> = (0..top.len())
        .filter(|&i| top.as_bytes()[i] != b' ' && (i == 0 || top.as_bytes()[i - 1] == b' '))
        .collect();
    starts
        .iter()
        .map(|&s| {
            let first = top[s..].split(' ').next().unwrap_or_default();
            let under = bottom.get(s..).unwrap_or_default();
            match under.split("  ").next().filter(|w| !w.starts_with(' ')) {
                Some(w) if !w.is_empty() => format!("{first} {w}"),
                _ => first.to_owned(),
            }
        })
        .collect()
}

#[test]
fn tui_stats_matches_the_cli_words() {
    let mut stats = history();
    stats.rounds = 3;
    stats.iterated_runs = 1;
    let app = app_with(false, StatsState::Ready(Box::new(stats)));
    for (w, h) in [(80, 24), (120, 40)] {
        let all = rows(&app, w, h);
        let want: Vec<String> = STATS_HEADER.iter().map(|h| h.to_lowercase()).collect();
        assert_eq!(column_words(&all[2], &all[3]), want, "{w}x{h}: {all:#?}");
        // Every row of the table fits 80 columns: the header is stacked.
        assert!(
            all[2..6].iter().all(|r| r.chars().count() <= 78),
            "{all:#?}"
        );
        // The footer, under the table, wrapped whole, never cut.
        let start = 1 + all.iter().position(|r| r.starts_with("hub ")).unwrap();
        let end = start + all[start..].iter().position(String::is_empty).unwrap();
        assert_eq!(
            all[start..end].join(" "),
            STATS_FOOTER.to_lowercase(),
            "{w}x{h}: {all:#?}"
        );
        assert!(
            all.iter().any(|r| r == "rounds 3 ended · 1 run iterated"),
            "{w}x{h}: {all:#?}"
        );
        // 9.19 follow-up 2: the deciders line is the CLI's, word for word
        // (`daemon/src/run/stats.rs` prints the same `common::labels::deciders_text`).
        let deciders = "deciders: 6 calls, 1 fallback · size cross-check: 4 checked, 1 raised";
        assert_eq!(common::labels::deciders_text(&history()), deciders);
        assert!(all.iter().any(|r| r == deciders), "{w}x{h}: {all:#?}");
        assert!(!all.iter().any(|r| r.contains("size raised")), "{all:#?}");
    }
    // Without a round line, no rounds row.
    let app = app_with(false, StatsState::Ready(Box::new(history())));
    assert!(!rows(&app, 80, 24).iter().any(|r| r.starts_with("rounds")));
}

/// The CLI's token unit (`daemon/src/run/stats.rs`): `1.2k` under 10k.
#[test]
fn stats_cells_use_the_clis_units() {
    let cases = [
        (999, "999"),
        (1000, "1.0k"),
        (1250, "1.2k"),
        (9999, "9.9k"),
        (10_000, "10k"),
        (2_345_678, "2.3M"),
    ];
    for (n, want) in cases {
        assert_eq!(common::numbers::tokens(n), want, "{n}");
    }
}

/// Milestone 9.19 decision 4: the median work time is a `duration`, floored, under
/// `med work`; 89 s is `1m 29s`, never a rounded-up `1m`.
#[test]
fn stats_work_floors() {
    let mut stats = history();
    stats.rows[0].median_work_secs = Some(89);
    let app = app_with(false, StatsState::Ready(Box::new(stats)));
    let all = rows(&app, 80, 24);
    assert!(all[2].contains("  med  "), "{:?}", all[2]);
    assert!(all[3].contains("  work  "), "{:?}", all[3]);
    assert!(all[4].contains("  1m 29s  "), "{:?}", all[4]);
    assert!(!all[4].contains("2m"), "{:?}", all[4]);
}
