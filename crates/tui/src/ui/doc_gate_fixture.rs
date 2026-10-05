//! Milestone 9.6 task 17: the fixtures the gate screen's tests, the app's tests and the
//! render audit share: a design run waiting at its brainstorm gate (v1, two drafts) or
//! its spec gate (v2, a disputed finding), and the screen opened on it.

use crate::app::App;
use crate::app::screens::Screen;
use crate::settings::UiSettings;
use crate::tree::run_fixtures::{RUN_ID, gate_fixture};
use crate::ui::audit;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use proto::{
    ApproachTag, ClientMsg, DaemonMsg, DesignMode, DocAuthor, DocFinding, DocGateInfo, DocGateKind,
    DocInfo, DocKind, DocSeverity, DocView, ReportSummary, RevisingCause, RunReply, RunRequest,
    RunState,
};

/// The merged report at the brainstorm gate, with the engine's appendix.
pub(crate) const REPORT: &str = "\
# Brainstorm: password reset

## Where they agree
- Tokens expire.

## Where they disagree
claude: stateless. codex: stored. Judgment: stored.

## Approaches
### Signed tokens [claude]
Stateless.
### Stored tokens [codex]
A table.

## Recommendation
Stored tokens.

## Questions for you
- Which mailer?
- How long do tokens live?

## Appendix: the drafts

### claude
###### Understanding
Reset by mail.
";

/// The spec at its gate, v2.
pub(crate) const SPEC: &str = "\
# Password reset

## Requirements
R1 A user asks for a reset by mail.
R2 The link expires after an hour.

## Testing
```
cargo test reset
```
";

pub(crate) fn key(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
}

pub(crate) fn code(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn doc(kind: DocKind, version: u32, author: DocAuthor, reason: &str) -> DocInfo {
    DocInfo {
        kind,
        version,
        author,
        reason: reason.into(),
        bytes: 100,
        requirements: Vec::new(),
    }
}

fn brainstormer(label: &str) -> DocAuthor {
    DocAuthor::Brainstormer {
        label: label.into(),
    }
}

pub(crate) fn finding(id: &str, severity: DocSeverity, text: &str) -> DocFinding {
    DocFinding {
        id: id.into(),
        severity,
        place: "R2".into(),
        text: text.into(),
    }
}

/// The gate fixture's run in the design flow, waiting at its `kind` gate: the
/// brainstorm gate at v1 (drafts `claude` v1 and `codex` v2, the report's summary), or
/// the spec gate at v2 (two changes, the disputed `F2`).
pub(crate) fn design_run(kind: DocGateKind) -> proto::RunInfo {
    let (mut snap, _) = gate_fixture();
    let mut run = snap.runs.remove(0);
    run.tasks.clear();
    run.state = RunState::AwaitingApproval;
    run.design = DesignMode::Full;
    let o = DocAuthor::Orchestrator;
    run.docs = vec![
        doc(DocKind::BrainstormDraft, 1, brainstormer("claude"), "draft"),
        doc(DocKind::BrainstormDraft, 2, brainstormer("codex"), "draft"),
        doc(
            DocKind::Brainstorm,
            1,
            o.clone(),
            "merged by the orchestrator",
        ),
    ];
    let gate = DocGateInfo {
        kind,
        version: 1,
        revising: None,
        disputed: Vec::new(),
        not_reviewed: None,
        changes_summary: Vec::new(),
        same_runtime: false,
        report: None,
        revising_cause: RevisingCause::Changes,
    };
    run.doc_gate = Some(match kind {
        DocGateKind::Brainstorm => DocGateInfo {
            report: Some(ReportSummary {
                agree: 1,
                disagree: 1,
                approaches: vec![
                    ApproachTag {
                        name: "Signed tokens".into(),
                        tag: "claude".into(),
                    },
                    ApproachTag {
                        name: "Stored tokens".into(),
                        tag: "codex".into(),
                    },
                ],
            }),
            ..gate
        },
        _ => {
            run.docs
                .push(doc(DocKind::Spec, 1, o.clone(), "first version"));
            let reason = "revised after your note: \"split R1\"";
            run.docs.push(doc(DocKind::Spec, 2, o, reason));
            DocGateInfo {
                version: 2,
                disputed: vec![finding("F2", DocSeverity::Minor, "Say which hour.")],
                changes_summary: vec!["+ R2".into(), "~ Testing: 2 lines".into()],
                ..gate
            }
        }
    });
    run
}

/// An app with `run` as its one run, on a `w`×`h` terminal.
pub(crate) fn app_with(run: proto::RunInfo, w: u16, h: u16) -> App {
    let (mut snap, windows) = gate_fixture();
    snap.runs = vec![run];
    let mut app = App::new(windows, "/tmp".into(), UiSettings::default());
    let _ = app.set_terminal_size(w, h);
    let _ = app.run_subscription();
    app.on_daemon(DaemonMsg::Run(RunReply::Snapshot(snap)));
    app
}

/// The tagged requests among `effects`, with their ids.
pub(crate) fn sent(effects: &[crate::app::Effect]) -> Vec<(u64, RunRequest)> {
    effects
        .iter()
        .filter_map(|e| match e {
            crate::app::Effect::Send(ClientMsg::RunTagged { id, request }) => {
                Some((*id, request.clone()))
            }
            _ => None,
        })
        .collect()
}

/// The daemon answers the tagged request `id` with `reply`.
pub(crate) fn reply(app: &mut App, reply: RunReply) {
    app.on_daemon(DaemonMsg::Run(reply));
}

pub(crate) fn view(kind: DocKind, version: u32, text: &str) -> DocView {
    DocView {
        run: RUN_ID.into(),
        kind,
        version,
        text: text.into(),
        diff: None,
        findings: Vec::new(),
        draft_review: None,
    }
}

/// The gate screen opened on `kind`'s gate of [`design_run`] and its version shown,
/// on a `w`×`h` terminal: the spec v2 with its findings answered.
pub(crate) fn opened_with(run: proto::RunInfo, w: u16, h: u16) -> App {
    let kind = run.doc_gate.as_ref().map(|g| g.kind).expect("a gate");
    let mut app = app_with(run, w, h);
    let effects = app.open_doc_gate(RUN_ID);
    let [(id, RunRequest::ShowDoc { version, .. })] = &sent(&effects)[..] else {
        panic!("one ShowDoc: {effects:?}");
    };
    let doc = match kind {
        DocGateKind::Brainstorm => view(DocKind::Brainstorm, version.unwrap(), REPORT),
        _ => DocView {
            findings: vec![
                (
                    finding("F1", DocSeverity::Blocking, "R1 has no check."),
                    Some("fixed".into()),
                ),
                (
                    finding("F2", DocSeverity::Minor, "Say which hour."),
                    Some("kept: the hour is in R2".into()),
                ),
            ],
            ..view(DocKind::Spec, version.unwrap(), SPEC)
        },
    };
    reply(
        &mut app,
        RunReply::Doc {
            doc: Box::new(doc),
            request_id: Some(*id),
        },
    );
    app
}

pub(crate) fn opened(kind: DocGateKind, w: u16, h: u16) -> App {
    opened_with(design_run(kind), w, h)
}

pub(crate) fn screen(app: &App) -> &crate::app::doc_gate::DocGateScreen {
    match &app.screen {
        Some(Screen::DocGate(s)) => s,
        other => panic!("the gate screen: {other:?}"),
    }
}

pub(crate) fn rows(app: &App, w: u16, h: u16) -> Vec<String> {
    audit::rows(&audit::draw(app, w, h))
}
