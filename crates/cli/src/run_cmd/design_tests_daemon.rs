//! Task M9.6.16: each design command parsed by clap and driven through
//! `run_cmd::dispatch` against a recorded fake daemon (a real Unix socket in a temp dir,
//! no daemon behind it), which answers the runs list with a design run, `run show` with
//! a document, and records every other request.

use std::path::Path;

use clap::error::ErrorKind;
use proto::{
    ClientMsg, DaemonMsg, DocGateAction, DocGateKind, DocKind, DocView, RoundDesign, RunInfo,
    RunReply, RunRequest, RunsSnapshot, read_frame, write_frame,
};
use tokio::sync::mpsc;

use super::super::dispatch;
use super::tests::{ID, design_run, parse, spec_v2};
use super::{EDIT_CAP, read_capped};

/// What the fake daemon answers: the runs list, `run show`'s document, and every other
/// request `Done`, or `Refused` with `refusal`.
#[derive(Clone)]
struct Answers {
    runs: Vec<RunInfo>,
    doc: DocView,
    refusal: Option<String>,
}

fn answers(refusal: Option<&str>) -> Answers {
    Answers {
        runs: vec![design_run()],
        doc: spec_v2(),
        refusal: refusal.map(str::to_string),
    }
}

/// A fake daemon on `socket`: every connection is greeted, and every request but the
/// runs list is sent to the returned receiver before it is answered.
fn fake_daemon(socket: &Path, answers: Answers) -> mpsc::UnboundedReceiver<RunRequest> {
    let listener = tokio::net::UnixListener::bind(socket).unwrap();
    let (tx, rx) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                return;
            };
            let (tx, answers) = (tx.clone(), answers.clone());
            tokio::spawn(async move {
                let (mut rd, mut wr) = stream.into_split();
                let hello = read_frame::<_, ClientMsg>(&mut rd).await.unwrap().unwrap();
                assert!(matches!(hello, ClientMsg::Hello { .. }));
                let welcome = DaemonMsg::Welcome {
                    daemon_version: "test".into(),
                    windows: vec![],
                };
                write_frame(&mut wr, &welcome).await.unwrap();
                while let Ok(Some(ClientMsg::Run(request))) = read_frame(&mut rd).await {
                    let reply = match (&request, &answers.refusal) {
                        (RunRequest::List, _) => RunReply::Snapshot(RunsSnapshot {
                            revision: 1,
                            runs: answers.runs.clone(),
                            now: 0,
                            proposals: Vec::new(),
                            idle_orchestrators: Vec::new(),
                        }),
                        (_, Some(refusal)) => RunReply::refused("run", refusal.clone()),
                        (RunRequest::ShowDoc { .. }, None) => RunReply::Doc {
                            doc: Box::new(answers.doc.clone()),
                            request_id: None,
                        },
                        (_, None) => RunReply::done("run", "ok"),
                    };
                    if !matches!(request, RunRequest::List) {
                        tx.send(request).unwrap();
                    }
                    write_frame(&mut wr, &DaemonMsg::Run(reply)).await.unwrap();
                }
            });
        }
    });
    rx
}

/// Every request the fake daemon received so far.
fn received(rx: &mut mpsc::UnboundedReceiver<RunRequest>) -> Vec<RunRequest> {
    let mut all = Vec::new();
    while let Ok(request) = rx.try_recv() {
        all.push(request);
    }
    all
}

fn gate(kind: DocGateKind, action: DocGateAction) -> RunRequest {
    RunRequest::DocGate {
        run: ID.into(),
        kind,
        action,
    }
}

/// `anthrex run <args>` through the CLI's own dispatch, against `socket`.
async fn command(socket: &Path, args: &[&str]) -> anyhow::Result<()> {
    let parsed = parse(args).unwrap_or_else(|e| panic!("{args:?}: {e:?}"));
    let bound = std::time::Duration::from_secs(20);
    tokio::time::timeout(bound, dispatch(parsed, socket, None))
        .await
        .expect("answered")
}

/// The brief's test: each command sends exactly its request, to the run its argument
/// names by a suffix.
#[tokio::test]
async fn each_command_sends_its_request() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("d.sock");
    let mut rx = fake_daemon(&socket, answers(None));
    let file = dir.path().join("spec.md");
    std::fs::write(&file, "# Reset\n\n## Goal\n").unwrap();
    let file = file.display().to_string();
    let spec = DocGateKind::Spec;
    let cases: Vec<(Vec<&str>, RunRequest)> = vec![
        (
            vec!["approve", "3f9a", "--gate", "spec"],
            gate(spec, DocGateAction::Approve),
        ),
        (
            vec!["approve", "3f9a", "--gate", "plan"],
            gate(DocGateKind::Plan, DocGateAction::Approve),
        ),
        // A plain approve is 9.5's request; the daemon decides what it means here.
        (
            vec!["approve", "3f9a"],
            RunRequest::Approve { run_id: ID.into() },
        ),
        (
            vec!["changes", "3f9a", "--gate", "spec", "--note", "split R1"],
            gate(
                spec,
                DocGateAction::Changes {
                    note: "split R1".into(),
                    review: false,
                },
            ),
        ),
        (
            vec![
                "changes",
                "3f9a",
                "--gate",
                "brainstorm",
                "--note",
                "n",
                "--review",
            ],
            gate(
                DocGateKind::Brainstorm,
                DocGateAction::Changes {
                    note: "n".into(),
                    review: true,
                },
            ),
        ),
        (
            vec!["edit-doc", "3f9a", "--gate", "spec", "--file", &file],
            gate(
                spec,
                DocGateAction::Edit {
                    text: "# Reset\n\n## Goal\n".into(),
                },
            ),
        ),
        (
            vec!["rethink", "3f9a", "--note", "smaller"],
            gate(
                DocGateKind::Brainstorm,
                DocGateAction::Rethink {
                    note: "smaller".into(),
                },
            ),
        ),
        (
            vec!["back", "3f9a", "--gate", "plan", "--note", "R3 is wrong"],
            gate(
                DocGateKind::Plan,
                DocGateAction::Back {
                    note: "R3 is wrong".into(),
                },
            ),
        ),
        (
            vec!["show", "3f9a", "--doc", "spec"],
            RunRequest::ShowDoc {
                run: ID.into(),
                kind: DocKind::Spec,
                version: None,
                diff: false,
                findings: false,
            },
        ),
        (
            vec![
                "show",
                "3f9a",
                "--doc",
                "brainstorm",
                "--version",
                "1",
                "--diff",
                "--findings",
            ],
            RunRequest::ShowDoc {
                run: ID.into(),
                kind: DocKind::Brainstorm,
                version: Some(1),
                diff: true,
                findings: true,
            },
        ),
        (
            vec!["iterate", "3f9a", "add a link", "--design", "full"],
            RunRequest::Iterate {
                run: ID.into(),
                goal: "add a link".into(),
                design: Some(RoundDesign::Full),
            },
        ),
        (
            vec!["iterate", "3f9a", "add a link"],
            RunRequest::Iterate {
                run: ID.into(),
                goal: "add a link".into(),
                design: None,
            },
        ),
    ];
    for (args, want) in cases {
        command(&socket, &args)
            .await
            .unwrap_or_else(|e| panic!("{args:?}: {e}"));
        assert_eq!(received(&mut rx), [want], "{args:?}");
    }
}

/// The daemon's refusals are the command's error, as the daemon wrote them: ruling
/// T1-O1's for a plain approve at the brainstorm gate, ruling T11-3's for an edit of the
/// plan, and M9.6.15's for `iterate --design` on a run without the flow. The CLI sent
/// each one, and decided none of them.
#[tokio::test]
async fn refusals_are_the_daemons_text() {
    let cases = [
        (
            vec!["approve", "3f9a"],
            "run add-reset-3f9a is waiting at the brainstorm gate; approve it with anthrex run approve add-reset-3f9a --gate brainstorm",
        ),
        (
            vec!["edit-doc", "3f9a", "--gate", "plan", "--file", "FILE"],
            "the plan is made from its tasks; change them with anthrex run edit",
        ),
        (
            vec!["iterate", "3f9a", "more", "--design", "amend"],
            "this run has no spec to amend; iterate with --design off",
        ),
        (
            vec!["back", "3f9a", "--gate", "brainstorm", "--note", "n"],
            "back is only for the spec and plan gates",
        ),
        (
            vec!["show", "3f9a", "--doc", "plan", "--version", "7"],
            "run add-reset-3f9a has no plan v7",
        ),
    ];
    for (args, refusal) in cases {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("d.sock");
        let mut rx = fake_daemon(&socket, answers(Some(refusal)));
        let file = dir.path().join("plan.md");
        std::fs::write(&file, "# Plan\n").unwrap();
        let file = file.display().to_string();
        let args: Vec<&str> = args
            .iter()
            .map(|a| if *a == "FILE" { file.as_str() } else { a })
            .collect();
        let error = command(&socket, &args).await.expect_err("refused");
        assert_eq!(error.to_string(), refusal, "{args:?}");
        assert_eq!(received(&mut rx).len(), 1, "{args:?}: sent once");
    }
}

/// The brief's test: `edit-doc` reads the whole file up to [`EDIT_CAP`], the spec's own
/// cap and the most `run show` prints, so a shown version sent back always fits. A
/// larger file is a usage error (clap's, exit 2) before anything is sent.
#[tokio::test]
async fn edit_doc_reads_the_file_and_caps_it() {
    use daemon::run::design::template::cap_bytes;
    assert_eq!(EDIT_CAP, cap_bytes(DocKind::Spec));
    assert!(EDIT_CAP >= cap_bytes(DocKind::Brainstorm));

    let dir = tempfile::tempdir().unwrap();
    let full = dir.path().join("full.md");
    let text = format!("# T\n{}", "é".repeat((EDIT_CAP - 4) / 2));
    assert_eq!(text.len(), EDIT_CAP);
    std::fs::write(&full, &text).unwrap();
    assert_eq!(read_capped(&full).unwrap(), text);

    let over = dir.path().join("over.md");
    std::fs::write(&over, format!("{text}x")).unwrap();
    let error = read_capped(&over).unwrap_err();
    let usage = error.downcast_ref::<clap::Error>().expect("a usage error");
    assert_eq!(usage.kind(), ErrorKind::ValueValidation);
    assert_eq!(
        usage.to_string(),
        format!(
            "error: --file {}: the file is over the 64 KiB a document may be; nothing was sent\n",
            over.display()
        )
    );

    let missing = dir.path().join("missing.md");
    let error = read_capped(&missing).unwrap_err();
    assert!(error.downcast_ref::<clap::Error>().is_none());
    assert!(
        error
            .to_string()
            .starts_with(&format!("cannot read {}: ", missing.display())),
        "{error}"
    );
    std::fs::write(&missing, [0xff, 0xfe]).unwrap();
    assert_eq!(
        read_capped(&missing).unwrap_err().to_string(),
        format!("{} is not UTF-8 text", missing.display())
    );

    // Through the command: the usage error, and nothing reaches the daemon.
    let socket = dir.path().join("d.sock");
    let mut rx = fake_daemon(&socket, answers(None));
    let over = over.display().to_string();
    let args = [
        "edit-doc",
        "3f9a",
        "--gate",
        "spec",
        "--file",
        over.as_str(),
    ];
    let error = command(&socket, &args).await.unwrap_err();
    assert!(error.downcast_ref::<clap::Error>().is_some(), "{error}");
    assert!(received(&mut rx).is_empty());
}
