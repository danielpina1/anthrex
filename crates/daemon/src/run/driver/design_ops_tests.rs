//! Milestone 9.6 task M9.6.8: the input pack the driver reads off the engine for a
//! brainstormer's first turn (decision 11): the stored scout reports, and a continued
//! goal's previous approved spec read through its index's check (decision 30). Real
//! files under the read rig's data dir; no agent.

use proto::{
    DocAuthor, DocKind, Effort, Route, RunState, Runtime, ScoutFile, ScoutKind, ScoutReport,
    Strength, TokenUsage,
};

use super::read_rig::Rig;
use crate::run::design::pack::{freeze, previous_spec};
use crate::run::design::state::{self, DesignState, NewDoc};
use crate::run::driver::design_io::write_new;
use crate::run::engine::Effect;
use crate::run::model::Run;

const SPEC: &str = "# Reset\n\n## Goal and success criteria\nUsers reset passwords.\n\n\
                    ## Requirements\nR1 Tokens expire. Check: a clock test.\n\n## Risks\nMail.\n";

fn report(id: &str) -> ScoutReport {
    ScoutReport {
        id: id.into(),
        kind: ScoutKind::Area,
        run_id: None,
        question: "Where are tokens kept?".into(),
        summary: "In the auth crate.".into(),
        files: vec![ScoutFile {
            path: "crates/auth/src/lib.rs".into(),
            why: "the store".into(),
        }],
        modules: Vec::new(),
        interfaces: vec!["fn reset(token)".into()],
        risks: vec!["mail delays".into()],
        profile: None,
        route: Route {
            runtime: Runtime::Codex,
            model: String::new(),
            strength: Strength::Standard,
            effort: Effort::MEDIUM,
        },
        window_id: None,
        started_at: 0,
        finished_at: 0,
        tool_calls: 0,
        usage: TokenUsage::default(),
    }
}

/// A design run with the user's answers and one stored run scout report.
fn brainstorming(run: &mut Run, dir: &std::path::Path) {
    run.design_mode = proto::DesignMode::Full;
    run.state = RunState::Brainstorming;
    run.repo_dir = dir.join("repo-data");
    std::fs::create_dir_all(&run.repo_dir).unwrap();
    run.scout_reports = vec!["s1".into()];
    run.orch.design = Some(DesignState {
        answers: Some("Links live an hour.".into()),
        pack: Some(freeze(run, None)),
        ..DesignState::default()
    });
    let scouts = run.data_dir.join("scouts");
    std::fs::create_dir_all(&scouts).unwrap();
    let text = serde_json::to_string(&report("s1")).unwrap();
    std::fs::write(scouts.join("s1.json"), text).unwrap();
}

/// Ruling T8-2: the previous spec frozen into the run's pack, as the engine freezes it
/// when the brainstormers are queued.
fn freeze_previous(rig: &Rig) {
    let mut engine = crate::lock(&rig.runs.state);
    let earlier = previous_spec(engine.runs.values(), &rig.run_id);
    let run = engine.runs.get_mut(&rig.run_id).unwrap();
    run.orch.design.as_mut().unwrap().pack = Some(freeze(run, earlier));
}

/// The previous run of a chain, finished, its spec v1 stored and written.
fn previous(rig: &Rig) {
    let mut prev = crate::run::orch::test_support::run_of(1);
    prev.id = "prev-run-0001".into();
    prev.data_dir = {
        let engine = crate::lock(&rig.runs.state);
        engine.runs[&rig.run_id].data_dir.join("prev")
    };
    prev.design_mode = proto::DesignMode::Full;
    prev.orch.design = Some(DesignState::default());
    prev.state = RunState::Complete;
    prev.continued_by = Some(rig.run_id.clone());
    let doc = NewDoc::new(DocKind::Spec, DocAuthor::Orchestrator, "submitted", SPEC);
    let (_, effect) = state::store(&mut prev, doc, 10).unwrap();
    // Task M9.6.10: approved, so its requirements are stored.
    let design = prev.orch.design.as_mut().unwrap();
    design.approved_spec = Some(1);
    design.requirements = crate::run::design::requirements::scan(SPEC);
    let Effect::WriteDoc { path, text, .. } = effect else {
        panic!("not a write");
    };
    write_new(&path, &text).unwrap();
    crate::lock(&rig.runs.state)
        .runs
        .insert(prev.id.clone(), prev);
}

/// The round's pack file, `design/brainstorm/pack-r1.md`.
fn pack_file(rig: &Rig) -> std::path::PathBuf {
    let engine = crate::lock(&rig.runs.state);
    state::design_dir(&engine.runs[&rig.run_id]).join("brainstorm/pack-r1.md")
}

/// The pack built anew from the frozen inputs: the round's file is removed first, as
/// if no start of the round had written it yet (ruling T8-6 sends that file after).
async fn built(rig: &Rig) -> String {
    rig.runs.doc_writes.forget_pack(&pack_file(rig));
    rig.runs.brainstorm_pack(&rig.run_id).await.unwrap().0
}

#[tokio::test(flavor = "multi_thread")]
async fn the_pack_reads_the_reports_and_the_previous_spec_off_the_engine() {
    let rig = Rig::new(brainstorming).await;
    let pack = built(&rig).await;
    for needle in [
        "Links live an hour.",
        "Scout report s1 (Where are tokens kept?)",
        "crates/auth/src/lib.rs: the store",
        "fn reset(token)",
        "mail delays",
    ] {
        assert!(pack.contains(needle), "{needle} in\n{pack}");
    }
    assert!(!pack.contains("Related earlier work"));
    previous(&rig);
    freeze_previous(&rig);
    let pack = built(&rig).await;
    assert!(pack.contains("Related earlier work"), "{pack}");
    assert!(pack.contains("prev/design/spec-v1.md"), "{pack}");
    assert!(pack.contains("R1 Tokens expire. Check: a clock test."));
    assert!(
        !pack.contains("Mail."),
        "only the Goal and Requirements sections"
    );
    // A spec file changed since it was stored is never carried (task 5's I-1).
    let path = {
        let engine = crate::lock(&rig.runs.state);
        let prev = &engine.runs["prev-run-0001"];
        let design = prev.orch.design.as_ref().unwrap();
        let v = design.find(DocKind::Spec, Some(1)).unwrap();
        state::design_dir(prev).join(design.file_name(v))
    };
    std::fs::write(
        &path,
        SPEC.replace("an hour", "a day").replace("expire", "lapse"),
    )
    .unwrap();
    let pack = built(&rig).await;
    assert!(!pack.contains("Related earlier work"), "{pack}");
}

/// Ruling T15-1: a previous run that iterated is carried as round 1's spec then each
/// approved amendment, each read against its index entry; one that no longer matches
/// is a line saying so, round 1's spec staying.
#[tokio::test(flavor = "multi_thread")]
async fn the_pack_reads_round_ones_spec_and_each_amendment() {
    let rig = Rig::new(brainstorming).await;
    previous(&rig);
    let amendment = "# Reset in the app\n\n## Goal and success criteria\nThe app too.\n\n\
                     ## Requirements\nR2 The app opens links. Check: a link test.\n";
    let path = {
        let mut engine = crate::lock(&rig.runs.state);
        let prev = engine.runs.get_mut("prev-run-0001").unwrap();
        let doc = NewDoc::new(
            DocKind::Spec,
            DocAuthor::Orchestrator,
            "submitted",
            amendment,
        );
        let (_, effect) = state::store(prev, doc, 20).unwrap();
        let design = prev.orch.design.as_mut().unwrap();
        design.approved_before = design.approved_specs();
        let round =
            crate::run::design::round::DesignRound::starting(design, 2, proto::RoundDesign::Amend);
        design.round = Some(round);
        design.approved_spec = Some(2);
        let Effect::WriteDoc { path, text, .. } = effect else {
            panic!("not a write");
        };
        write_new(&path, &text).unwrap();
        path
    };
    freeze_previous(&rig);
    let pack = built(&rig).await;
    let at = |needle: &str| {
        pack.find(needle)
            .unwrap_or_else(|| panic!("{needle} in\n{pack}"))
    };
    assert!(at("R1 Tokens expire.") < at("  Its round 2 amendment, "));
    assert!(at("prev/design/spec-v2.md") < at("R2 The app opens links."));
    std::fs::write(&path, amendment.replace("links", "pages")).unwrap();
    let pack = built(&rig).await;
    assert!(pack.contains("R1 Tokens expire."), "{pack}");
    assert!(!pack.contains("R2 The app opens links."), "{pack}");
    // Ruling T15-11 (N4): its place says so.
    assert!(
        pack.contains("/prev/design/spec-v2.md: could not be read back"),
        "{pack}"
    );
}

/// Ruling T8-2: the pack reads exactly the inputs frozen when the brainstormers were
/// queued: a run scout's report that lands between the two starts, and a previous spec
/// approved since, change nothing; the two packs are byte-equal.
#[tokio::test(flavor = "multi_thread")]
async fn the_pack_reads_exactly_its_frozen_inputs() {
    let rig = Rig::new(brainstorming).await;
    let first = built(&rig).await;
    assert!(first.contains("Scout report s1"), "{first}");
    let scouts = {
        let mut engine = crate::lock(&rig.runs.state);
        let run = engine.runs.get_mut(&rig.run_id).unwrap();
        run.scout_reports.push("s2".into());
        run.data_dir.join("scouts")
    };
    let text = serde_json::to_string(&report("s2")).unwrap();
    std::fs::write(scouts.join("s2.json"), text).unwrap();
    previous(&rig);
    let second = built(&rig).await;
    assert_eq!(first, second);
    assert!(!second.contains("Scout report s2"));
}

/// Ruling T8-3: a design folder under a symlinked data dir is denied by both its paths,
/// as given and canonical (resolved through its nearest existing ancestor, since the
/// folder is made later), in `permissions.deny` and `sandbox.filesystem.denyRead`.
#[tokio::test(flavor = "multi_thread")]
async fn a_symlinked_data_dir_is_denied_by_both_its_paths() {
    use crate::headless::ClaudeSandbox;
    use crate::headless::argv::{CLI_CAPS, claude_settings};
    use crate::run::driver::design_ops::with_canonical;
    let dir = tempfile::tempdir().unwrap();
    let real = dir.path().join("real");
    std::fs::create_dir_all(real.join("runs/r1")).unwrap();
    let link = dir.path().join("link");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    let design = link.join("runs/r1/design");
    let canonical = std::fs::canonicalize(&real).unwrap().join("runs/r1/design");
    assert_ne!(design, canonical);
    let denied = with_canonical(vec![design.clone()]).await;
    assert_eq!(denied, [design.clone(), canonical.clone()]);
    let sandbox = ClaudeSandbox {
        writable_roots: Vec::new(),
        deny_write: Vec::new(),
        deny_read: denied,
    };
    let path = std::path::Path::new("/opt/anthrex");
    let settings = claude_settings(path, 7, Some(&sandbox), &CLI_CAPS);
    let shown = |p: &std::path::Path| p.display().to_string();
    assert_eq!(
        settings["permissions"]["deny"],
        serde_json::json!([
            format!("Read(/{}/**)", shown(&design)),
            format!("Read(/{}/**)", shown(&canonical))
        ])
    );
    assert_eq!(
        settings["sandbox"]["filesystem"]["denyRead"],
        serde_json::json!([shown(&design), shown(&canonical)])
    );
    // A path with nothing to resolve is denied as given, once.
    let plain = std::fs::canonicalize(dir.path())
        .unwrap()
        .join("runs/r2/design");
    assert_eq!(with_canonical(vec![plain.clone()]).await, [plain]);
}

/// Ruling T8-4: a Claude brainstormer cannot read the Codex sessions folder, where a
/// Codex brainstormer's session would be saved (`$CODEX_HOME/sessions`, else
/// `~/.codex/sessions`); a Codex brainstormer gets nothing more.
#[test]
fn a_claude_brainstormer_is_denied_the_codex_sessions() {
    use crate::run::design::state::{DesignAgent, DesignAgentState};
    use crate::run::driver::design_ops::{codex_sessions_dir, deny_codex_sessions};
    use crate::scout::design_spec::brainstormer_spec;
    let env = |pairs: &'static [(&'static str, &'static str)]| {
        move |key: &str| {
            (pairs.iter())
                .find(|(k, _)| *k == key)
                .map(|(_, v)| std::ffi::OsString::from(v))
        }
    };
    const HOME: &[(&str, &str)] = &[("HOME", "/Users/u")];
    const BOTH: &[(&str, &str)] = &[("HOME", "/Users/u"), ("CODEX_HOME", "/opt/codex")];
    let dir = codex_sessions_dir(env(HOME));
    assert_eq!(dir, Some("/Users/u/.codex/sessions".into()));
    assert_eq!(
        codex_sessions_dir(env(BOTH)),
        Some("/opt/codex/sessions".into())
    );
    assert_eq!(codex_sessions_dir(env(&[])), None);
    let mut run = crate::run::orch::test_support::run_of(1);
    run.design_mode = proto::DesignMode::Full;
    let agent = |label: &str, runtime| DesignAgent {
        label: label.into(),
        role: proto::AgentRole::Brainstormer,
        route: Route {
            runtime,
            model: String::new(),
            strength: Strength::Frontier,
            effort: Effort::HIGH,
        },
        session: 1,
        window_id: None,
        state: DesignAgentState::Running,
        calls: 0,
        tokens: 0,
        started: None,
        listed: false,
        unsubmitted: false,
        round: 1,
    };
    let mut claude = brainstormer_spec(&run, &agent("claude", Runtime::Claude));
    deny_codex_sessions(&mut claude, dir.clone());
    let denied = &claude.headless.claude_sandbox.as_ref().unwrap().deny_read;
    assert_eq!(
        denied,
        &[state::design_dir(&run), "/Users/u/.codex/sessions".into()]
    );
    let mut codex = brainstormer_spec(&run, &agent("codex", Runtime::Codex));
    let before = codex.clone();
    deny_codex_sessions(&mut codex, dir);
    assert_eq!(codex, before);
}

/// Ruling T8-6: the first start of a round writes its pack to
/// `design/brainstorm/pack-r<k>.md` and reports its length and SHA-256; every later
/// start sends exactly that file, read back against them, whatever changed since (a new
/// report, a new profile); a file missing or changed is never replaced by another pack.
#[tokio::test(flavor = "multi_thread")]
async fn the_rounds_pack_is_written_once_and_every_start_sends_it() {
    use crate::run::design::pack::PackFile;
    use crate::run::design::state::sha256_hex;
    let rig = Rig::new(brainstorming).await;
    let (first, file) = rig.runs.brainstorm_pack(&rig.run_id).await.unwrap();
    let path = pack_file(&rig);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), first);
    let expected = PackFile {
        bytes: first.len() as u64,
        sha256: sha256_hex(first.as_bytes()),
    };
    assert_eq!(file.as_ref(), Some(&expected));
    // A start that raced the first, or followed a failed launch, sends the same file.
    let (again, _) = rig.runs.brainstorm_pack(&rig.run_id).await.unwrap();
    assert_eq!(again, first);
    // The engine recorded it; a later start reads it back, and reports nothing new.
    let scouts = {
        let mut engine = crate::lock(&rig.runs.state);
        let run = engine.runs.get_mut(&rig.run_id).unwrap();
        let design = run.orch.design.as_mut().unwrap();
        design.pack.as_mut().unwrap().file = Some(expected);
        run.scout_reports.push("s2".into());
        run.data_dir.join("scouts")
    };
    std::fs::write(
        scouts.join("s2.json"),
        serde_json::to_string(&report("s2")).unwrap(),
    )
    .unwrap();
    let (later, none) = rig.runs.brainstorm_pack(&rig.run_id).await.unwrap();
    assert_eq!((later.as_str(), none), (first.as_str(), None));
    std::fs::write(&path, first.replace("an hour", "a day")).unwrap();
    let changed = rig.runs.brainstorm_pack(&rig.run_id).await.unwrap_err();
    assert!(changed.contains("does not match"), "{changed}");
    std::fs::remove_file(&path).unwrap();
    let missing = rig.runs.brainstorm_pack(&rig.run_id).await.unwrap_err();
    assert!(missing.contains("pack-r1.md"), "{missing}");
}

/// Fix round 2: when the canonical paths cannot be resolved in time, the paths as
/// given are denied, and the fallback is logged as a warning.
#[tokio::test]
async fn a_denial_that_cannot_be_resolved_in_time_is_warned() {
    use crate::run::driver::design_ops::resolved_by;
    use std::io::Write;
    use std::sync::{Arc, Mutex};
    #[derive(Clone, Default)]
    struct Log(Arc<Mutex<Vec<u8>>>);
    impl Write for Log {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            crate::lock(&self.0).extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let log = Log::default();
    let writer = log.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(move || writer.clone())
        .with_ansi(false)
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);
    // The resolver blocks until the test releases it, after the wait has passed, so no
    // scheduling stall can let it finish in time.
    static RELEASED: (Mutex<bool>, std::sync::Condvar) =
        (Mutex::new(false), std::sync::Condvar::new());
    fn stuck(_: &std::path::Path) -> Option<std::path::PathBuf> {
        let mut released = crate::lock(&RELEASED.0);
        while !*released {
            released = RELEASED.1.wait(released).unwrap_or_else(|e| e.into_inner());
        }
        Some("/elsewhere".into())
    }
    let given = vec![std::path::PathBuf::from("/data/runs/r1/design")];
    let wait = std::time::Duration::from_millis(20);
    let denied = resolved_by(given.clone(), stuck, wait).await;
    *crate::lock(&RELEASED.0) = true;
    RELEASED.1.notify_all();
    assert_eq!(denied, given);
    let text = String::from_utf8(crate::lock(&log.0).clone()).unwrap();
    assert!(text.contains("WARN"), "{text}");
    assert!(text.contains("denied as given"), "{text}");
}

/// Task M9.6.9 (decision 7): a rethink's round reads the previous merged report through
/// its index entry's check, and the pack carries it, without the appendix of drafts,
/// with the user's note; a report changed since it was stored is not carried.
#[tokio::test(flavor = "multi_thread")]
async fn a_rethinks_pack_reads_the_previous_report_off_the_engine() {
    use crate::run::design::pack::FrozenRethink;
    use crate::run::design::report::attach;
    let rig = Rig::new(brainstorming).await;
    let report = "## Where they agree\nTokens.\n\n## Approaches\n### Stored tokens [both]\n";
    let file = attach(
        report,
        &[("claude".into(), Ok("## Understanding\nMINE\n".into()))],
    );
    let path = {
        let mut engine = crate::lock(&rig.runs.state);
        let run = engine.runs.get_mut(&rig.run_id).unwrap();
        let doc = NewDoc::new(
            DocKind::Brainstorm,
            DocAuthor::Orchestrator,
            "submitted",
            &file,
        );
        let (version, effect) = state::store(run, doc, 10).unwrap();
        let Effect::WriteDoc { path, text, .. } = effect else {
            panic!("not a write");
        };
        write_new(&path, &text).unwrap();
        let design = run.orch.design.as_mut().unwrap();
        design.rethinks = 1;
        let mut frozen = design.pack.clone().unwrap();
        frozen.round = 2;
        frozen.rethink = Some(FrozenRethink {
            note: "Think about SSO too.".into(),
            path: path.clone(),
            version,
        });
        design.pack = Some(frozen);
        path
    };
    let read = rig.runs.brainstorm_pack(&rig.run_id).await.unwrap().0;
    for needle in [
        "Think about SSO too.",
        "The previous merged report, v1:",
        "Tokens.",
    ] {
        assert!(read.contains(needle), "{needle} in\n{read}");
    }
    assert!(
        !read.contains("MINE") && !read.contains("Appendix"),
        "{read}"
    );
    assert!(read.contains("Scout report s1"), "the same reports: {read}");
    // The round's own pack file, pack-r2.md, holds it.
    let round = path.parent().unwrap().join("brainstorm/pack-r2.md");
    assert_eq!(std::fs::read_to_string(&round).unwrap(), read);
    rig.runs.doc_writes.forget_pack(&round);
    std::fs::remove_file(&path).unwrap();
    std::fs::write(&path, file.replace("Tokens.", "Changed.")).unwrap();
    let read = rig.runs.brainstorm_pack(&rig.run_id).await.unwrap().0;
    assert!(
        read.contains("The previous merged report, v1, could not be read."),
        "{read}"
    );
    assert!(!read.contains("Changed."), "{read}");
}

/// Task 8's re-review and the final fix wave's FW-36: the driver gives the engine the
/// end's cause by type, and compares no text. The machine's typed unsubmitted end is
/// `Unsubmitted`; a failure with the same words but not typed, or any other failure,
/// is `Failed`, and is not relaunched.
#[test]
fn an_unsubmitted_end_reaches_the_engine_as_its_own_cause() {
    use super::super::design_ops::design_end;
    use crate::run::engine::ScoutEnd;
    use crate::scout::design_spec::DOC_REVIEWER_TEXTS;
    use crate::scout::machine::unsubmitted;
    use crate::scout::service::ScoutOutcome;
    let own = unsubmitted(&DOC_REVIEWER_TEXTS);
    let typed = Some(ScoutOutcome::Unsubmitted {
        reason: own.clone(),
    });
    assert_eq!(
        design_end(typed),
        ScoutEnd::Unsubmitted {
            reason: own.clone()
        }
    );
    let failed = |reason: String| Some(ScoutOutcome::Failed { reason });
    assert_eq!(
        design_end(failed(own.clone())),
        ScoutEnd::Failed { reason: own }
    );
    assert_eq!(design_end(Some(ScoutOutcome::Accepted)), ScoutEnd::Reported);
    assert!(matches!(design_end(None), ScoutEnd::Failed { .. }));
}
