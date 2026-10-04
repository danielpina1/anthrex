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
            effort: Effort::Medium,
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
    let _ = std::fs::remove_file(pack_file(rig));
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
            effort: Effort::High,
        },
        session: 1,
        window_id: None,
        state: DesignAgentState::Running,
        calls: 0,
        tokens: 0,
        started: None,
        listed: false,
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
