//! Task M9.6.9: the merged report's appendix (decision 13) and what the gate reads from
//! the report (DF §6.1).

use proto::{ApproachTag, DocKind, ReportSummary};

use super::{APPENDIX, attach, body, split, summary};
use crate::run::design::template::tests::{DRAFT, REPORT};
use crate::run::design::template::{heading, lines};

fn labels() -> Vec<String> {
    vec!["claude".into(), "codex".into()]
}

/// Decision 13: the drafts follow the report under `## Appendix: the drafts`, each under
/// `### <label>`, its own headings demoted below the label's; a brainstormer without a
/// draft is named with why.
#[test]
fn the_appendix_follows_the_report_with_each_draft_under_its_label() {
    let drafts = vec![
        ("claude".to_string(), Ok(DRAFT.to_string())),
        ("codex".to_string(), Err("it crashed".to_string())),
    ];
    let file = attach(REPORT, &drafts);
    let (report, appendix) = file.split_at(file.find(APPENDIX).unwrap());
    assert_eq!(report, format!("{}\n\n", REPORT.trim_end()));
    let expected_head = "## Appendix: the drafts\n\n### claude\n\n##### Understanding\n";
    assert!(appendix.starts_with(expected_head), "{appendix}");
    assert!(
        appendix.contains("\n###### 1. Signed tokens\n"),
        "{appendix}"
    );
    assert!(
        appendix.ends_with("\n### codex\n\n(no draft: it crashed)\n"),
        "{appendix}"
    );
    // Inside the appendix, the only headings at or above the label's level are the
    // labels: the drafts' sections never pass for the report's.
    let doc = lines(&file);
    let top: Vec<&str> = (doc.iter())
        .filter_map(|l| heading(l).filter(|(n, _)| *n <= 3).map(|(_, t)| t))
        .skip_while(|t| *t != "Appendix: the drafts")
        .collect();
    assert_eq!(top, ["Appendix: the drafts", "claude", "codex"]);
}

/// A fence the report or a draft leaves open is closed before what follows, so the
/// appendix's heading and the next label are never code.
#[test]
fn an_open_fence_is_closed_before_the_appendix_and_each_label() {
    let report = format!("{REPORT}```\nnot closed\n");
    let draft = format!("{DRAFT}~~~~\nnot closed either\n");
    let drafts = vec![
        ("claude".to_string(), Ok(draft)),
        ("codex".to_string(), Ok(DRAFT.to_string())),
    ];
    let file = attach(&report, &drafts);
    let headings: Vec<&str> = (lines(&file).iter())
        .filter_map(|l| heading(l).filter(|(n, _)| *n <= 3).map(|(_, t)| t))
        .collect();
    assert!(
        headings.ends_with(&["Appendix: the drafts", "claude", "codex"]),
        "{headings:?}"
    );
    assert_eq!(split(&file).0.trim_end(), format!("{report}```").trim_end());
}

/// The engine's appendix is the last `## Appendix: the drafts` line outside code;
/// [`split`] returns what comes before it, and [`body`] does so for a brainstorm only.
#[test]
fn split_finds_the_engines_appendix() {
    let drafts = vec![("claude".to_string(), Ok(DRAFT.to_string()))];
    let file = attach(REPORT, &drafts);
    let (report, appendix) = split(&file);
    assert_eq!(report.trim_end(), REPORT.trim_end());
    assert!(appendix.unwrap().starts_with(APPENDIX));
    assert_eq!(split(REPORT), (REPORT, None));
    // A report quoting the heading, and a draft holding it in a code block or as a
    // heading of its own (demoted), leave the engine's as the one found.
    let quoting = format!("{REPORT}\n{APPENDIX}\nmine\n");
    let hostile = format!("{DRAFT}```\n{APPENDIX}\n```\n{APPENDIX}\n");
    let file = attach(&quoting, &[("codex".to_string(), Ok(hostile))]);
    assert_eq!(split(&file).0.trim_end(), quoting.trim_end());
    assert_eq!(
        body(DocKind::Brainstorm, &file).trim_end(),
        quoting.trim_end()
    );
    assert_eq!(body(DocKind::Spec, &file), file);
}

/// DF §6.1: the Review panel's agree and disagree counts and each approach's tag, read
/// from the report: a section's points are its sub-headings, else its list items, else
/// its paragraphs.
#[test]
fn the_summary_counts_points_and_tags_each_approach() {
    assert_eq!(
        summary(REPORT, &labels()),
        ReportSummary {
            agree: 1,
            disagree: 1,
            approaches: vec![
                ApproachTag {
                    name: "Signed tokens".into(),
                    tag: "claude".into(),
                },
                ApproachTag {
                    name: "Stored tokens".into(),
                    tag: "both".into(),
                },
            ],
        }
    );
    let listed = REPORT
        .replace(
            "Both want tokens.\n",
            "- tokens\n  - nested, not a point\n* mail\n1. a table\n",
        )
        .replace(
            "claude: stateless. codex: stored. Judgment: stored.\n",
            "### Storage\nclaude: none.\n### Expiry\ncodex: an hour.\n- a list under a heading\n",
        )
        .replace(
            "### 1. Signed tokens [claude]",
            "### 1. Signed tokens [CODEX] [both]",
        );
    let read = summary(&listed, &labels());
    assert_eq!((read.agree, read.disagree), (3, 2));
    assert_eq!(
        read.approaches[0].tag, "codex",
        "the first tag, as the label is spelled"
    );
    let paragraphs = REPORT.replace("Both want tokens.\n", "One.\nstill one.\n\nTwo.\n");
    assert_eq!(summary(&paragraphs, &labels()).agree, 2);
    // The appendix is not the report: its drafts' approaches are not counted.
    let file = attach(REPORT, &[("claude".to_string(), Ok(DRAFT.to_string()))]);
    assert_eq!(summary(&file, &labels()), summary(REPORT, &labels()));
}
