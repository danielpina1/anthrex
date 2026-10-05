//! Task M9.6.5 fix round 1: a design file is shown only as it was stored (I-1); a
//! re-sent identical write succeeds; temp files are unique (m2), the index's newest
//! write wins (m2), a file system without hard links still never loses a version
//! (m3), new folders are created one by one (m4), and a diff and findings are capped
//! (m5).

use std::io::{Error, ErrorKind};
use std::path::Path;

use proto::run_wire::request;
use proto::{DocFinding, DocKind, DocSeverity, RunReply, RunRequest};

use super::tests::{SPEC_1, SPEC_2, apply, design_run, names, service, spec, store, tmp};
use super::{DOC_READ_CAP, DocWrites, FINDINGS_CAP, make_dirs, temp_name, write_new, write_new_at};
use crate::run::design::state;
use crate::run::test_support::RUN_ID;

fn show(version: Option<u32>, diff: bool, findings: bool) -> RunRequest {
    RunRequest::ShowDoc {
        run: RUN_ID.into(),
        kind: DocKind::Spec,
        version,
        diff,
        findings,
    }
}

fn mismatch(n: u32) -> RunReply {
    let text = format!("document spec v{n} does not match what was stored; it was not shown");
    RunReply::refused(request::SHOW_DOC, text)
}

#[tokio::test]
async fn a_stored_file_that_changed_is_not_shown() {
    let dir = tmp();
    let s = service(dir.path(), design_run(dir.path()));
    let (_, effect) = store(&s, spec(SPEC_1));
    let v1 = apply(&s, effect).await;
    let (_, effect) = store(&s, spec(SPEC_2));
    let v2 = apply(&s, effect).await;

    // The same length, other bytes: refused, never shown.
    std::fs::write(&v2, SPEC_2.replace("expires", "explode")).unwrap();
    assert_eq!(s.request(show(Some(2), false, false)).await, mismatch(2));
    // Another length.
    std::fs::write(&v2, format!("{SPEC_2}more")).unwrap();
    assert_eq!(s.request(show(None, false, false)).await, mismatch(2));
    std::fs::write(&v2, SPEC_2).unwrap();
    assert!(matches!(
        s.request(show(Some(2), true, false)).await,
        RunReply::Doc { .. }
    ));

    // A changed previous version is not diffed against either.
    std::fs::write(&v1, "R9 something else").unwrap();
    assert_eq!(s.request(show(Some(2), true, false)).await, mismatch(1));
    assert!(matches!(
        s.request(show(Some(2), false, false)).await,
        RunReply::Doc { .. }
    ));

    // A missing file names the error.
    std::fs::remove_file(&v2).unwrap();
    let RunReply::Refused { message, .. } = s.request(show(Some(2), false, false)).await else {
        panic!("a missing file is refused");
    };
    assert!(
        message.starts_with("could not read the spec v2: "),
        "{message}"
    );
}

#[test]
fn an_identical_rewrite_is_accepted_and_another_text_refused() {
    let dir = tmp();
    let path = dir.path().join("design/spec-v1.md");
    write_new(&path, SPEC_1).unwrap();
    write_new(&path, SPEC_1).expect("a re-sent write is idempotent");
    let refused = write_new(&path, SPEC_2).unwrap_err();
    assert!(
        refused.ends_with("exists; a design document is never rewritten"),
        "{refused}"
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), SPEC_1);
    assert_eq!(names(&dir.path().join("design")), ["spec-v1.md"]);
}

#[test]
fn temp_files_are_unique_and_a_stale_one_is_removed() {
    let dir = tmp();
    let path = dir.path().join("spec-v1.md");
    let a = temp_name(&path, 7);
    assert_eq!(
        a.file_name().unwrap().to_string_lossy(),
        format!("spec-v1.md.{}.7.tmp", std::process::id())
    );
    assert_ne!(a, temp_name(&path, 8));
    std::fs::write(&a, "a crashed write's leftover").unwrap();
    write_new_at(&path, SPEC_1, 7, &|from, to| std::fs::hard_link(from, to)).unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), SPEC_1);
    assert_eq!(names(dir.path()), ["spec-v1.md"]);
}

#[test]
fn an_older_index_never_replaces_a_newer() {
    let dir = tmp();
    let index = dir.path().join("design/versions.json");
    let writes = DocWrites::default();
    let older = writes.index_writer(index.clone(), "[1]".into());
    let newer = writes.index_writer(index.clone(), "[1, 2]".into());
    newer().unwrap();
    older().unwrap();
    assert_eq!(std::fs::read_to_string(&index).unwrap(), "[1, 2]");
    writes.index_writer(index.clone(), "[1, 2, 3]".into())().unwrap();
    assert_eq!(std::fs::read_to_string(&index).unwrap(), "[1, 2, 3]");
    assert_eq!(names(&dir.path().join("design")), ["versions.json"]);
}

#[test]
fn an_unsupported_hard_link_falls_back_to_a_new_file() {
    let dir = tmp();
    let path = dir.path().join("spec-v1.md");
    let no_links = |_: &Path, _: &Path| Err(Error::from(ErrorKind::Unsupported));
    write_new_at(&path, SPEC_1, 1, &no_links).expect("written in place");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), SPEC_1);
    assert_eq!(names(dir.path()), ["spec-v1.md"], "no temp file left");
    // It still never replaces a file.
    write_new_at(&path, SPEC_1, 2, &no_links).expect("the same text");
    let refused = write_new_at(&path, SPEC_2, 3, &no_links).unwrap_err();
    assert!(
        refused.ends_with("exists; a design document is never rewritten"),
        "{refused}"
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), SPEC_1);
}

/// The final fix wave's FW-31: a link refused with EPERM (FAT, some FUSE mounts), or
/// with ENOTSUP or EOPNOTSUPP (macOS exFAT, FAT and FUSE), takes the in-place fallback
/// too; another error does not.
#[test]
fn a_link_refused_as_unsupported_falls_back_to_a_new_file() {
    for (name, errno) in [
        ("EPERM", libc::EPERM),
        ("ENOTSUP", libc::ENOTSUP),
        ("EOPNOTSUPP", libc::EOPNOTSUPP),
    ] {
        let dir = tmp();
        let path = dir.path().join("spec-v1.md");
        let refused = |_: &Path, _: &Path| Err(Error::from_raw_os_error(errno));
        write_new_at(&path, SPEC_1, 1, &refused).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), SPEC_1, "{name}");
        assert_eq!(
            names(dir.path()),
            ["spec-v1.md"],
            "{name}: no temp file left"
        );
    }
    let dir = tmp();
    let path = dir.path().join("spec-v1.md");
    let eio = |_: &Path, _: &Path| Err(Error::from_raw_os_error(libc::EIO));
    assert!(write_new_at(&path, SPEC_1, 1, &eio).is_err());
    assert!(!path.exists(), "no fallback for another error");
}

#[test]
fn missing_folders_are_created_one_by_one_and_listed() {
    let dir = tmp();
    let deep = dir.path().join("runs/r/design/brainstorm");
    let made = make_dirs(&deep).unwrap();
    assert_eq!(
        made,
        [
            dir.path().join("runs"),
            dir.path().join("runs/r"),
            dir.path().join("runs/r/design"),
            deep.clone(),
        ]
    );
    assert!(deep.is_dir());
    assert!(make_dirs(&deep).unwrap().is_empty(), "nothing new to make");
}

#[tokio::test]
async fn a_long_diff_and_many_findings_are_capped() {
    let dir = tmp();
    let s = service(dir.path(), design_run(dir.path()));
    // Each under line_diff's 64 KiB, every line different: the diff is twice as long.
    let text = |tag: &str| {
        (0..2_000)
            .map(|i| format!("{tag} line {i:05} ......\n"))
            .collect()
    };
    let (old, new): (String, String) = (text("old"), text("new"));
    assert!(old.len() < DOC_READ_CAP && new.len() < DOC_READ_CAP);
    let (_, effect) = store(&s, spec(&old));
    apply(&s, effect).await;
    let (_, effect) = store(&s, spec(&new));
    apply(&s, effect).await;
    let finding = |i: usize| DocFinding {
        id: format!("F{i}"),
        severity: DocSeverity::Minor,
        place: "## Design".into(),
        text: "x".repeat(1_000),
    };
    let findings: Vec<_> = (1..=40)
        .map(|i| (finding(i), Some("fixed".into())))
        .collect();
    let effect = {
        let engine = crate::lock(&s.state);
        state::store_findings(&engine.runs[RUN_ID], DocKind::Spec, 2, &findings).unwrap()
    };
    apply(&s, effect).await;

    let RunReply::Doc { doc, .. } = s.request(show(Some(2), true, true)).await else {
        panic!("shown");
    };
    let diff = doc.diff.expect("a diff");
    assert!(diff.len() <= DOC_READ_CAP, "{}", diff.len());
    let (head, marker) = diff.rsplit_once('\n').unwrap();
    let full = crate::run::design::changes::line_diff(&old, &new);
    assert!(full.starts_with(head));
    assert_eq!(
        marker,
        format!("[diff cut: {} bytes]", full.len() - head.len())
    );

    let json = serde_json::to_string(&doc.findings).unwrap();
    assert!(json.len() <= FINDINGS_CAP, "{} bytes", json.len());
    let shown = doc.findings.len() - 1;
    assert!(shown > 0 && shown < 40, "{shown}");
    assert_eq!(doc.findings[..shown], findings[..shown]);
    let (last, answer) = doc.findings.last().unwrap();
    assert_eq!(last.text, format!("[findings cut: {} more]", 40 - shown));
    assert_eq!(answer, &None);
}
