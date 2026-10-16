//! Task M9.15.5: `anthrex lessons` against an isolated daemon (decision 20). The
//! harness pins every agent binary to `fake-agent` or a nonexistent path; no decider
//! runs here.

use std::path::PathBuf;
use std::process::Output;

use daemon::lessons::LESSONS_FILE;
use daemon::lessons::fold::title_fp;
use daemon::run::history_io::append_json;
use proto::{
    DistillCause, DistilledLine, LESSONS_VERSION, LessonDraft, LessonInfo, LessonLayer, LessonLine,
    LessonStatus, QueuedLine,
};

use crate::support::run_harness::RunHarness;

const TITLE: &str = "run cargo fmt before claiming done";

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// The repository's project (its main checkout, as git names it) and data directory.
fn project(h: &RunHarness) -> (PathBuf, PathBuf) {
    let project = h.repo.canonicalize().unwrap();
    let repo_dir = daemon::profile::repo_dir(&h.data(), &project);
    (project, repo_dir)
}

/// One proposed semantic lesson in the repository's store; its id.
fn seed(h: &RunHarness) -> String {
    let (_, repo_dir) = project(h);
    let path = repo_dir.join(LESSONS_FILE);
    let queued = "seed-0001/t1/distill".to_string();
    let lines = [
        LessonLine::Queued(QueuedLine {
            v: LESSONS_VERSION,
            record_id: queued.clone(),
            at: 1_800_000_000,
            run_id: "seed-0001".into(),
            task_id: Some("t1".into()),
            cause: DistillCause::TaskRecovered,
            dossier: "## outcome\nrecovered".into(),
            fps: Vec::new(),
        }),
        LessonLine::Distilled(DistilledLine {
            v: LESSONS_VERSION,
            record_id: format!("{queued}/distilled"),
            at: 1_800_000_000,
            queued,
            items: vec![LessonDraft {
                layer: LessonLayer::Semantic,
                title: TITLE.into(),
                body: "CI rejects unformatted code.".into(),
                files: vec!["crates/**".into()],
            }],
            dropped: 0,
            usage: None,
        }),
    ];
    for line in &lines {
        append_json(&path, line).unwrap();
    }
    format!("l-{}", title_fp(TITLE))
}

fn lessons(h: &RunHarness, args: &[&str]) -> Output {
    let repo = h.repo.display().to_string();
    let mut all = vec!["--dir", repo.as_str(), "lessons"];
    all.extend_from_slice(args);
    h.anthrex(&all)
}

fn listed(h: &RunHarness) -> Vec<LessonInfo> {
    let output = lessons(h, &["list", "--all", "--json"]);
    assert!(output.status.success(), "{}", stderr(&output));
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn lessons_list_on_an_empty_repository() {
    let h = RunHarness::new("");
    let (project, _) = project(&h);
    let output = lessons(&h, &["list"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(
        stdout(&output),
        format!("no lessons for {} yet\n", project.display())
    );
    assert_eq!(listed(&h), Vec::new());
}

#[test]
fn approve_without_yes_and_no_terminal_is_refused() {
    let h = RunHarness::new("");
    let id = seed(&h);
    let (_, repo_dir) = project(&h);
    let before = std::fs::read(repo_dir.join(LESSONS_FILE)).unwrap();
    let output = lessons(&h, &["approve", &id]);
    assert_eq!(output.status.code(), Some(1), "{}", stdout(&output));
    assert!(
        stderr(&output).contains("stdin is not a terminal; pass --yes"),
        "{}",
        stderr(&output)
    );
    assert_eq!(std::fs::read(repo_dir.join(LESSONS_FILE)).unwrap(), before);
    assert_eq!(listed(&h)[0].status, LessonStatus::Proposed);
}

#[test]
fn approve_with_yes_changes_the_status() {
    let h = RunHarness::new("");
    let id = seed(&h);
    let output = lessons(&h, &["approve", &id[..8], "--yes"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "approved 1\n");
    let after = listed(&h);
    assert_eq!(
        (after[0].id.as_str(), after[0].status),
        (id.as_str(), LessonStatus::Approved)
    );
    // A second decision is refused in the daemon's words and exits 1.
    let output = lessons(&h, &["reject", &id, "--yes"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr(&output).contains(&format!(
            "{id} is approved; only a proposed or on-trial lesson can be approved or rejected"
        )),
        "{}",
        stderr(&output)
    );
    // The list shows it under decision 20's layout.
    let text = stdout(&lessons(&h, &["list"]));
    assert!(text.contains(": 1 active, 0 to review\n"), "{text}");
    assert!(
        text.contains(&format!("{id}  approved  semantic  –  {TITLE}\n")),
        "{text}"
    );
}
