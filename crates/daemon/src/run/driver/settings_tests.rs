//! Milestone 9.0.6 decisions 28, 29 and 37 on a real `RunService` (built as
//! `driver/tests.rs` builds one). The config path is always under a `tempfile` directory,
//! handed over with `with_settings`; nothing here reads `ANTHREX_CONFIG`.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use proto::{Origin, RunReply, RunState, SettingsDoc, SettingsReply, SettingsRequest};

use super::{WRITE_TIMEOUT_TEXT, WRITE_UNKNOWN_TEXT};
use crate::live_config::{LiveSettings, SETTINGS_WRITE_TIMEOUT, SaveFn, SettingsIo};
use crate::manager::{GitRoots, ManagerConfig, WindowManager};
use crate::run::driver::{RunContext, RunService};

struct NoRoots;
impl GitRoots for NoRoots {
    fn register(&self, _: PathBuf) {}
    fn unregister(&self, _: &Path) {}
}

/// The injected timeout of the timeout tests (decision 28's `SETTINGS_WRITE_TIMEOUT`,
/// shortened; the save stand-in blocks until released, so it always fires).
const SHORT: Duration = Duration::from_millis(50);

/// How long a released save may take to land: the real `config::settings::save` on a
/// tempdir, which the daemon budgets `SETTINGS_WRITE_TIMEOUT` for, twice over.
const LAND_WAIT: Duration = Duration::from_secs(2 * SETTINGS_WRITE_TIMEOUT.as_secs());

fn service(path: &Path, io: Option<SettingsIo>) -> Arc<RunService> {
    let config = ManagerConfig::for_tests("/tmp/ax-unused.sock".into(), "/bin/sh".into());
    let (manager, _events) = WindowManager::new(config);
    let data = path.parent().unwrap().join("data");
    let orchestrator = config::load(path).0.orchestrator;
    let live = LiveSettings::defaults_of(orchestrator.clone());
    let mut ctx = RunContext::new(data, manager.config(), orchestrator, Arc::new(NoRoots))
        .with_settings(live, path.to_path_buf());
    if let Some(io) = io {
        ctx.settings_io = io;
    }
    RunService::new(manager, ctx)
}

fn settings_reply(reply: RunReply) -> SettingsReply {
    match reply {
        RunReply::Settings { reply, .. } => *reply,
        other => panic!("not a settings reply: {other:?}"),
    }
}

async fn get(s: &RunService) -> SettingsReply {
    settings_reply(
        s.request(proto::RunRequest::Settings(SettingsRequest::Get))
            .await,
    )
}

async fn put(s: &RunService, doc: SettingsDoc) -> SettingsReply {
    let request = SettingsRequest::Put { settings: doc };
    settings_reply(s.request(proto::RunRequest::Settings(request)).await)
}

fn live_doc(s: &RunService) -> SettingsDoc {
    config::settings::doc_of(&s.ctx.settings.current().orchestrator)
}

fn file_doc(path: &Path) -> SettingsDoc {
    config::settings::doc_of(&config::load(path).0.orchestrator)
}

fn entries(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

const TEXT: &str = "# mine\n[orchestrator]\nmax_writers = 3 # three\nworker_sandbox = false\n";

fn written(dir: &Path) -> PathBuf {
    let path = dir.join("config.toml");
    std::fs::write(&path, TEXT).unwrap();
    path
}

#[tokio::test]
async fn get_answers_from_memory() {
    let dir = tempfile::tempdir().unwrap();
    // A directory: any read of it fails, so a `Current` reply came from memory.
    let path = dir.path().join("config.toml");
    std::fs::create_dir(&path).unwrap();
    let s = service(&path, None);
    let SettingsReply::Current {
        doc,
        origin,
        path: at,
    } = get(&s).await
    else {
        panic!("no Current reply")
    };
    assert_eq!(
        doc,
        config::settings::doc_of(&config::Orchestrator::default())
    );
    assert_eq!(at, path);
    assert!(origin.values().all(|o| *o == Origin::Default), "{origin:?}");
}

#[tokio::test]
async fn a_refused_put_writes_nothing_and_swaps_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let path = written(dir.path());
    let s = service(&path, None);
    let before = live_doc(&s);
    // Refused by `validate`, and by the writer (an owned key in an inline table).
    let mut empty = before.clone();
    empty.models.clear();
    let reply = put(&s, empty).await;
    assert_eq!(
        reply,
        SettingsReply::Refused {
            problems: vec!["enable at least one model".into()]
        }
    );
    std::fs::write(&path, "[orchestrator]\nagent = { model = \"\" }\n").unwrap();
    let inline = std::fs::read(&path).unwrap();
    let mut doc = before.clone();
    doc.orchestrator.runtime = Some(proto::Runtime::Claude);
    let SettingsReply::Refused { problems } = put(&s, doc).await else {
        panic!("an inline table was rewritten")
    };
    assert!(problems[0].contains("does not edit"), "{problems:?}");
    assert_eq!(std::fs::read(&path).unwrap(), inline);
    assert_eq!(entries(dir.path()), ["config.toml"]);
    assert_eq!(live_doc(&s), before);
}

#[tokio::test]
async fn a_put_writes_swaps_and_answers_saved() {
    let dir = tempfile::tempdir().unwrap();
    let path = written(dir.path());
    let s = service(&path, None);
    let mut doc = live_doc(&s);
    assert_eq!(doc.limits.max_writers, 3);
    doc.limits.max_writers = 1;
    let SettingsReply::Saved { doc: saved, origin } = put(&s, doc.clone()).await else {
        panic!("not saved")
    };
    assert_eq!(saved, doc);
    assert_eq!(origin["orchestrator.max_writers"], Origin::File);
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(
        text.contains("max_writers = 1 # three") && text.starts_with("# mine"),
        "{text}"
    );
    assert_eq!(live_doc(&s), doc);
    assert!(
        !s.ctx.settings.current().orchestrator.worker_sandbox,
        "a key Settings does not own stays as loaded"
    );
    let SettingsReply::Current { doc: now, .. } = get(&s).await else {
        panic!("no Current reply")
    };
    assert_eq!(now, doc);
}

/// Ruling T18-7: `[orchestrator.design].default` hand-edited while the daemon runs (the
/// only way to change it) leaves the daemon's report stale; a put of the stale report,
/// or of none, still saves, and the hand edit stays in the file.
#[tokio::test]
async fn a_hand_edited_design_default_still_saves() {
    let dir = tempfile::tempdir().unwrap();
    let path = written(dir.path());
    let s = service(&path, None);
    let stale = live_doc(&s);
    assert_eq!(stale.design_default, Some(proto::DesignMode::Full));
    let mut text = std::fs::read_to_string(&path).unwrap();
    text.push_str("\n[orchestrator.design]\ndefault = \"off\"\n");
    std::fs::write(&path, text).unwrap();
    for (sent, writers) in [(stale.design_default, 1), (None, 2)] {
        let doc = SettingsDoc {
            design_default: sent,
            limits: proto::SettingsLimits {
                max_writers: writers,
                ..stale.limits.clone()
            },
            ..stale.clone()
        };
        let reply = put(&s, doc).await;
        assert!(
            matches!(reply, SettingsReply::Saved { .. }),
            "{sent:?}: {reply:?}"
        );
        let file = file_doc(&path);
        assert_eq!(file.limits.max_writers, writers);
        assert_eq!(file.design_default, Some(proto::DesignMode::Off));
    }
}

/// A save stand-in that counts how many saves run at once, around the real one.
fn counting(inside: Arc<AtomicUsize>, most: Arc<AtomicUsize>) -> SaveFn {
    Arc::new(move |path, doc, cancel| {
        let now = inside.fetch_add(1, Ordering::SeqCst) + 1;
        most.fetch_max(now, Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(20));
        let saved = config::settings::save(path, doc, cancel);
        inside.fetch_sub(1, Ordering::SeqCst);
        saved
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_puts_never_interleave() {
    let dir = tempfile::tempdir().unwrap();
    let path = written(dir.path());
    let (inside, most) = (Arc::default(), Arc::new(AtomicUsize::new(0)));
    let io = SettingsIo {
        timeout: SETTINGS_WRITE_TIMEOUT,
        save: counting(inside, most.clone()),
    };
    let s = service(&path, Some(io));
    let (mut one, mut two) = (live_doc(&s), live_doc(&s));
    one.limits.max_writers = 1;
    one.models.truncate(1);
    two.limits.max_writers = 2;
    two.limits.max_bounces = 4;
    let (a, b) = tokio::join!(put(&s, one.clone()), put(&s, two.clone()));
    assert!(matches!(a, SettingsReply::Saved { .. }), "{a:?}");
    assert!(matches!(b, SettingsReply::Saved { .. }), "{b:?}");
    assert_eq!(most.load(Ordering::SeqCst), 1, "two saves ran at once");
    let on_disk = file_doc(&path);
    assert!(on_disk == one || on_disk == two, "{on_disk:?}");
    assert_eq!(live_doc(&s), on_disk);
}

/// A save stand-in that waits for `release` and then runs the real save. With `claim`
/// it first takes the rename's claim, as a real save that had passed its check would.
fn held(release: Arc<AtomicBool>, done: Arc<AtomicBool>, claim: bool) -> SaveFn {
    Arc::new(move |path, doc, cancel| {
        let claimed = claim && !cancel.swap(true, Ordering::SeqCst);
        let deadline = Instant::now() + LAND_WAIT * 3;
        while !release.load(Ordering::SeqCst) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        let saved = if claimed {
            config::settings::save(path, doc, &AtomicBool::new(false))
        } else {
            config::settings::save(path, doc, cancel)
        };
        done.store(true, Ordering::SeqCst);
        saved
    })
}

async fn wait_for(what: &str, deadline: Duration, f: impl Fn() -> bool) {
    let end = Instant::now() + deadline;
    while !f() {
        assert!(Instant::now() < end, "waited {deadline:?} for {what}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_put_that_outlives_the_timeout_refuses_and_does_not_rename() {
    let dir = tempfile::tempdir().unwrap();
    let path = written(dir.path());
    let (release, done) = (
        Arc::new(AtomicBool::new(false)),
        Arc::new(AtomicBool::new(false)),
    );
    let save = held(release.clone(), done.clone(), false);
    let s = service(
        &path,
        Some(SettingsIo {
            timeout: SHORT,
            save,
        }),
    );
    let before = live_doc(&s);
    let mut doc = before.clone();
    doc.limits.max_writers = 1;
    let reply = put(&s, doc).await;
    assert_eq!(
        reply,
        SettingsReply::Refused {
            problems: vec![WRITE_TIMEOUT_TEXT.into()]
        }
    );
    release.store(true, Ordering::SeqCst);
    wait_for("the held save to return", LAND_WAIT, || {
        done.load(Ordering::SeqCst)
    })
    .await;
    assert_eq!(std::fs::read_to_string(&path).unwrap(), TEXT);
    assert_eq!(entries(dir.path()), ["config.toml"]);
    // Give the background task its swap chance; nothing may change.
    let _guard = s.settings_write.lock().await;
    assert_eq!(live_doc(&s), before);
}

/// Task 9 notes: a save that had claimed the rename when the timeout fired is reported
/// as not known yet, never as "nothing changed", and is swapped in once it lands.
#[tokio::test(flavor = "multi_thread")]
async fn a_put_whose_rename_had_begun_reports_the_outcome_unknown_and_lands() {
    let dir = tempfile::tempdir().unwrap();
    let path = written(dir.path());
    let (release, done) = (
        Arc::new(AtomicBool::new(false)),
        Arc::new(AtomicBool::new(false)),
    );
    let save = held(release.clone(), done.clone(), true);
    let s = service(
        &path,
        Some(SettingsIo {
            timeout: SHORT,
            save,
        }),
    );
    let mut doc = live_doc(&s);
    doc.limits.max_writers = 1;
    let reply = put(&s, doc.clone()).await;
    assert_eq!(
        reply,
        SettingsReply::Refused {
            problems: vec![WRITE_UNKNOWN_TEXT.into()]
        }
    );
    // The exact text (Exact user-visible text): no advice to reopen Settings.
    assert_eq!(
        WRITE_UNKNOWN_TEXT,
        "config.toml was still being written after 5 s; if the write completes, new runs use the new settings"
    );
    release.store(true, Ordering::SeqCst);
    wait_for("the late save to be applied", LAND_WAIT, || {
        live_doc(&s) == doc
    })
    .await;
    assert_eq!(file_doc(&path), doc);
}

#[test]
fn the_scout_roster_is_live() {
    use crate::scout::spec::Roster;
    let live = LiveSettings::defaults_of(config::Orchestrator::default());
    let roster = Roster::Live(live.clone());
    assert_eq!(roster.current(), config::default_roster());
    let mut changed = config::Orchestrator::default();
    changed.models.truncate(1);
    live.swap_owned(&changed, Default::default());
    assert_eq!(roster.current(), changed.models);
    let fixed: Roster = config::default_roster().into();
    assert_eq!(fixed.current(), config::default_roster());
}

/// Decision 37: the runs a profile edit waits for are the project's runs that are
/// neither terminal nor complete.
#[test]
fn live_runs_in_lists_the_projects_unfinished_runs() {
    use crate::run::test_support::{PROFILE, plan_with, run_ok, task_toml};
    let dir = tempfile::tempdir().unwrap();
    let s = service(&dir.path().join("config.toml"), None);
    let project = PathBuf::from("/work/app");
    let states = [
        ("r-0001", RunState::Running, &project),
        ("r-0002", RunState::Complete, &project),
        ("r-0003", RunState::Accepted, &project),
        ("r-0004", RunState::AwaitingApproval, &project),
    ];
    let other = PathBuf::from("/work/other");
    let plan = plan_with(PROFILE, &[task_toml("t1", "S", "[\"crates/a/**\"]", "")]);
    for (id, run_state, at) in states
        .into_iter()
        .chain([("r-0005", RunState::Running, &other)])
    {
        let mut run = run_ok(&plan);
        (run.id, run.state, run.project) = (id.into(), run_state, at.clone());
        crate::lock(&s.state).runs.insert(id.into(), run);
    }
    assert_eq!(s.live_runs_in(&project), ["r-0001", "r-0004"]);
    assert!(s.live_runs_in(Path::new("/work/none")).is_empty());
}

/// Decision 29: the onboarding scouts `profile::service::wire` builds read the run
/// service's live roster, so a save reaches the next scout without a restart.
#[test]
fn the_wired_scouts_read_the_live_roster() {
    let dir = tempfile::tempdir().unwrap();
    let s = service(&dir.path().join("config.toml"), None);
    let (data, socket) = (dir.path().join("data"), dir.path().join("d.sock"));
    let default = config::Orchestrator::default();
    let profiles = crate::profile::service::wire(&s.manager, &s, &data, &socket, &default);
    let roster = &profiles.scouts().context().roster;
    assert_eq!(roster.current(), config::default_roster());
    let mut changed = default.clone();
    changed.models.truncate(1);
    s.live_settings().swap_owned(&changed, Default::default());
    assert_eq!(roster.current(), changed.models);
}
