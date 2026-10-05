//! Task M9.6.16: the design flow's flags, and `run show`'s and `run status`'s texts,
//! which are pure. `design_tests_daemon.rs` drives the commands against a recorded fake
//! daemon.

use clap::Parser;
use clap::error::ErrorKind;
use proto::{
    ApproachTag, DesignMode, DocAuthor, DocFinding, DocGateInfo, DocGateKind, DocInfo, DocKind,
    DocSeverity, DocView, ReportSummary, RevisingCause, RoundDesign, RunInfo, RunState,
};

use super::super::RunCommand;
use super::super::status::{run_block, tests::example};
use super::*;

#[derive(Parser, Debug)]
pub(super) struct Cli {
    #[command(subcommand)]
    command: RunCommand,
}

pub(super) fn parse(args: &[&str]) -> Result<RunCommand, ErrorKind> {
    let mut all = vec!["run"];
    all.extend_from_slice(args);
    Cli::try_parse_from(all)
        .map(|cli| cli.command)
        .map_err(|e| e.kind())
}

pub(super) const ID: &str = "add-reset-3f9a";

/// The example run in the design flow, waiting at the spec gate with v2.
pub(super) fn design_run() -> RunInfo {
    let doc = |version: u32, reason: &str| DocInfo {
        kind: DocKind::Spec,
        version,
        author: DocAuthor::Orchestrator,
        reason: reason.into(),
        bytes: 900,
        requirements: vec!["R1".into(), "R2".into()],
    };
    RunInfo {
        state: RunState::AwaitingApproval,
        design: DesignMode::Full,
        doc_gate: Some(DocGateInfo {
            kind: DocGateKind::Spec,
            version: 2,
            revising: None,
            disputed: vec![finding("F2", DocSeverity::Minor)],
            not_reviewed: None,
            changes_summary: vec!["+ R2".into(), "~ Testing: 2 lines".into()],
            same_runtime: false,
            report: None,
            revising_cause: RevisingCause::Changes,
        }),
        docs: vec![
            doc(1, "first version"),
            doc(2, "revised after your note: \"split R1\""),
        ],
        ..example()
    }
}

/// The design run's spec v2, as `run show` returns it.
pub(super) fn spec_v2() -> DocView {
    DocView {
        run: ID.into(),
        kind: DocKind::Spec,
        version: 2,
        text: "# Reset\n".into(),
        diff: None,
        findings: Vec::new(),
    }
}

pub(super) fn finding(id: &str, severity: DocSeverity) -> DocFinding {
    DocFinding {
        id: id.into(),
        severity,
        place: "R1".into(),
        text: format!("{id}'s text"),
    }
}

/// The brief's test: `run show` prints the version's text on stdout, then its diff and
/// its findings when asked; the header, the gate's change summary, its disputed
/// findings and, at the brainstorm gate, the report's counts go to stderr. Everything
/// is drawn through `printable`, and a report's appendix line for a draft that could
/// not be read back is printed as it was stored.
#[test]
fn show_prints_the_document_the_diff_and_findings() {
    let info = design_run();
    let mut doc = spec_v2();
    doc.text = "# Reset\n\x1b[2JR1 reset by mail".into();
    doc.diff = Some("@@ -1,1 +1,2 @@\n # Reset\n+R1 reset by mail".into());
    doc.findings = vec![
        (finding("F1", DocSeverity::Blocking), Some("fixed".into())),
        (
            finding("F2", DocSeverity::Minor),
            Some("kept: out of scope".into()),
        ),
        (
            DocFinding {
                place: String::new(),
                ..finding("F3", DocSeverity::Minor)
            },
            None,
        ),
    ];
    let (out, err) = show_text(&info, &doc, (true, true));
    assert_eq!(
        out,
        "# Reset\n [2JR1 reset by mail\n\
         === diff against v1 ===\n\
         @@ -1,1 +1,2 @@\n # Reset\n+R1 reset by mail\n\
         === findings: 3 ===\n\
         F1 blocking at R1: F1's text\n  answer: fixed\n\
         F2 minor at R1: F2's text\n  answer: kept: out of scope\n\
         F3 minor: F3's text\n  no answer\n"
    );
    assert_eq!(
        err,
        "Spec · run add-reset-3f9a · v2 of 2 · revised after your note: \"split R1\"\n\
         changes: + R2; ~ Testing: 2 lines\n\
         disputed: F2\n"
    );
    // Without the flags: the text alone on stdout, with no newline added (ruling T16-2).
    let (out, _) = show_text(&info, &doc, (false, false));
    assert_eq!(out, "# Reset\n [2JR1 reset by mail");
    // v1: no diff, no findings, and no gate lines (the gate shows v2).
    let first = DocView {
        version: 1,
        diff: None,
        findings: Vec::new(),
        ..doc.clone()
    };
    let (out, err) = show_text(&info, &first, (true, true));
    assert!(
        out.ends_with("=== no earlier version to diff against ===\n=== findings: none ===\n"),
        "{out}"
    );
    assert_eq!(err, "Spec · run add-reset-3f9a · v1 of 2 · first version\n");

    // The brainstorm gate: the report's counts and tags, and an unreadable draft's
    // appendix line as stored.
    let mut info = design_run();
    let gate = info.doc_gate.as_mut().unwrap();
    gate.kind = DocGateKind::Brainstorm;
    gate.version = 1;
    gate.changes_summary.clear();
    gate.disputed.clear();
    gate.not_reviewed = Some("the reviewer timed out".into());
    gate.report = Some(ReportSummary {
        agree: 3,
        disagree: 2,
        approaches: vec![
            ApproachTag {
                name: "Mail link".into(),
                tag: "both".into(),
            },
            ApproachTag {
                name: "Admin reset".into(),
                tag: "codex".into(),
            },
        ],
    });
    info.docs = vec![DocInfo {
        kind: DocKind::Brainstorm,
        version: 1,
        author: DocAuthor::Orchestrator,
        reason: String::new(),
        bytes: 10,
        requirements: Vec::new(),
    }];
    let unread = "(no draft: its draft could not be read back: the file is missing)";
    let report = DocView {
        kind: DocKind::Brainstorm,
        version: 1,
        text: format!("# Report\n\n## Appendix: the drafts\n\n### codex\n\n{unread}\n"),
        diff: None,
        findings: Vec::new(),
        run: ID.into(),
    };
    let (out, err) = show_text(&info, &report, (false, false));
    assert_eq!(out, report.text);
    assert!(out.contains(unread));
    assert_eq!(
        err,
        "Brainstorm · run add-reset-3f9a · v1 of 1\n\
         not reviewed: the reviewer timed out\n\
         report: 3 agree, 2 disagree; approaches: Mail link [both], Admin reset [codex]\n"
    );
}

/// Ruling T16-2 (m4): `run show` prints the stored text exactly, so an unchanged show
/// and edit-doc round trip sends the same bytes back; a text without a final newline
/// gets none. A section asked for after it starts on its own line.
#[test]
fn show_prints_the_stored_bytes_exactly() {
    let info = design_run();
    let doc = DocView {
        text: "# Reset\n\nR1 reset by mail".into(),
        ..spec_v2()
    };
    let (out, _) = show_text(&info, &doc, (false, false));
    assert_eq!(out, doc.text);
    let (out, _) = show_text(&info, &doc, (false, true));
    assert_eq!(out, format!("{}\n=== findings: none ===\n", doc.text));
}

/// The brief's test: `run status` names the phase, and while a gate waits for the user
/// `waiting for you: <kind> v<n> (anthrex run show <run> --doc <kind>)`; while the
/// orchestrator revises, the revision and the user's note instead.
#[test]
fn status_names_the_waiting_gate() {
    let run = design_run();
    let block = run_block(&run, 0);
    let lines = "  design: at the spec gate\n  \
        waiting for you: spec v2 (anthrex run show add-reset-3f9a --doc spec)\n";
    assert_eq!(status_lines(&run), lines);
    assert!(block.contains(lines), "{block}");
    // After the orchestrator's lines, before the task table.
    assert!(block.find(lines) < block.find("  ID "), "{block}");

    let mut revising = design_run();
    let note = format!("split R1{}", "x".repeat(80));
    revising.doc_gate.as_mut().unwrap().revising = Some(format!("{note}\n\x1b[2J"));
    assert_eq!(
        status_lines(&revising),
        format!(
            "  design: at the spec gate, revising v3… (your note: \"{}\")\n",
            &note[..60]
        )
    );

    for (state, phase) in [
        (RunState::Brainstorming, "brainstorming"),
        (RunState::Specifying, "specifying"),
        (RunState::Planning, "planning"),
    ] {
        let run = RunInfo {
            state,
            doc_gate: None,
            ..design_run()
        };
        assert_eq!(status_lines(&run), format!("  design: {phase}\n"));
        // Paused there: the phase it paused in, and nothing waits for the user.
        let paused = RunInfo {
            state: RunState::Paused,
            paused_from: Some(state),
            ..run
        };
        assert_eq!(status_lines(&paused), format!("  design: {phase}\n"));
    }
    let paused_at_gate = RunInfo {
        state: RunState::Paused,
        paused_from: Some(RunState::AwaitingApproval),
        ..design_run()
    };
    assert_eq!(
        status_lines(&paused_at_gate),
        "  design: at the spec gate\n"
    );
    // Past the plan gate the design flow has nothing to say.
    let running = RunInfo {
        state: RunState::Running,
        doc_gate: None,
        ..design_run()
    };
    assert_eq!(status_lines(&running), "");
}

/// Task M9.6.17 (task 16's m3): a design run's `off` round says so where the phase line
/// would be (planning, its 9.3 plan gate, paused there), and a halted design run names
/// the phase its plain resume returns to. A read-back revision's note is anthrex's, so
/// it is not called the user's.
#[test]
fn status_names_an_off_round_and_a_halted_phase() {
    let off = RunInfo {
        state: RunState::Planning,
        doc_gate: None,
        round: 2,
        round_design: Some(RoundDesign::Off),
        ..design_run()
    };
    assert_eq!(status_lines(&off), "  design: off this round\n");
    for (state, paused_from) in [
        (RunState::AwaitingApproval, None),
        (RunState::Paused, Some(RunState::Planning)),
    ] {
        let run = RunInfo {
            state,
            paused_from,
            ..off.clone()
        };
        assert_eq!(
            status_lines(&run),
            "  design: off this round\n",
            "{state:?}"
        );
    }
    let running = RunInfo {
        state: RunState::Running,
        ..off.clone()
    };
    assert_eq!(status_lines(&running), "");
    let amend = RunInfo {
        state: RunState::Specifying,
        round_design: Some(RoundDesign::Amend),
        ..off
    };
    assert_eq!(status_lines(&amend), "  design: specifying\n");

    for (phase, word) in [
        (RunState::Brainstorming, "brainstorming"),
        (RunState::Specifying, "specifying"),
        (RunState::Planning, "planning"),
    ] {
        let halted = RunInfo {
            state: RunState::Halted,
            halted_phase: Some(phase),
            ..design_run()
        };
        assert_eq!(
            status_lines(&halted),
            format!("  design: halted in {word}\n")
        );
        let block = run_block(&halted, 0);
        assert!(
            block.contains(&format!("  design: halted in {word}\n")),
            "{block}"
        );
    }
    // Halted past the plan gate (or in an off round): 9.3's status.
    let halted = RunInfo {
        state: RunState::Halted,
        doc_gate: None,
        ..design_run()
    };
    assert_eq!(status_lines(&halted), "");

    let mut read_back = design_run();
    let gate = read_back.doc_gate.as_mut().unwrap();
    gate.revising = Some("anthrex could not read back the stored spec v2; submit it again".into());
    gate.revising_cause = RevisingCause::ReadBack;
    assert_eq!(
        status_lines(&read_back),
        "  design: at the spec gate, revising v3… \
         (anthrex could not read back the stored spec v2; submit it ag)\n"
    );
    read_back.doc_gate.as_mut().unwrap().revising_cause = RevisingCause::Back;
    assert!(status_lines(&read_back).contains("(your note: \"anthrex could not"));
}

/// A run without the design flow prints exactly what it printed before milestone 9.6,
/// byte for byte, whatever its state.
#[test]
fn a_non_design_runs_status_is_unchanged() {
    assert_eq!(
        run_block(&example(), 0),
        include_str!("status_single_golden.txt")
    );
    for state in [
        RunState::Planning,
        RunState::AwaitingApproval,
        RunState::Brainstorming,
    ] {
        let off = RunInfo {
            state,
            design: DesignMode::Off,
            ..design_run()
        };
        assert_eq!(status_lines(&off), "", "{state:?}");
        let block = run_block(&off, 0);
        assert!(!block.contains("design:") && !block.contains("waiting for you"));
    }
}

/// The brief's test: `--design` on start and iterate, `--gate` on approve, and the
/// design commands' flags; an unknown value is clap's usage error.
#[test]
fn design_flags_are_parsed() {
    let design = |args: &[&str]| match parse(args).unwrap() {
        RunCommand::Start { design, .. } => design,
        other => panic!("{other:?}"),
    };
    assert_eq!(
        design(&["start", "--goal", "g", "--design", "full"]),
        Some(DesignArg::Full)
    );
    assert_eq!(
        design(&["start", "--goal", "g", "--design", "off"]),
        Some(DesignArg::Off)
    );
    assert_eq!(design(&["start", "--goal", "g"]), None);
    // A plan file's run is the daemon's to refuse.
    assert_eq!(
        design(&["start", "--plan", "p.toml", "--design", "full"]),
        Some(DesignArg::Full)
    );
    assert_eq!(DesignMode::from(DesignArg::Full), DesignMode::Full);
    assert_eq!(DesignMode::from(DesignArg::Off), DesignMode::Off);
    for bad in ["amend", "on", "Full", ""] {
        assert_eq!(
            parse(&["start", "--goal", "g", "--design", bad]).unwrap_err(),
            ErrorKind::InvalidValue,
            "{bad:?}"
        );
    }

    let round = |args: &[&str]| match parse(args).unwrap() {
        RunCommand::Iterate { design, .. } => design.map(RoundDesign::from),
        other => panic!("{other:?}"),
    };
    for (value, want) in [
        ("amend", RoundDesign::Amend),
        ("full", RoundDesign::Full),
        ("off", RoundDesign::Off),
    ] {
        assert_eq!(round(&["iterate", "r", "t", "--design", value]), Some(want));
    }
    assert_eq!(round(&["iterate", "r", "t"]), None);
    assert_eq!(
        parse(&["iterate", "r", "t", "--design", "yes"]).unwrap_err(),
        ErrorKind::InvalidValue
    );

    match parse(&["approve", "r", "--gate", "brainstorm"]).unwrap() {
        RunCommand::Approve { gate, hold, .. } => {
            assert_eq!((gate.gate, hold), (Some(GateArg::Brainstorm), None))
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(
        parse(&["approve", "r", "--gate", "spec", "--hold", "h1"]).unwrap_err(),
        ErrorKind::ArgumentConflict
    );
    for bad in [
        vec!["approve", "r", "--gate", "draft"],
        vec!["show", "r", "--doc", "brainstorm_draft"],
        vec!["back", "r", "--gate", "specs", "--note", "n"],
    ] {
        assert_eq!(parse(&bad).unwrap_err(), ErrorKind::InvalidValue, "{bad:?}");
    }
    for missing in [
        vec!["show", "r"],
        vec!["changes", "r", "--gate", "spec"],
        vec!["edit-doc", "r", "--gate", "spec"],
        vec!["rethink", "r"],
        vec!["back", "r", "--note", "n"],
    ] {
        assert_eq!(
            parse(&missing).unwrap_err(),
            ErrorKind::MissingRequiredArgument,
            "{missing:?}"
        );
    }

    // `--review` and `--no-review`: the last one given wins; no review by default.
    let review = |extra: &[&str]| {
        let mut args = vec!["changes", "r", "--gate", "spec", "--note", "n"];
        args.extend_from_slice(extra);
        match parse(&args).unwrap() {
            RunCommand::Design(DesignCommand::Changes { review, .. }) => review,
            other => panic!("{other:?}"),
        }
    };
    assert!(!review(&[]));
    assert!(review(&["--review"]));
    assert!(!review(&["--no-review"]));
    assert!(!review(&["--review", "--no-review"]));
    assert!(review(&["--no-review", "--review"]));
}

/// The help names `--review`'s default (decision 15: a revision after the user's
/// changes is not reviewed unless asked).
#[test]
fn changes_help_names_the_review_default() {
    use clap::CommandFactory;
    let mut cli = Cli::command();
    let changes = cli.find_subcommand_mut("changes").expect("run changes");
    let help = changes.render_long_help().to_string();
    assert!(help.contains("(default: --no-review)"), "{help}");
}

/// Ruling T17-1: `run approve --gate <kind> --version <n>` names the version reviewed;
/// `--version` needs `--gate`, and without it the approve is unchanged.
#[test]
fn approve_takes_the_version_reviewed() {
    let version = |args: &[&str]| match parse(args).unwrap() {
        RunCommand::Approve { gate, .. } => (gate.gate, gate.version),
        other => panic!("{other:?}"),
    };
    assert_eq!(
        version(&["approve", "r", "--gate", "spec", "--version", "2"]),
        (Some(GateArg::Spec), Some(2))
    );
    assert_eq!(
        version(&["approve", "r", "--gate", "spec"]),
        (Some(GateArg::Spec), None)
    );
    assert_eq!(version(&["approve", "r"]), (None, None));
    assert_eq!(
        parse(&["approve", "r", "--version", "2"]).unwrap_err(),
        ErrorKind::MissingRequiredArgument
    );
    assert_eq!(
        parse(&["approve", "r", "--gate", "spec", "--version", "v2"]).unwrap_err(),
        ErrorKind::ValueValidation
    );
    assert_eq!(
        parse(&["approve", "r", "--gate", "spec", "--hold", "h"]).unwrap_err(),
        ErrorKind::ArgumentConflict
    );
}
