//! Task M9.2.14: `run prs`'s table, `run status`'s delivery lines and `--delivery`'s
//! parse, on constructed snapshots (pure).

use proto::{
    CiState, DeliveryInfo, DeliveryMode, FullInfo, PrState, RunInfo, RunState, StageInfo,
    StagePrInfo, ThreadCounts,
};

use super::super::status::{run_block, tests::example};
use super::{mode, prs_table, status_lines};

fn stage(n: u16, pr: Option<StagePrInfo>) -> StageInfo {
    StageInfo {
        actions: Vec::new(),
        n,
        branch: format!("anthrex/add-reset-3f9a/stage-{n}"),
        head: Some(format!("{n}{n}{n}{n}")),
        tasks: 2,
        merged: 1,
        full: FullInfo::default(),
        fix_tasks: Vec::new(),
        propagate_red: None,
        pr,
        round: 1,
    }
}

fn pr(number: u64, state: PrState, ci: CiState) -> StagePrInfo {
    StagePrInfo {
        number,
        url: format!("https://github.com/fake/app/pull/{number}"),
        state,
        base: "main".into(),
        opened_at: 1_700_000_000,
        head: "1a2b3c4".into(),
        ci,
        checks: Vec::new(),
        threads: ThreadCounts::default(),
        fix_tasks: Vec::new(),
        paused: false,
        human_review_secs: 0,
        merged_at: None,
        merge_commit: None,
    }
}

fn delivered(stages: Vec<StageInfo>) -> RunInfo {
    let mut run = example();
    run.stages = stages;
    run.delivery = Some(DeliveryInfo {
        mode: DeliveryMode::Pr,
        remote: "origin".into(),
        repo: "fake/app".into(),
        watching: true,
        delivering: false,
        poll_secs: 60,
        skipped_stages: Vec::new(),
        alerts: Vec::new(),
    });
    run
}

/// Interfaces "CLI", character for character.
#[test]
fn prs_table_is_the_interfaces_example() {
    let mut p = pr(142, PrState::Open, CiState::Red);
    p.threads = ThreadCounts {
        new: 2,
        tasked: 1,
        replied: 0,
        ignored: 4,
    };
    p.fix_tasks = vec!["fix3 ci working".into()];
    let run = delivered(vec![stage(1, Some(p)), stage(2, None)]);
    assert_eq!(
        prs_table(&run),
        "STAGE  PR     STATE    CI       THREADS                      FIX TASKS\n\
         1/2    #142   open     red      2 new, 1 tasked, 0 replied   fix3 working\n\
         2/2    –      waiting  –        –                            –\n"
    );
}

#[test]
fn prs_table_shows_every_state() {
    let mut paused = pr(143, PrState::Open, CiState::Pending);
    paused.paused = true;
    paused.fix_tasks = vec!["fix4 review merged".into(), "fix5 sync queued".into()];
    let mut run = delivered(vec![
        stage(1, Some(pr(141, PrState::Merged, CiState::Green))),
        stage(2, Some(pr(1_234_567, PrState::Closed, CiState::None))),
        stage(3, Some(paused)),
        stage(4, None),
        stage(5, None),
    ]);
    run.delivery.as_mut().unwrap().skipped_stages = vec![4];
    let table = prs_table(&run);
    let lines: Vec<&str> = table.lines().skip(1).collect();
    assert_eq!(
        lines,
        [
            "1/5    #141   merged   green    0 new, 0 tasked, 0 replied   –",
            // A cell as wide as its column or wider ends in one space.
            "2/5    #1234567 closed   none     0 new, 0 tasked, 0 replied   –",
            "3/5    #143   paused   pending  0 new, 0 tasked, 0 replied   fix4 merged, fix5 queued",
            "4/5    –      skipped  –        –                            –",
            "5/5    –      waiting  –        –                            –",
        ]
    );
}

#[test]
fn status_shows_the_delivery_and_prs_lines() {
    let mut red = pr(142, PrState::Open, CiState::Red);
    red.threads = ThreadCounts {
        new: 1,
        tasked: 1,
        replied: 3,
        ignored: 2,
    };
    let mut run = delivered(vec![
        stage(1, Some(pr(141, PrState::Merged, CiState::Green))),
        stage(2, Some(red)),
        stage(3, Some(pr(143, PrState::Open, CiState::Pending))),
    ]);
    assert_eq!(
        status_lines(&run),
        "  delivery: pr to origin (fake/app), watching every 60 s\n  \
         prs: #141 merged, #142 open (ci red, 2 threads), #143 open (ci pending)\n"
    );
    run.delivery.as_mut().unwrap().watching = false;
    run.stages.clear();
    assert_eq!(
        status_lines(&run),
        "  delivery: pr to origin (fake/app), not watching\n"
    );
    // A local run has neither line, and `run status` prints milestone 9.1's text.
    assert_eq!(status_lines(&example()), "");
}

#[test]
fn status_header_says_delivering_when_decision_36_holds() {
    let mut run = delivered(vec![stage(1, Some(pr(7, PrState::Open, CiState::Green)))]);
    run.state = RunState::Running;
    let header = |run: &RunInfo| run_block(run, 0).lines().next().unwrap().to_string();
    assert!(!header(&run).contains("delivering"), "{}", header(&run));
    run.delivery.as_mut().unwrap().delivering = true;
    assert!(
        header(&run).starts_with(&format!("{}  running (delivering)  ", run.run_id)),
        "{}",
        header(&run)
    );
    let block = run_block(&run, 0);
    assert!(
        block.contains("\n  delivery: pr to origin (fake/app), watching every 60 s\n  prs: #7 open (ci green)\n"),
        "{block}"
    );
    run.state = RunState::Paused;
    assert!(!header(&run).contains("delivering"), "{}", header(&run));
}

#[test]
fn delivery_flag_parses_pr_and_local_only() {
    assert_eq!(mode(None).unwrap(), None);
    assert_eq!(mode(Some("pr")).unwrap(), Some(DeliveryMode::Pr));
    assert_eq!(mode(Some("local")).unwrap(), Some(DeliveryMode::Local));
    let empty = mode(Some("")).unwrap_err().to_string();
    assert!(
        empty.contains("a value is required for '--delivery <pr|local>'"),
        "{empty}"
    );
    for wrong in ["PR", "github", "pr,local", "-x"] {
        let error = mode(Some(wrong)).unwrap_err().to_string();
        assert!(
            error.contains(&format!(
                "invalid value '{wrong}' for '--delivery <pr|local>'"
            )) && error.contains("[possible values: pr, local]"),
            "{wrong}: {error}"
        );
    }
}
