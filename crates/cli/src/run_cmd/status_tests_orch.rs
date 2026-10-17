//! Milestone 9.9 task 9: `run status` prints what the orchestrator handled and the
//! question it waits on (OFA §4.6).

use common::numbers::Zone;
use proto::{AskInfo, Effort, HandledInfo, OrchestratorInfo, RunState, Runtime, TaskState};

use super::tests::{example, route};
use super::{printable, run_block};

fn orchestrator() -> OrchestratorInfo {
    OrchestratorInfo {
        route: serde_json::from_value(serde_json::json!({
            "runtime": "claude", "model": "", "strength": "frontier", "effort": "high",
        }))
        .expect("a Route"),
        window_id: Some(12),
        live: true,
        started_at: 1,
        plan_submitted: true,
        summary: None,
        notes: vec![],
        wakes: 0,
        wake_held: false,
        stuck: None,
        ask: None,
        handled: vec![],
        handled_total: 0,
        untrusted: None,
    }
}

#[test]
fn status_prints_handled_and_the_pending_question() {
    let mut run = example();
    run.orchestrator = Some(orchestrator());
    let quiet = run_block(&run, 0, Zone::Local(0));
    assert!(!quiet.contains("orchestrator handled"), "{quiet}");
    assert!(!quiet.contains("orchestrator asks"), "{quiet}");

    let o = run.orchestrator.as_mut().unwrap();
    o.handled_total = 2;
    o.handled = vec![
        HandledInfo {
            at: 1,
            op: "retry".into(),
            target: "t2".into(),
            reason: "flaky".into(),
        },
        HandledInfo {
            at: 2,
            op: "override".into(),
            target: "t3".into(),
            reason: "ok".into(),
        },
    ];
    o.ask = Some(AskInfo {
        id: 1,
        question: "tabs or spaces?".into(),
        options: vec!["tabs".into(), "spaces".into()],
        context: String::new(),
        asked_at: 5,
    });
    let out = run_block(&run, 0, Zone::Local(0));
    let lines: Vec<&str> = out.lines().collect();
    let at = lines
        .iter()
        .position(|l| *l == "  orchestrator handled 2")
        .unwrap_or_else(|| panic!("{out}"));
    assert_eq!(
        &lines[at + 1..at + 4],
        [
            "  orchestrator asks: tabs or spaces?",
            "    1. tabs",
            "    2. spaces"
        ],
        "{out}"
    );
}

/// Task 9 M2: the ask's question and options are daemon-supplied text from a model; a
/// line break, an escape sequence, a bidi override and a zero-width joiner each stay
/// off the terminal, and the question and every option keep their one line, whether or
/// not `printable` runs after. Mutant: `one_line` removed from either, red.
#[test]
fn status_ask_lines_are_sanitised() {
    let mut run = example();
    let mut o = orchestrator();
    o.ask = Some(AskInfo {
        id: 1,
        question: "tabs\nor\x1b[2J spa\u{202E}ces\u{200D}?\x07".into(),
        options: vec![
            "ta\u{202E}bs\x1b]0;pwn\x07\nsecond".into(),
            "sp\u{200D}aces\r\x1b[31m".into(),
        ],
        context: String::new(),
        asked_at: 5,
    });
    run.orchestrator = Some(o);
    let raw = run_block(&run, 0, Zone::Local(0));
    // The block alone: every control character a space, so the ask keeps its lines.
    assert!(!raw.contains(['\x1b', '\x07', '\r']), "{raw:?}");
    // What the terminal is given: the hidden format characters gone too.
    let shown = printable(&raw);
    assert!(
        !shown.contains(['\x1b', '\x07', '\r', '\u{202E}', '\u{200D}']),
        "{shown:?}"
    );
    for out in [raw, shown] {
        let lines: Vec<&str> = out.lines().collect();
        let at = lines
            .iter()
            .position(|l| l.starts_with("  orchestrator asks: "))
            .unwrap_or_else(|| panic!("{out:?}"));
        let plain = |l: &str| {
            l.replace(['\u{202E}', '\u{200D}'], "")
                .trim_end()
                .to_owned()
        };
        assert_eq!(
            plain(lines[at]),
            "  orchestrator asks: tabs or [2J spaces?",
            "{out:?}"
        );
        assert_eq!(
            plain(lines[at + 1]),
            "    1. tabs ]0;pwn  second",
            "{out:?}"
        );
        assert_eq!(plain(lines[at + 2]), "    2. spaces  [31m", "{out:?}");
        assert!(lines[at + 3].starts_with("  ID "), "{out:?}");
    }
}

/// Milestone 9.13 decision 22: the taint under the orchestrator, its source on one line
/// with control characters gone, the time the viewer's labelled clock (milestone 9.19).
#[test]
fn untrusted_line_is_printed() {
    let mut run = example();
    run.orchestrator = Some(orchestrator());
    assert!(!run_block(&run, 0, Zone::Local(0)).contains("untrusted"));
    run.orchestrator.as_mut().unwrap().untrusted = Some(proto::UntrustedInfo {
        // 2026-10-12 14:07:59Z.
        at: 1_791_814_079,
        source: "review comments\n\x1b[2Jon stage 1's pull request".into(),
    });
    let out = run_block(&run, 1_791_814_079, Zone::Local(0));
    let lines: Vec<&str> = out.lines().collect();
    let at = lines
        .iter()
        .position(|l| l.starts_with("  orchestrator: "))
        .unwrap_or_else(|| panic!("{out}"));
    assert_eq!(
        lines[at + 1],
        "  untrusted text read: review comments  [2Jon stage 1's pull request at 14:07 +00:00; override, accept_red and approve_hold go to the user",
        "{out:?}"
    );
}

/// Milestone 9 (the brief's CLI section): the orchestrator, planners, holds and summary
/// lines, `planning` as a state, and the task table's `paused(message)`, `reported`
/// and ` (held)`.
#[test]
fn status_shows_orchestrator_planners_holds_summary_and_paused_lines() {
    use proto::{BlockInfo, BlockReason, HoldInfo, HoldKind, HoldState, OrchestratorInfo};
    use proto::{PlannerInfo, PlannerState};
    let mut run = example();
    assert!(!run_block(&run, 0, Zone::Local(0)).contains("orchestrator:"));
    run.state = RunState::Planning;
    run.orchestrator = Some(OrchestratorInfo {
        route: route(Runtime::Claude, "", Effort::HIGH),
        window_id: Some(12),
        live: true,
        started_at: 1,
        plan_submitted: false,
        summary: None,
        notes: vec![],
        wakes: 0,
        wake_held: false,
        stuck: None,
        ask: None,
        handled: vec![],
        handled_total: 0,
        untrusted: None,
    });
    let planner = |epic: &str, state| PlannerInfo {
        epic: epic.into(),
        title: epic.into(),
        area: vec![],
        route: route(Runtime::Codex, "", Effort::MEDIUM),
        window_id: None,
        state,
        started_at: 1,
        ended_at: None,
        edits_accepted: 0,
        edits_rejected: 0,
        last_rejection: None,
        replans: vec![],
        note: None,
        covers: Vec::new(),
        frame: None,
    };
    run.planners = vec![
        planner("mail", PlannerState::Finished),
        planner("api", PlannerState::Planning),
    ];
    let hold = |id: &str, state, tasks: &[&str]| HoldInfo {
        id: id.into(),
        kind: HoldKind::Promotion,
        state,
        tasks: tasks.iter().map(|t| t.to_string()).collect(),
        created_at: 1,
        decided_at: None,
        decided_by: None,
    };
    run.holds = vec![
        hold("promotion", HoldState::Approved, &["t1"]),
        hold("epic:mail", HoldState::Awaiting, &["t3", "t4"]),
    ];
    run.tasks[2].epic = Some("mail".into());
    run.tasks[2].hold = Some("epic:mail".into());
    run.tasks[1].hold = Some("promotion".into());
    run.tasks[3].epic = Some("mail".into());
    run.tasks[3].block = Some(BlockInfo::new(
        BlockReason::MessagePause,
        "asked to stop and wait: hold on",
    ));
    run.tasks[0].state = TaskState::Reported;
    let text = run_block(&run, 0, Zone::Local(0));
    let lines: Vec<&str> = text.lines().collect();
    assert!(
        lines[0].starts_with("add-reset-3f9a  planning  0/4 merged  "),
        "{text}"
    );
    assert_eq!(
        &lines[3..6],
        [
            "  orchestrator: window 12, claude (default), live",
            "  planners: mail finished (2 tasks), api planning",
            "  holds: promotion approved (1 task), epic:mail awaiting (2 tasks)",
        ],
        "{text}"
    );
    assert_eq!(
        &lines[7..11],
        [
            "  t1   M◆   tdd    reported       0    -              claude claude-opus-5 high      4 5",
            "  t2   M    tdd    review         1    review 1       codex (default) medium         6 9",
            "  t3   S    check  queued (held)  0    -              claude claude-sonnet-5 low",
            "  t4   S    none   paused(message) 3    check 3        claude claude-sonnet-5 low     7",
        ],
        "{text}"
    );

    let o = run.orchestrator.as_mut().unwrap();
    o.live = false;
    o.window_id = None;
    o.summary = Some("All done.".into());
    o.route = route(Runtime::Codex, "gpt-5", Effort::HIGH);
    run.planners.clear();
    run.holds.clear();
    let text = run_block(&run, 0, Zone::Local(0));
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(
        &lines[3..5],
        [
            "  orchestrator: window -, codex gpt-5, exited",
            "  summary: written",
        ],
        "{text}"
    );
    assert!(
        !text.contains("planners:") && !text.contains("holds:"),
        "{text}"
    );
}

/// Milestone 9.19 decision 3 (ruling R1): `run status` is a client, so the promotion and
/// untrusted lines are the viewer's clock, labelled with the offset passed in, and
/// `+00:00` (never `UTC`) at offset 0. Another day names it.
#[test]
fn run_status_times_are_labelled() {
    // 2026-10-17 14:30 UTC, a Saturday.
    const NOW: u64 = 1_792_247_400;
    let mut run = example();
    run.promote_requested_at = Some(NOW - 600);
    run.orchestrator = Some(orchestrator());
    run.orchestrator.as_mut().unwrap().untrusted = Some(proto::UntrustedInfo {
        at: NOW - 86_400,
        source: "review comments".into(),
    });
    let line = |out: &str, start: &str| -> String {
        let found = out.lines().find(|l| l.starts_with(start));
        found.unwrap_or_else(|| panic!("{out}")).to_owned()
    };
    for (zone, promoted, read) in [
        (Zone::Local(7200), "16:20 +02:00", "Fri 16:30 +02:00"),
        (Zone::Local(0), "14:20 +00:00", "Fri 14:30 +00:00"),
        (Zone::Local(-18_000), "09:20 -05:00", "Fri 09:30 -05:00"),
    ] {
        let out = run_block(&run, NOW, zone);
        assert_eq!(
            line(&out, "  promotion: "),
            format!("  promotion: requested at {promoted}")
        );
        let untrusted = line(&out, "  untrusted text read: ");
        assert!(
            untrusted.starts_with(&format!(
                "  untrusted text read: review comments at {read}; "
            )),
            "{untrusted}"
        );
        assert!(!out.contains(" UTC"), "a client never prints UTC: {out}");
    }
}
