//! Milestone 9.17 decision 26 (ruling C14): the stats page says how many task records
//! were judged by their replacements, after the deciders line, and nothing at zero.

use crate::app::stats::StatsState;

use super::tests::{app_with, history, rows};

const SIZES: [(u16, u16); 2] = [(80, 24), (120, 40)];

#[test]
fn stats_page_says_replaced_only_above_zero() {
    for ascii in [false, true] {
        let base = app_with(ascii, StatsState::Ready(Box::new(history())));
        let mut stats = history();
        stats.replaced = 1;
        let app = app_with(ascii, StatsState::Ready(Box::new(stats)));
        for (w, h) in SIZES {
            let (base, all) = (rows(&base, w, h), rows(&app, w, h));
            assert!(!base.iter().any(|l| l.contains("replaced")), "{w}x{h}");
            let deciders = all
                .iter()
                .position(|l| l.starts_with("deciders: "))
                .expect("the deciders line");
            assert_eq!(all[..=deciders], base[..=deciders], "{w}x{h}");
            assert_eq!(
                all[deciders + 1],
                "replaced 1 (judged by their replacements)",
                "{w}x{h}"
            );
            let rest = &base[deciders + 1..base.len() - 1];
            assert_eq!(&all[deciders + 2..], rest, "{w}x{h}");
        }
    }
}
