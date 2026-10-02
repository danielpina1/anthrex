//! Milestone 9.2 task M9.2.15: the stage row's pull-request suffix, exact for each
//! state, in Unicode and ASCII (decision 42).

use super::row_suffix;
use crate::graph::run_text::stage_text_in;
use crate::tree::pr_fixtures::{delivery, pr, pr_fixture};
use crate::tree::run_fixtures::{PROJECT, RUN_ID, run};
use crate::tree::stage_fixtures::stage;
use proto::{CiState, PrState, RunInfo, RunState, StageInfo, StagePrInfo, ThreadCounts};

fn pr_run() -> RunInfo {
    let mut info = run(RUN_ID, PROJECT, RunState::Running);
    info.delivery = Some(delivery(false));
    info
}

fn with_pr(pr: Option<StagePrInfo>) -> StageInfo {
    let mut s = stage(2, Some(&"2".repeat(40)), 2, 1);
    s.pr = pr;
    s
}

fn threads(new: u32, tasked: u32) -> ThreadCounts {
    ThreadCounts {
        new,
        tasked,
        replied: 5,
        ignored: 7,
    }
}

#[test]
fn stage_row_suffix_is_exact_for_each_state() {
    let run = pr_run();
    let open = |ci, t: ThreadCounts| {
        let mut p = pr(142, PrState::Open, ci);
        p.threads = t;
        Some(p)
    };
    let mut paused = pr(142, PrState::Open, CiState::Green);
    paused.paused = true;
    let cases: Vec<(&str, Option<StagePrInfo>, &str, &str)> = vec![
        (
            "open green, two threads",
            open(CiState::Green, threads(1, 1)),
            "  #142  ci ✓  2 threads",
            "  #142  ci +  2 threads",
        ),
        (
            "red, one thread",
            open(CiState::Red, threads(0, 1)),
            "  #142  ci ✗  1 thread",
            "  #142  ci x  1 thread",
        ),
        (
            "pending, replied and ignored threads not counted",
            open(CiState::Pending, threads(0, 0)),
            "  #142  ci …",
            "  #142  ci ...",
        ),
        (
            "no checks",
            open(CiState::None, ThreadCounts::default()),
            "  #142  ci –",
            "  #142  ci _",
        ),
        (
            "paused",
            Some(paused),
            "  #142  ci ✓  paused",
            "  #142  ci +  paused",
        ),
        (
            "merged",
            Some(pr(142, PrState::Merged, CiState::Red)),
            "  #142  merged",
            "  #142  merged",
        ),
        (
            "closed",
            Some(pr(142, PrState::Closed, CiState::Green)),
            "  #142  closed",
            "  #142  closed",
        ),
        ("no PR yet", None, "  no PR yet", "  no PR yet"),
    ];
    for (name, pr, unicode, ascii) in cases {
        let s = with_pr(pr);
        assert_eq!(row_suffix(&run, &s, false), unicode, "{name}");
        assert_eq!(row_suffix(&run, &s, true), ascii, "{name} (ASCII)");
    }
}

/// A stage the engine skipped (decision 19: no changes) never gets a PR.
#[test]
fn a_skipped_stage_says_so() {
    let mut run = pr_run();
    if let Some(d) = run.delivery.as_mut() {
        d.skipped_stages = vec![2];
    }
    assert_eq!(row_suffix(&run, &with_pr(None), false), "  skipped");
}

/// A local run (no `delivery`) has no suffix, whatever its stages say.
#[test]
fn a_local_run_has_no_suffix() {
    let run = run(RUN_ID, PROJECT, RunState::Running);
    let s = with_pr(Some(pr(142, PrState::Open, CiState::Red)));
    assert_eq!(row_suffix(&run, &s, false), "");
    assert_eq!(row_suffix(&run, &with_pr(None), true), "");
}

/// The whole content row, as decision 42 writes it.
#[test]
fn the_stage_row_reads_tier_3_then_its_pr() {
    let (snap, _) = pr_fixture();
    let run = &snap.runs[0];
    let rows: Vec<String> = (run.stages.iter())
        .map(|s| stage_text_in(run, s, false))
        .collect();
    assert_eq!(
        rows,
        [
            "stage 1/3  tier 3 ✓ 38s  #141  merged",
            "stage 2/3  tier 3 ✓ 38s  #142  ci ✗  3 threads",
            "stage 3/3  tier 3 ✓ 38s  #143  ci …",
        ]
    );
    assert_eq!(
        stage_text_in(run, &run.stages[1], true),
        "stage 2/3  tier 3 + 38s  #142  ci x  3 threads"
    );
}

/// A `pr`-mode stage whose PR is `number`, open, red, `threads` to address, and paused,
/// in a run of `stages` stages, its tier 3 bisecting: the widest stage row there is.
fn widest(number: u64, stages: u16, threads_open: u32) -> (RunInfo, StageInfo) {
    let mut run = pr_run();
    run.stages = (1..=stages).map(|n| stage(n, None, 0, 0)).collect();
    let mut s = with_pr({
        let mut p = pr(number, PrState::Open, CiState::Red);
        p.threads = threads(threads_open, 0);
        p.paused = true;
        Some(p)
    });
    s.n = stages;
    s.full.state = proto::FullState::Bisecting;
    (run, s)
}

/// Review C, M4: the stage box's cap fits the longest suffix, `paused` with a five-digit
/// PR number included, after the longest tier-3 text, uncut in either form.
#[test]
fn the_longest_stage_row_fits_its_box() {
    let (run, s) = widest(99_999, 99, 99);
    let unicode = "stage 99/99  tier 3 ✗ bisecting  #99999  ci ✗  99 threads  paused";
    assert_eq!(stage_text_in(&run, &s, false), unicode);
    assert_eq!(
        stage_text_in(&run, &s, true),
        "stage 99/99  tier 3 x bisecting  #99999  ci x  99 threads  paused"
    );
    let room = usize::from(crate::graph::MAX_STAGE_NODE_WIDTH) - 4 - 2;
    assert_eq!(unicode::width(unicode), room, "the cap is exactly this row");
}

/// Review C, M4: a row too wide for the box loses its PR number's digits first, never
/// the state word at its end.
#[test]
fn a_longer_pr_number_is_cut_before_the_state_word() {
    let (run, s) = widest(1_234_567, 99, 99);
    assert_eq!(
        stage_text_in(&run, &s, false),
        "stage 99/99  tier 3 ✗ bisecting  #1234…  ci ✗  99 threads  paused"
    );
    assert_eq!(
        stage_text_in(&run, &s, true),
        "stage 99/99  tier 3 x bisecting  #12...  ci x  99 threads  paused"
    );
    // A narrower row keeps its whole number.
    let (run, s) = widest(1_234_567, 9, 9);
    assert_eq!(
        stage_text_in(&run, &s, false),
        "stage 9/9  tier 3 ✗ bisecting  #1234567  ci ✗  9 threads  paused"
    );
}

mod unicode {
    pub fn width(text: &str) -> usize {
        unicode_width::UnicodeWidthStr::width(text)
    }
}
