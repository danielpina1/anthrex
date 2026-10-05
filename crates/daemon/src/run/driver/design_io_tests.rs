//! Task M9.6.5: design documents on a real temp dir. The engine half (`state::store`)
//! numbers a version and emits its write; this file writes it once and never again,
//! and `run show` (`RunRequest::ShowDoc`) reads it back off the engine.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use proto::run_wire::request;
use proto::{DocAuthor, DocFinding, DocKind, DocSeverity, DocView, RunReply, RunRequest};
use sha2::{Digest, Sha256};

use super::super::RunService;
use super::{DOC_READ_CAP, cut_text, write_new};
use crate::manager::{GitRoots, ManagerConfig, WindowManager};
use crate::run::design::changes::line_diff;
use crate::run::design::state::{self, DesignState, DocVersion, NewDoc};
use crate::run::engine::Effect;
use crate::run::model::Run;
use crate::run::test_support::{PROFILE, RUN_ID, plan_with, run_ok, task_toml};

struct NoRoots;
impl GitRoots for NoRoots {
    fn register(&self, _: PathBuf) {}
    fn unregister(&self, _: &Path) {}
}

pub(super) const SPEC_1: &str = "# Reset\n\n## Requirements\nR1 A user can ask for a reset link.\n";
pub(super) const SPEC_2: &str =
    "# Reset\n\n## Requirements\nR1 A user can ask for a reset link.\nR2 The link expires.\n";

/// A design run (`run.orch.design` set) whose data directory is under `data`.
pub(super) fn design_run(data: &Path) -> Run {
    let mut run = run_ok(&plan_with(PROFILE, &[task_toml("t1", "S", "[\"a\"]", "")]));
    run.data_dir = data.join("runs").join(RUN_ID);
    run.design_mode = proto::DesignMode::Full;
    run.orch.design = Some(DesignState::default());
    run
}

pub(super) fn service(data: &Path, run: Run) -> Arc<RunService> {
    let config = ManagerConfig::for_tests("/tmp/ax-unused.sock".into(), "/bin/sh".into());
    let (manager, _events) = WindowManager::new(config);
    let s = RunService::for_manager(&manager, data.to_path_buf(), Arc::new(NoRoots));
    crate::lock(&s.state).runs.insert(run.id.clone(), run);
    s
}

/// `state::store` on the service's run, under the engine lock, as the engine does.
pub(super) fn store(s: &RunService, doc: NewDoc) -> (DocVersion, Effect) {
    let mut engine = crate::lock(&s.state);
    let run = engine.runs.get_mut(RUN_ID).expect("the run");
    state::store(run, doc, 2_000).expect("stored")
}

pub(super) fn spec(text: &str) -> NewDoc {
    NewDoc::new(DocKind::Spec, DocAuthor::Orchestrator, "ready", text)
}

/// Applies a `WriteDoc` the way `driver/effects.rs::apply` does.
pub(super) async fn apply(s: &RunService, effect: Effect) -> PathBuf {
    let Effect::WriteDoc {
        path,
        text,
        index,
        doc,
    } = effect
    else {
        panic!("not a write: {effect:?}");
    };
    s.write_doc(path.clone(), text, index, doc).await;
    path
}

pub(super) fn names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

pub(super) fn tmp() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("ax-design-io-")
        .tempdir_in("/tmp")
        .expect("temp dir")
}

#[tokio::test]
async fn a_written_version_is_atomic_and_listed() {
    let dir = tmp();
    let s = service(dir.path(), design_run(dir.path()));
    let (version, effect) = store(&s, spec(SPEC_1));
    assert_eq!(version.n, 1);
    let design_dir = dir.path().join("runs").join(RUN_ID).join("design");
    let path = apply(&s, effect).await;
    assert_eq!(path, design_dir.join("spec-v1.md"));

    // The whole text, and no temp file left beside it.
    assert_eq!(std::fs::read_to_string(&path).unwrap(), SPEC_1);
    assert_eq!(names(&design_dir), ["spec-v1.md", "versions.json"]);

    // `versions.json` mirrors the run's index: the entry's size, hash and requirements.
    let listed: Vec<DocVersion> =
        serde_json::from_str(&std::fs::read_to_string(design_dir.join("versions.json")).unwrap())
            .unwrap();
    let design = crate::lock(&s.state).runs[RUN_ID]
        .orch
        .design
        .clone()
        .unwrap();
    assert_eq!(listed, design.versions);
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].bytes, SPEC_1.len() as u64);
    let sha: String = (Sha256::digest(SPEC_1.as_bytes()).iter())
        .map(|b| format!("{b:02x}"))
        .collect();
    assert_eq!(listed[0].sha256, sha);
    assert_eq!(listed[0].requirements, ["R1"]);
    assert_eq!((listed[0].at, listed[0].reason.as_str()), (2_000, "ready"));

    // A second version is added to the index; the first file is untouched.
    let (_, effect) = store(&s, spec(SPEC_2));
    apply(&s, effect).await;
    let listed: Vec<DocVersion> =
        serde_json::from_str(&std::fs::read_to_string(design_dir.join("versions.json")).unwrap())
            .unwrap();
    assert_eq!(listed.iter().map(|v| v.n).collect::<Vec<_>>(), [1, 2]);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), SPEC_1);
    assert_eq!(
        names(&design_dir),
        ["spec-v1.md", "spec-v2.md", "versions.json"]
    );
}

#[tokio::test]
async fn a_version_is_never_rewritten() {
    let dir = tmp();
    let s = service(dir.path(), design_run(dir.path()));
    let (v1, effect) = store(&s, spec(SPEC_1));
    let path = apply(&s, effect).await;

    // The driver refuses to write an existing (kind, n): the file keeps its text.
    let refused = write_new(&path, SPEC_2).unwrap_err();
    assert!(
        refused.ends_with("spec-v1.md exists; a design document is never rewritten"),
        "{refused}"
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), SPEC_1);
    // A `WriteDoc` aimed at it changes nothing either.
    s.write_doc(path.clone(), SPEC_2.into(), None, None).await;
    assert_eq!(std::fs::read_to_string(&path).unwrap(), SPEC_1);

    // The engine always takes n+1, the same text included, and per kind.
    let (v2, _) = store(&s, spec(SPEC_1));
    let (v3, effect) = store(&s, spec(SPEC_2));
    assert_eq!((v1.n, v2.n, v3.n), (1, 2, 3));
    let Effect::WriteDoc { path: p3, .. } = effect else {
        panic!()
    };
    assert!(p3.ends_with("design/spec-v3.md"), "{p3:?}");
    let brainstorm = NewDoc::new(DocKind::Brainstorm, DocAuthor::Orchestrator, "merged", "b");
    assert_eq!(store(&s, brainstorm).0.n, 1);

    // A rethink's second draft from the same brainstormer gets a file of its own.
    let draft = |label: &str| {
        let author = DocAuthor::Brainstormer {
            label: label.into(),
        };
        NewDoc::new(DocKind::BrainstormDraft, author, "draft", "d")
    };
    let files: Vec<PathBuf> = ["claude", "codex", "claude"]
        .iter()
        .map(|label| match store(&s, draft(label)).1 {
            Effect::WriteDoc { path, .. } => path,
            other => panic!("{other:?}"),
        })
        .collect();
    let design = dir.path().join("runs").join(RUN_ID).join("design");
    assert_eq!(
        files,
        [
            design.join("brainstorm/draft-claude.md"),
            design.join("brainstorm/draft-codex.md"),
            design.join("brainstorm/draft-claude-v3.md"),
        ]
    );
}

#[tokio::test]
async fn show_doc_returns_the_text_the_diff_and_the_findings() {
    let dir = tmp();
    let s = service(dir.path(), design_run(dir.path()));
    let (_, effect) = store(&s, spec(SPEC_1));
    apply(&s, effect).await;
    let (_, effect) = store(&s, spec(SPEC_2));
    apply(&s, effect).await;
    let finding = DocFinding {
        id: "F1".into(),
        severity: DocSeverity::Blocking,
        place: "R2".into(),
        text: "How long until it expires?".into(),
    };
    let minor = DocFinding {
        id: "F2".into(),
        severity: DocSeverity::Minor,
        place: "## Risks".into(),
        text: "Name a risk.".into(),
    };
    let findings = vec![
        (finding, Some("fixed".to_string())),
        (minor, Some("kept: none apply".to_string())),
    ];
    let effect = {
        let engine = crate::lock(&s.state);
        state::store_findings(&engine.runs[RUN_ID], DocKind::Spec, 2, &findings).unwrap()
    };
    apply(&s, effect).await;

    let show = |version, diff, findings| RunRequest::ShowDoc {
        run: RUN_ID.into(),
        kind: DocKind::Spec,
        version,
        diff,
        findings,
    };
    let doc = |reply: RunReply| match reply {
        RunReply::Doc { doc, .. } => *doc,
        other => panic!("{other:?}"),
    };
    let v2 = doc(s.request(show(Some(2), true, true)).await);
    assert_eq!(
        v2,
        DocView {
            run: RUN_ID.into(),
            kind: DocKind::Spec,
            version: 2,
            text: SPEC_2.into(),
            diff: Some(line_diff(SPEC_1, SPEC_2)),
            findings: findings.clone(),
            draft_review: None,
        }
    );
    assert!(v2.diff.as_ref().unwrap().contains("+R2 The link expires."));

    // The latest when no version is named; nothing asked for, nothing given.
    let latest = doc(s.request(show(None, false, false)).await);
    assert_eq!(
        (latest.version, latest.diff, latest.findings),
        (2, None, vec![])
    );
    assert_eq!(latest.text, SPEC_2);

    // v1 has nothing before it to diff against, and no stored findings.
    let v1 = doc(s.request(show(Some(1), true, true)).await);
    assert_eq!(
        (v1.text.as_str(), v1.diff, v1.findings),
        (SPEC_1, None, vec![])
    );
}

#[tokio::test]
async fn show_doc_of_a_missing_version_is_refused() {
    let dir = tmp();
    let s = service(dir.path(), design_run(dir.path()));
    let (_, effect) = store(&s, spec(SPEC_1));
    apply(&s, effect).await;
    let show = |run: &str, kind, version| RunRequest::ShowDoc {
        run: run.into(),
        kind,
        version,
        diff: true,
        findings: true,
    };
    let refused = |message: String| RunReply::refused(request::SHOW_DOC, message);
    assert_eq!(
        s.request(show(RUN_ID, DocKind::Spec, Some(3))).await,
        refused(format!("run {RUN_ID} has no spec v3"))
    );
    assert_eq!(
        s.request(show(RUN_ID, DocKind::Plan, None)).await,
        refused(format!("run {RUN_ID} has no plan yet"))
    );
    assert_eq!(
        s.request(show(RUN_ID, DocKind::BrainstormDraft, Some(1)))
            .await,
        refused(format!("run {RUN_ID} has no brainstorm draft v1"))
    );
    assert_eq!(
        s.request(show("nope", DocKind::Spec, None)).await,
        refused("unknown run nope".into())
    );

    // A listed version whose file is gone names the error.
    let file = dir
        .path()
        .join("runs")
        .join(RUN_ID)
        .join("design/spec-v1.md");
    std::fs::remove_file(&file).unwrap();
    let RunReply::Refused { message, .. } = s.request(show(RUN_ID, DocKind::Spec, None)).await
    else {
        panic!("a missing file is refused");
    };
    assert!(
        message.starts_with("could not read the spec v1: "),
        "{message}"
    );

    // A run without the design flow has no documents.
    crate::lock(&s.state)
        .runs
        .get_mut(RUN_ID)
        .unwrap()
        .orch
        .design = None;
    assert_eq!(
        s.request(show(RUN_ID, DocKind::Spec, None)).await,
        refused(format!("run {RUN_ID} does not use the design flow"))
    );
}

#[test]
fn a_long_document_is_capped_at_64_kib() {
    // Fix round 1, m1: three bytes a character, after 0, 1 and 2 ASCII bytes, so for one
    // of them the cut splits a character, and the replacement character that leaves is
    // dropped.
    for lead in ["", "x", "xy"] {
        let text = format!("{lead}{}", "€".repeat(DOC_READ_CAP));
        let read = cut_text(text.as_bytes(), DOC_READ_CAP, "cut");
        assert!(read.len() <= DOC_READ_CAP, "{}", read.len());
        let (head, marker) = read.rsplit_once('\n').unwrap();
        assert!(
            text.starts_with(head),
            "{lead:?}: ends {:?}",
            head.chars().last()
        );
        assert_eq!(marker, format!("[cut: {} bytes]", text.len() - head.len()));
    }
    assert_eq!(cut_text(b"short", DOC_READ_CAP, "cut"), "short");
}
