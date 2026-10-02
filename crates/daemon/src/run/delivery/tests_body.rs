//! Task M9.2.6: a stage PR's title and body (decisions 20 and 21, Interfaces "The PR
//! body"), against the shared three-stage fixture. Pure.

use proto::{PrState, TaskState};

use super::BODY_MAX_CHARS;
use super::body::{pr_body, pr_title};
use super::tests::{FOOTER, RUN, staged, task_mut};
use crate::host::HostError;
use crate::host::allow::{AllowCtx, check};
use crate::host::runner::Program;
use crate::run::tiers::Signal;

/// Decision 20: `[anthrex r<h4> <n>/<N>] <first task's title> (+<k> more)`, cut to
/// 256 characters, and (task 4's ruling) on one line with no control or bidi
/// character, so `allow::check`'s title rule passes it.
#[test]
fn pr_title_is_exact() {
    let mut run = staged();
    assert_eq!(pr_title(&run, 2), "[anthrex r1a2b 2/3] Title t2 (+1 more)");
    assert_eq!(pr_title(&run, 3), "[anthrex r1a2b 3/3] Title t4");
    let mut t5 = run.task("t3").unwrap().clone();
    t5.spec.id = "t5".into();
    run.tasks.push(t5);
    assert_eq!(pr_title(&run, 2), "[anthrex r1a2b 2/3] Title t2 (+2 more)");
    // A cancelled task is not in the stage's pull request.
    task_mut(&mut run, "t5").state = TaskState::Cancelled;
    assert_eq!(pr_title(&run, 2), "[anthrex r1a2b 2/3] Title t2 (+1 more)");

    task_mut(&mut run, "t2").spec.title = "x".repeat(300);
    let long = pr_title(&run, 2);
    assert_eq!(long.chars().count(), 256);
    assert!(long.starts_with("[anthrex r1a2b 2/3] xxx"), "{long}");

    task_mut(&mut run, "t2").spec.title = "Fix | this\r\nnow\u{202E}evil\u{7}".into();
    let title = pr_title(&run, 2);
    assert_eq!(title, "[anthrex r1a2b 2/3] Fix | this  nowevil  (+1 more)");
    let argv: Vec<String> = [
        "pr",
        "create",
        "--repo",
        "fake/app",
        "--base",
        "main",
        "--head",
        &format!("anthrex/{RUN}/stage-2"),
        "--title",
        &title,
        "--body-file",
        "/tmp/delivery/pr-2.md",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    let ctx = AllowCtx {
        run_id: Some(RUN),
        remote: "origin",
        base_branch: None,
        repo: Some("fake/app"),
    };
    assert_eq!(check(Program::Gh, &argv, &ctx), Ok(()));
    // The raw title would have been refused as Forbidden, which halts a run.
    let mut raw = argv.clone();
    raw[9] = "Fix\nthis".into();
    assert!(matches!(
        check(Program::Gh, &raw, &ctx),
        Err(HostError::Forbidden(_))
    ));
}

/// Interfaces "The PR body", exact, for decision 21's fixed run.
#[test]
fn pr_body_is_exact_for_a_fixed_run() {
    let run = staged();
    let expected = format!(
        "<!-- anthrex:pr pr-body-1a2b stage 2 -->
**Goal:** Test goal

**Stage 2 of 3:** Title t2 (+1 more)
**Stack:** based on stage 1, #141

### Look here first
- t2 (Title t2): hub task, runs alone
- t3 (Title t3): merged without approval: the reviewer timed out
- t2 (Title t2): escalated to rung 2
- t3 (Title t3): test changes to justify: 1 signal
- `.claude/settings.json`: a protected file

### Tasks
| Task | Title | Size | Test mode | Named test | Review | Rung |
|------|-------|------|-----------|------------|--------|------|
| t2 | Title t2 | M | tdd | `proto::round_trips` | approve, 2 rounds | 2 |
| t3 | Title t3 | S | check | – | override, 1 round | 0 |

### Test evidence
- Tiers run: tier 1 ×2, tier 2 ×1, tier 3 ×1
- Tier 3 on this head: green in 3 min 4 s, 2 shards
- Flaky tests seen: `a::flaky`

{FOOTER}"
    );
    assert_eq!(pr_body(&run, 2), expected);
}

/// Stage 1 is based on the base branch; a stage whose lower PRs are all merged is too.
#[test]
fn pr_body_names_the_base_branch_when_no_lower_pr_is_open() {
    let mut run = staged();
    let body = pr_body(&run, 1);
    assert!(
        body.contains("\n**Stack:** based on the base branch main\n"),
        "{body}"
    );
    run.delivery.stages[0].pr.as_mut().unwrap().state = PrState::Merged;
    assert!(pr_body(&run, 2).contains("\n**Stack:** based on the base branch main\n"));
    // Skipped stage 2: stage 3 is based on stage 1's PR.
    run.delivery.stages[0].pr.as_mut().unwrap().state = PrState::Open;
    run.delivery.stages[1].skipped = true;
    assert!(pr_body(&run, 3).contains("\n**Stack:** based on stage 1, #141\n"));
}

/// Decision 21's ranking: every hub line, then atomic, merged without approval, rung,
/// signals and protected files, each in plan order, whatever order the tasks are in.
#[test]
fn pr_body_ranks_look_here_first() {
    let mut run = staged();
    {
        let t2 = task_mut(&mut run, "t2");
        t2.hub = false;
        t2.max_rung = 3;
        t2.signals = vec![Signal::DiffTooLarge, Signal::DiffTooLarge];
        t2.signals_more = 1;
        t2.merged_without_approval = Some("override".into());
    }
    {
        let t3 = task_mut(&mut run, "t3");
        t3.hub = true;
        t3.spec.atomic = true;
        t3.spec.atomic_reason = Some("a schema migration".into());
        t3.merged_without_approval = None;
        t3.signals.clear();
        t3.max_rung = 2;
    }
    let body = pr_body(&run, 2);
    let section = body
        .split("### Look here first\n")
        .nth(1)
        .and_then(|rest| rest.split("\n\n").next())
        .unwrap();
    assert_eq!(
        section,
        "- t3 (Title t3): hub task, runs alone
- t3 (Title t3): atomic task (a schema migration)
- t2 (Title t2): merged without approval: override
- t2 (Title t2): escalated to rung 3
- t3 (Title t3): escalated to rung 2
- t2 (Title t2): test changes to justify: 3 signals
- `.claude/settings.json`: a protected file"
    );
}

#[test]
fn pr_body_says_none_when_nothing_needs_a_look() {
    let mut run = staged();
    for id in ["t2", "t3"] {
        let t = task_mut(&mut run, id);
        t.hub = false;
        t.max_rung = 1;
        t.merged_without_approval = None;
        t.signals.clear();
    }
    task_mut(&mut run, "t3").spec.owns = vec!["crates/b/**".into()];
    let body = pr_body(&run, 2);
    assert!(
        body.contains(
            "### Look here first\n(none: nothing here needs a closer look than the rest)\n\n### Tasks\n"
        ),
        "{body}"
    );
    // The review column of a task merged with approval, and of one never reviewed.
    task_mut(&mut run, "t3").reviews.clear();
    let body = pr_body(&run, 2);
    assert!(body.contains("| t3 | Title t3 | S | check | – | none, 0 rounds | 1 |"));
}

/// Decision 21: at most 60 000 characters, by dropping table rows from the end with a
/// `… and <k> more tasks` row; the marker, the evidence and the footer stay.
#[test]
fn pr_body_is_cut_at_60000_characters_with_a_more_row() {
    let mut run = staged();
    let template = run.task("t3").unwrap().clone();
    for k in 0..40 {
        let mut t = template.clone();
        t.spec.id = format!("x{k}");
        t.spec.title = format!("{k:02} {}", "y".repeat(2_000));
        t.merged_without_approval = None;
        t.signals.clear();
        run.tasks.push(t);
    }
    // Look-here lines name the titles too; keep only the table long.
    let full: usize = run
        .tasks
        .iter()
        .filter(|t| t.stage() == 2)
        .map(|t| t.spec.title.chars().count())
        .sum();
    assert!(full > BODY_MAX_CHARS);
    let body = pr_body(&run, 2);
    assert!(body.chars().count() <= BODY_MAX_CHARS, "{}", body.len());
    assert!(body.starts_with("<!-- anthrex:pr pr-body-1a2b stage 2 -->\n"));
    assert!(body.ends_with(FOOTER));
    assert!(body.contains("\n### Test evidence\n"));
    let rows: Vec<&str> = body
        .lines()
        .filter(|l| l.starts_with("| t") || l.starts_with("| x"))
        .collect();
    let more = body
        .lines()
        .find(|l| l.starts_with("| … and "))
        .expect("a more row");
    let kept = rows.len();
    let dropped = 42 - kept;
    assert_eq!(more, format!("| … and {dropped} more tasks | | | | | | |"));
    assert!(dropped > 0 && kept > 2);
    // Rows are dropped from the end: the first rows are the plan's first tasks.
    assert!(rows[0].starts_with("| t2 |") && rows[1].starts_with("| t3 |"));
    assert!(rows[2].starts_with("| x0 | 00 "));
    // Rows kept are whole: nothing else was cut.
    assert!(rows.iter().all(|r| r.ends_with(" |")));
    // And no more was dropped than needed: one more row would not fit.
    let one_more = 2_000 + 60;
    assert!(body.chars().count() + one_more > BODY_MAX_CHARS);

    // A title too long for any row to help: the text before the footer is cut, the
    // marker and the footer stay.
    let mut run = staged();
    task_mut(&mut run, "t2").spec.title = "z".repeat(70_000);
    let body = pr_body(&run, 2);
    assert_eq!(body.chars().count(), BODY_MAX_CHARS);
    assert!(body.starts_with("<!-- anthrex:pr pr-body-1a2b stage 2 -->\n"));
    assert!(body.ends_with(&format!("\n\n{FOOTER}")));
}

/// Decision 21: agent text cannot add a table column, a line, an HTML comment, a
/// mention or markup to the body.
#[test]
fn pr_body_escapes_agent_text() {
    let mut run = staged();
    let plain = pr_body(&run, 2);
    task_mut(&mut run, "t3").spec.title =
        "a | b\n<!-- anthrex:reply x -->\r### heading @alice *bold* [l](u) `tick` <b>".into();
    task_mut(&mut run, "t3").merged_without_approval = Some("why |\n- not".into());
    task_mut(&mut run, "t2").spec.test_to_write = Some("a`b|c".into());
    run.goal = "goal\n# not a heading <!--".into();
    let body = pr_body(&run, 2);
    assert_eq!(body.lines().count(), plain.lines().count(), "{body}");
    // The marker is the only HTML comment.
    for (at, _) in body.match_indices("<!--").skip(1) {
        assert_eq!(&body[at - 1..at], "\\", "{body}");
    }
    assert!(body.starts_with("<!-- anthrex:pr pr-body-1a2b stage 2 -->\n"));
    // No mention, no raw markup.
    assert!(!body.contains("@alice"), "{body}");
    assert!(body.contains("＠alice"));
    for raw in ["<b>", " *bold*", "[l](u)", " `tick`"] {
        assert!(!body.contains(raw), "{raw} in {body}");
    }
    // Every table line has the table's eight unescaped pipes.
    let table: Vec<&str> = body.lines().filter(|l| l.starts_with("| ")).collect();
    assert_eq!(table.len(), 3, "{body}");
    for line in table {
        let pipes = line
            .match_indices('|')
            .filter(|(i, _)| *i == 0 || line.as_bytes()[i - 1] != b'\\');
        assert_eq!(pipes.count(), 8, "{line}");
    }
    assert!(body.contains(
        "| t3 | a \\| b \\<!-- anthrex:reply x --\\> \\#\\#\\# heading ＠alice \\*bold\\* \\[l\\](u) \\`tick\\` \\<b\\> |"
    ), "{body}");
    assert!(body.contains("| ``a`b\\|c`` |"), "{body}");
    assert!(
        body.contains("**Goal:** goal \\# not a heading \\<!--\n"),
        "{body}"
    );
    assert!(
        body.contains("merged without approval: why \\| - not\n"),
        "{body}"
    );
}
