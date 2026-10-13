//! Task M9.15.5: `anthrex lessons`' arguments and decision 20's `list` layout.

use clap::Parser;
use proto::{LessonInfo, LessonLayer, LessonOrigin, LessonStatus};

use super::{LessonsCommand, list_text, prompt};

#[derive(Parser, Debug)]
struct Cli {
    #[command(subcommand)]
    command: LessonsCommand,
}

fn parse(args: &[&str]) -> Result<LessonsCommand, clap::Error> {
    Cli::try_parse_from(std::iter::once("lessons").chain(args.iter().copied())).map(|c| c.command)
}

fn ids(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

#[test]
fn the_four_verbs_parse_with_their_flags() {
    assert_eq!(
        parse(&["list"]).unwrap(),
        LessonsCommand::List {
            all: false,
            json: false
        }
    );
    assert_eq!(
        parse(&["list", "--all", "--json"]).unwrap(),
        LessonsCommand::List {
            all: true,
            json: true
        }
    );
    assert_eq!(
        parse(&["approve", "l-0123ab", "kf-abcdef", "--yes"]).unwrap(),
        LessonsCommand::Approve {
            ids: ids(&["l-0123ab", "kf-abcdef"]),
            yes: true
        }
    );
    assert_eq!(
        parse(&["reject", "l-0123ab"]).unwrap(),
        LessonsCommand::Reject {
            ids: ids(&["l-0123ab"]),
            yes: false
        }
    );
    assert_eq!(
        parse(&["forget", "--yes", "kf-abcdef"]).unwrap(),
        LessonsCommand::Forget {
            ids: ids(&["kf-abcdef"]),
            yes: true
        }
    );
    for bad in [
        &["approve"][..],
        &["reject", "--yes"],
        &["forget"],
        &["add", "a lesson"],
        &["list", "--yes"],
    ] {
        assert!(parse(bad).is_err(), "{bad:?}");
    }
}

fn lesson(id: &str, status: LessonStatus, layer: LessonLayer, title: &str) -> LessonInfo {
    LessonInfo {
        id: id.into(),
        layer,
        status,
        origin: if id.starts_with("kf-") {
            LessonOrigin::Signature
        } else {
            LessonOrigin::Distilled
        },
        title: title.into(),
        body: String::new(),
        files: Vec::new(),
        created_at: 1,
        last_seen: 2,
        runs: Vec::new(),
        green: 0,
        outcomes: 0,
        reconfirmed: 0,
        headlines: Vec::new(),
        retired_reason: None,
    }
}

fn fixed() -> Vec<LessonInfo> {
    let mut known = lesson(
        "kf-0123456789abcdef",
        LessonStatus::Known,
        LessonLayer::Episodic,
        "Known failure: error: boom",
    );
    known.body = "Seen 3 times in 2 runs (last 2027-01-15, gate check): error: boom".into();
    known.files = vec!["crates/a/**".into(), "crates/b/src/lib.rs".into()];
    let mut approved = lesson(
        "l-00aa11bb22cc33dd",
        LessonStatus::Approved,
        LessonLayer::Procedural,
        "Run cargo fmt before claiming done",
    );
    approved.body = "CI rejects unformatted code, and the reviewer cannot see why the gate \
                     failed until the check summary arrives, which costs a whole review round."
        .into();
    approved.green = 3;
    approved.outcomes = 4;
    let mut hostile = lesson(
        "l-ffee",
        LessonStatus::Proposed,
        LessonLayer::Semantic,
        "evil\u{1b}[31m title\u{202e}txt.exe\nsecond line",
    );
    hostile.body = "line one\nline two\u{1b}]8;;x\u{7}".into();
    hostile.files = vec!["a\nb".into()];
    vec![known, approved, hostile]
}

#[test]
fn list_matches_decision_20s_layout() {
    let want = "\
lessons for /work/repo: 2 active, 1 to review
kf-0123456789abcdef  known  episodic  –  Known failure: error: boom
    Seen 3 times in 2 runs (last 2027-01-15, gate check): error: boom
    files: crates/a/**, crates/b/src/lib.rs

l-00aa11bb22cc33dd  approved  procedural  3/4  Run cargo fmt before claiming done
    CI rejects unformatted code, and the reviewer cannot see why the gate failed until the check
    summary arrives, which costs a whole review round.

l-ffee  proposed  semantic  –  evil [31m titletxt.exe second line
    line one line two ]8;;x
    files: a b
";
    assert_eq!(list_text("/work/repo", &fixed()), want);
    for line in list_text("/work/repo", &fixed()).lines() {
        assert!(line.chars().count() <= super::WRAP, "{line:?}");
        assert!(!line.chars().any(|c| c.is_control()), "{line:?}");
    }
}

#[test]
fn an_empty_list_says_so() {
    assert_eq!(
        list_text("/work/repo", &[]),
        "no lessons for /work/repo yet\n"
    );
}

#[test]
fn a_question_names_the_lesson_and_its_title() {
    let known = fixed();
    assert_eq!(
        prompt("approve", "l-00aa11", &known),
        "approve l-00aa11bb22cc33dd \"Run cargo fmt before claiming done\"? [y/N] "
    );
    assert_eq!(
        prompt("forget", "l-ffee", &known),
        "forget l-ffee \"evil [31m titletxt.exe second line\"? [y/N] "
    );
    // Several or none: the id as typed (the daemon refuses it).
    assert_eq!(prompt("reject", "l-", &known), "reject l-? [y/N] ");
    assert_eq!(prompt("reject", "l-zz", &known), "reject l-zz? [y/N] ");
}
