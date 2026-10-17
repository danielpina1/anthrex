//! Task M9.18.3: decision 3's compaction and decision 4's thresholds, on real files in
//! temporary directories. A store has to be over [`LESSONS_COMPACT_BYTES`] to be
//! compacted at all, so every fixture that expects a rewrite carries one `queued` line
//! whose dossier is that long.

use std::collections::BTreeSet;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::time::{Duration, SystemTime};

use proto::{
    AttemptLine, DismissedLine, DistillCause, DistilledLine, GaveUpLine, LESSONS_VERSION,
    LessonDraft, LessonLayer, LessonLine, QueuedLine, RetiredLine, VerdictLine,
};

use super::super::LESSONS_FILE;
use super::super::fold::{self, Store};
use super::super::service::{DAY_SECS, capped};
use super::{Compacted, LESSONS_COMPACT_BYTES, compact};

/// The compaction's clock.
pub(super) const NOW: u64 = 1_800_000_000;
/// Two days before [`NOW`]: an attempt this old is outside the daily cap's window.
const OLD: u64 = NOW - 2 * DAY_SECS;

fn queued(id: &str, dossier_len: usize) -> LessonLine {
    LessonLine::Queued(QueuedLine {
        v: LESSONS_VERSION,
        record_id: format!("{id}/distill"),
        at: OLD - 100,
        run_id: id.into(),
        task_id: None,
        cause: DistillCause::RunFailed,
        dossier: "d".repeat(dossier_len),
        fps: vec![],
    })
}

/// A dossier long enough that its one line puts the store over the threshold.
fn big() -> usize {
    LESSONS_COMPACT_BYTES as usize + 1024
}

fn attempt(id: &str, n: u8, at: u64) -> LessonLine {
    LessonLine::Attempt(AttemptLine {
        v: LESSONS_VERSION,
        record_id: format!("{id}/distill/attempt/{n}"),
        at,
        queued: format!("{id}/distill"),
        n,
        error: Some("no answer".into()),
        usage: None,
    })
}

fn draft(title: &str) -> LessonDraft {
    LessonDraft {
        layer: LessonLayer::Episodic,
        title: title.into(),
        body: format!("{title}, in a body."),
        files: vec![],
    }
}

fn distilled(id: &str, at: u64, titles: &[&str]) -> LessonLine {
    LessonLine::Distilled(DistilledLine {
        v: LESSONS_VERSION,
        record_id: format!("{id}/distill/distilled"),
        at,
        queued: format!("{id}/distill"),
        items: titles.iter().map(|t| draft(t)).collect(),
        dropped: 0,
        usage: None,
    })
}

fn gave_up(id: &str, at: u64) -> LessonLine {
    LessonLine::GaveUp(GaveUpLine {
        v: LESSONS_VERSION,
        record_id: format!("{id}/distill/gave_up"),
        at,
        queued: format!("{id}/distill"),
        reason: "no answer".into(),
    })
}

fn lesson_id(title: &str) -> String {
    format!("l-{}", fold::title_fp(title))
}

fn verdict(record_id: &str, id: &str, approved: bool) -> LessonLine {
    LessonLine::Verdict(VerdictLine {
        v: LESSONS_VERSION,
        record_id: record_id.into(),
        at: OLD + 500,
        id: id.into(),
        approved,
    })
}

pub(super) fn json(line: &LessonLine) -> String {
    serde_json::to_string(line).unwrap()
}

/// `raw` written one per line, each with its newline.
pub(super) fn write_lines(path: &Path, raw: &[String]) {
    let text: String = raw.iter().map(|l| format!("{l}\n")).collect();
    std::fs::write(path, text).unwrap();
}

fn raw_lines(path: &Path) -> Vec<String> {
    let text = std::fs::read_to_string(path).unwrap();
    text.lines().map(str::to_string).collect()
}

/// A closed entry over the threshold: its `queued` line (kept), three old attempts
/// (dropped) and its `gave_up` (kept).
pub(super) fn padding() -> Vec<String> {
    vec![
        json(&queued("pad", big())),
        json(&attempt("pad", 1, OLD)),
        json(&attempt("pad", 2, OLD + 1)),
        json(&attempt("pad", 3, OLD + 2)),
        json(&gave_up("pad", OLD + 3)),
    ]
}

/// Each closed entry's `attempts` set to 0: decision 3's acceptance compares the folds
/// so.
fn zeroed(mut store: Store) -> Store {
    for entry in store.queued.values_mut() {
        if !entry.open {
            entry.attempts = 0;
        }
    }
    store
}

#[test]
fn compaction_keeps_the_fold() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(LESSONS_FILE);
    let (a, b) = ("The fixture must exist", "Run the formatter first");
    let before: Vec<LessonLine> = vec![
        queued("open", big()),
        attempt("open", 1, OLD),
        queued("dist", 10),
        attempt("dist", 1, OLD),
        attempt("dist", 2, OLD + 1),
        attempt("dist", 3, OLD + 2),
        distilled("dist", OLD + 10, &[a, b]),
        queued("gave", 10),
        attempt("gave", 1, OLD),
        attempt("gave", 2, OLD + 1),
        attempt("gave", 3, OLD + 2),
        gave_up("gave", OLD + 3),
        attempt("open", 2, OLD + 5),
        // Ruling C1: a closed entry's attempt inside the day stays for the cap.
        queued("recent", 10),
        attempt("recent", 1, NOW - 60),
        gave_up("recent", NOW - 30),
        verdict(&format!("{}/verdict/1", lesson_id(a)), &lesson_id(a), true),
        verdict(&format!("{}/verdict/2", lesson_id(b)), &lesson_id(b), false),
        LessonLine::Retired(RetiredLine {
            v: LESSONS_VERSION,
            record_id: format!("{}/retired", lesson_id(a)),
            at: OLD + 600,
            id: lesson_id(a),
            reason: "forgotten by the user".into(),
        }),
        LessonLine::Dismissed(DismissedLine {
            v: LESSONS_VERSION,
            record_id: "kf-0123456789abcdef/dismissed".into(),
            at: OLD + 700,
            fp: "0123456789abcdef".into(),
        }),
    ];
    let raw: Vec<String> = before.iter().map(json).collect();
    write_lines(&path, &raw);
    let text_before = std::fs::read_to_string(&path).unwrap();

    assert_eq!(
        compact(&path, NOW).unwrap(),
        Compacted::Rewrote {
            before: 20,
            after: 14
        }
    );

    let text_after = std::fs::read_to_string(&path).unwrap();
    let fold_before = fold::fold_text(&text_before, "before");
    let fold_after = fold::fold_text(&text_after, "after");
    assert!(fold_after.problems.is_empty(), "{:?}", fold_after.problems);
    assert_eq!(fold_after.queued["open/distill"].attempts, 2);
    assert_eq!(fold_after.lessons.len(), 2);
    assert_eq!(zeroed(fold_before), zeroed(fold_after));

    let gone: BTreeSet<String> = ["dist", "gave"]
        .iter()
        .flat_map(|id| (1..=3).map(move |n| format!("{id}/distill/attempt/{n}")))
        .collect();
    let expected: Vec<String> = (before.iter())
        .filter(|l| !gone.contains(l.record_id()))
        .map(json)
        .collect();
    assert_eq!(
        raw_lines(&path),
        expected,
        "exactly the six old closed attempts go"
    );

    // Ruling C1: the daily cap counts the same attempts before and after.
    let (lines_before, _) = fold::parse(&text_before, "");
    let (lines_after, _) = fold::parse(&text_after, "");
    for max in 1..=4 {
        assert_eq!(
            capped(&lines_before, NOW, max),
            capped(&lines_after, NOW, max),
            "max {max}"
        );
    }
    assert!(capped(&lines_after, NOW, 1).is_some());
}

#[test]
fn a_repeated_record_keeps_its_last_line() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(LESSONS_FILE);
    let id = lesson_id("Some lesson");
    let record = format!("{id}/verdict/1");
    let first = json(&verdict(&record, &id, true));
    let between = json(&verdict(&format!("{id}/verdict/2"), &id, true));
    let last = json(&verdict(&record, &id, false));
    assert_ne!(first, last);
    let mut raw = padding();
    raw.extend([first, between.clone(), last.clone()]);
    write_lines(&path, &raw);

    assert_eq!(
        compact(&path, NOW).unwrap(),
        Compacted::Rewrote {
            before: 8,
            after: 4
        }
    );
    assert_eq!(
        raw_lines(&path),
        [&raw[0], &raw[4], &between, &last].map(|s| s.clone())
    );
}

#[test]
fn a_line_that_does_not_parse_is_kept_verbatim() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(LESSONS_FILE);
    let pad = padding();
    let unknown = r#"{"type":"future_kind","v":1,"record_id":"pad/distill/attempt/2","at":5}"#;
    let higher = r#"{"type":"verdict","v":2,"record_id":"l-y/verdict/5","at":5,"id":"l-y","approved":true,"extra":[1, 2]}"#;
    let torn = r#"{"type":"verdict","v":1,"rec"#;
    let text = format!(
        "{}\n{}\n{unknown}\n{}\n{}\n{}\n{higher}\n{torn}",
        pad[0], pad[1], pad[2], pad[3], pad[4]
    );
    std::fs::write(&path, &text).unwrap();

    assert_eq!(
        compact(&path, NOW).unwrap(),
        Compacted::Rewrote {
            before: 8,
            after: 5
        }
    );
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        format!("{}\n{unknown}\n{}\n{higher}\n{torn}", pad[0], pad[4])
    );
}

#[test]
fn a_small_store_is_skipped() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(LESSONS_FILE);
    assert_eq!(compact(&path, NOW).unwrap(), Compacted::Skipped, "no file");

    // Under the threshold, though most of it would go.
    let mut small: Vec<String> = padding();
    small[0] = json(&queued("pad", 10));
    write_lines(&path, &small);
    let past = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000_000);
    std::fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(past)
        .unwrap();
    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(compact(&path, NOW).unwrap(), Compacted::Skipped);
    assert_eq!(std::fs::metadata(&path).unwrap().modified().unwrap(), past);
    assert_eq!(std::fs::read(&path).unwrap(), bytes);

    // Over it, but one line in five would go: less than a quarter.
    let id = lesson_id("Some lesson");
    let over = vec![
        json(&queued("x", big())),
        json(&attempt("x", 1, OLD)),
        json(&gave_up("x", OLD + 1)),
        json(&verdict(&format!("{id}/verdict/1"), &id, true)),
        json(&verdict(&format!("{id}/verdict/2"), &id, false)),
    ];
    write_lines(&path, &over);
    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(compact(&path, NOW).unwrap(), Compacted::Skipped);
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
}

#[test]
fn a_failed_rewrite_leaves_the_file() {
    // SAFETY: `geteuid` has no preconditions and cannot fail.
    if unsafe { libc::geteuid() } == 0 {
        eprintln!("skipped: root writes into a read-only directory");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    let path = repo.join(LESSONS_FILE);
    write_lines(&path, &padding());
    let bytes = std::fs::read(&path).unwrap();
    // The read needs no write permission on the directory; the temporary file does.
    std::fs::set_permissions(&repo, std::fs::Permissions::from_mode(0o555)).unwrap();
    let result = compact(&path, NOW);
    std::fs::set_permissions(&repo, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(result.is_err(), "{result:?}");
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    let names: Vec<_> = (std::fs::read_dir(&repo).unwrap().flatten())
        .map(|e| e.file_name())
        .collect();
    assert_eq!(names, [LESSONS_FILE], "no temporary file is left");
}

/// Task M9.18.3's review, M1: [`compact`] parses a line as the fold does, lossily. A
/// later line with invalid UTF-8 in a string that repeats a live `distilled` line's
/// `record_id` reopens its entry for the fold, so its old attempts must stay.
#[test]
fn a_line_with_invalid_utf8_parses_as_the_fold_parses_it() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(LESSONS_FILE);
    let mut bytes: Vec<u8> = Vec::new();
    let mut raw = padding();
    raw.extend([
        json(&queued("x", 10)),
        json(&attempt("x", 1, OLD)),
        json(&attempt("x", 2, OLD + 1)),
        json(&attempt("x", 3, OLD + 2)),
        json(&distilled("x", OLD + 10, &["Some lesson"])),
    ]);
    for line in &raw {
        bytes.extend_from_slice(line.as_bytes());
        bytes.push(b'\n');
    }
    bytes.extend_from_slice(
        br#"{"type":"verdict","v":1,"record_id":"x/distill/distilled","at":5,"id":"l-"#,
    );
    bytes.push(0xFF);
    bytes.extend_from_slice(b"\",\"approved\":true}\n");
    std::fs::write(&path, &bytes).unwrap();
    let fold_before = fold::fold_text(&String::from_utf8_lossy(&bytes), "s");
    assert!(
        fold_before.problems.is_empty(),
        "{:?}",
        fold_before.problems
    );
    assert!(fold_before.queued["x/distill"].open);
    assert_eq!(fold_before.queued["x/distill"].attempts, 3);

    // The padding's three attempts go, and the repeated `distilled` line; x's stay.
    assert_eq!(
        compact(&path, NOW).unwrap(),
        Compacted::Rewrote {
            before: 11,
            after: 7
        }
    );
    let after = std::fs::read(&path).unwrap();
    let fold_after = fold::fold_text(&String::from_utf8_lossy(&after), "s");
    assert_eq!(fold_after.queued["x/distill"].attempts, 3);
    assert_eq!(zeroed(fold_before), zeroed(fold_after));
}

/// Task M9.18.3's review, M2: the rewrite is installed when only the directory's
/// `fsync` fails (a directory that can be written and searched but not read cannot be
/// opened for it), so it is reported as a rewrite, not as a store left as it was.
#[test]
fn a_failed_directory_sync_is_still_a_rewrite() {
    // SAFETY: `geteuid` has no preconditions and cannot fail.
    if unsafe { libc::geteuid() } == 0 {
        eprintln!("skipped: root opens an unreadable directory");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    let path = repo.join(LESSONS_FILE);
    let raw = padding();
    write_lines(&path, &raw);
    std::fs::set_permissions(&repo, std::fs::Permissions::from_mode(0o300)).unwrap();
    let result = compact(&path, NOW);
    std::fs::set_permissions(&repo, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(
        result.unwrap(),
        Compacted::Rewrote {
            before: 5,
            after: 2
        }
    );
    assert_eq!(raw_lines(&path), [raw[0].clone(), raw[4].clone()]);
}

/// Task M9.18.3's review, M3: decision 4's size bound is inclusive. A store of exactly
/// [`LESSONS_COMPACT_BYTES`] is compacted; one byte shorter is not.
#[test]
fn a_store_of_exactly_the_threshold_is_compacted() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(LESSONS_FILE);
    // A closed entry, one of its three lines an old attempt; the dossier's `d`s are one
    // byte each, so the file's length is the empty dossier's plus `len`.
    let sized = |len: usize| {
        vec![
            json(&queued("pad", len)),
            json(&attempt("pad", 1, OLD)),
            json(&gave_up("pad", OLD + 1)),
        ]
    };
    let empty: usize = sized(0).iter().map(|l| l.len() + 1).sum();
    let len = LESSONS_COMPACT_BYTES as usize - empty;

    write_lines(&path, &sized(len - 1));
    assert_eq!(
        std::fs::metadata(&path).unwrap().len(),
        LESSONS_COMPACT_BYTES - 1
    );
    assert_eq!(compact(&path, NOW).unwrap(), Compacted::Skipped);

    write_lines(&path, &sized(len));
    assert_eq!(
        std::fs::metadata(&path).unwrap().len(),
        LESSONS_COMPACT_BYTES
    );
    assert_eq!(
        compact(&path, NOW).unwrap(),
        Compacted::Rewrote {
            before: 3,
            after: 2
        }
    );
}

/// Task M9.18.3's review, M3: decision 4's "at least a quarter" is inclusive. One line
/// of four going is a rewrite.
#[test]
fn exactly_a_quarter_dropped_is_compacted() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(LESSONS_FILE);
    let id = lesson_id("Some lesson");
    let raw = vec![
        json(&queued("x", big())),
        json(&attempt("x", 1, OLD)),
        json(&gave_up("x", OLD + 1)),
        json(&verdict(&format!("{id}/verdict/1"), &id, true)),
    ];
    write_lines(&path, &raw);
    assert_eq!(
        compact(&path, NOW).unwrap(),
        Compacted::Rewrote {
            before: 4,
            after: 3
        }
    );
    assert_eq!(
        raw_lines(&path),
        [&raw[0], &raw[2], &raw[3]].map(|s| s.clone())
    );
}
