//! M9.17 fix round 2, item 3: decision 17's `installed` reads `PATH` as a shell does, a
//! leading `~` expanded to `HOME`.

use std::ffi::OsStr;
use std::os::unix::fs::PermissionsExt;

use super::executable_in;

#[test]
fn a_tilde_entry_on_path_is_expanded_to_home() {
    let home = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(home.path().join("bin")).unwrap();
    for file in ["bin/agent", "top"] {
        let path = home.path().join(file);
        std::fs::write(&path, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let h = Some(home.path().as_os_str());
    let path = |p: &'static str| Some(OsStr::new(p));
    assert!(executable_in("agent", path("/nonexistent:~/bin"), h));
    assert!(executable_in("top", path("~"), h));
    assert!(!executable_in("agent", path("~"), h));
    // No HOME: the entry names nothing.
    assert!(!executable_in("agent", path("~/bin"), None));
    // `~user` is not the user's home; an absolute entry is read as it is.
    assert!(!executable_in("agent", path("~nobody/bin"), h));
    let absolute = home.path().join("bin");
    assert!(executable_in("agent", Some(absolute.as_os_str()), None));
    // A binary with a `/` never reads PATH.
    assert!(!executable_in("bin/agent", path("~"), h));
    assert!(!executable_in("agent", None, h));
}

/// M9.17 fix round 3, item 3: an empty `HOME` is no home; `~/bin` is not read as the
/// relative `bin`.
#[test]
fn an_empty_home_expands_nothing() {
    use std::path::{Path, PathBuf};
    let entry =
        |dir: &str, home: Option<&str>| super::path_entry(PathBuf::from(dir), home.map(OsStr::new));
    assert_eq!(entry("~/bin", Some("")), None);
    assert_eq!(entry("~", Some("")), None);
    assert_eq!(entry("~/bin", None), None);
    assert_eq!(
        entry("~/bin", Some("/h")),
        Some(Path::new("/h/bin").to_path_buf())
    );
    assert_eq!(entry("/usr/bin", Some("")), Some(PathBuf::from("/usr/bin")));
    assert_eq!(entry("~x/bin", Some("/h")), Some(PathBuf::from("~x/bin")));
}

/// M9.17 fix round 3, item 3: the check matches what launches the binary. Only the
/// orchestrator's window goes through a shell (`/bin/sh -c 'exec "$0" "$@"'`), and only
/// macOS's `/bin/sh` (bash) expands `~` in `PATH`; a headless session (a sub-planner, a
/// scout) is spawned directly, whose `PATH` search never expands it.
#[test]
fn only_a_window_on_macos_finds_a_binary_through_a_tilde_entry() {
    let home = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(home.path().join("bin")).unwrap();
    let agent = home.path().join("bin/claude");
    std::fs::write(&agent, "#!/bin/sh\n").unwrap();
    std::fs::set_permissions(&agent, std::fs::Permissions::from_mode(0o755)).unwrap();
    let path = Some(OsStr::new("~/bin"));
    let h = Some(home.path().as_os_str());
    let yes = |map: &std::collections::BTreeMap<String, bool>| map.get("claude") == Some(&true);

    let mac = super::found_in(("claude", "codex"), path, h, true);
    assert!(yes(&mac.window), "a macOS window's shell expands it");
    assert!(!yes(&mac.headless), "a headless spawn does not");
    let linux = super::found_in(("claude", "codex"), path, h, false);
    assert!(!yes(&linux.window) && !yes(&linux.headless));
    let empty = super::found_in(("claude", "codex"), path, Some(OsStr::new("")), true);
    assert!(!yes(&empty.window));
    // An absolute entry is found by both.
    let absolute = home.path().join("bin");
    let both = super::found_in(("claude", "codex"), Some(absolute.as_os_str()), None, false);
    assert!(yes(&both.window) && yes(&both.headless));
    assert_eq!(both.headless.get("codex"), Some(&false));
}

/// Whole-branch review, item 3: a runtime the orchestrator's window finds only through
/// a `~` entry in `PATH` is refused for the sub-planners with the reason, not with
/// "choose another runtime", which a user with only that runtime cannot follow.
#[test]
fn a_planner_runtime_found_only_through_a_tilde_entry_says_so() {
    use proto::Runtime;
    let mut run = crate::run::orch::test_support::run_of(1);
    run.roster = config::default_roster();
    let mut o = crate::run::orch::test_support::orchestrator();
    o.route.runtime = Runtime::Codex;
    o.route.model = String::new();
    run.orch.orchestrator = Some(o);
    let map = |claude: bool, codex: bool| -> std::collections::BTreeMap<String, bool> {
        [("claude".to_string(), claude), ("codex".to_string(), codex)].into()
    };
    run.orch.installed = map(false, false);
    let bins = ("/nonexistent/claude".to_string(), "codex".to_string());
    let headless = map(false, false);
    let missing = super::missing_in(&headless, &bins);
    let window = map(false, true);
    let refusal = super::planner_refusal(&run, &missing, &window).unwrap();
    assert_eq!(
        refusal,
        "the sub-planners' runtime codex is found only through a `~` entry in PATH, which \
         headless sessions (sub-planners and scouts) do not search; put codex's directory \
         in PATH as an absolute path"
    );
    // Not found at all: the plain refusal.
    let refusal = super::planner_refusal(&run, &missing, &map(false, false)).unwrap();
    assert!(
        refusal.starts_with("the sub-planners' runtime codex is not installed (codex is not"),
        "{refusal}"
    );
}

/// Task M9.3.6a fix round 1, m1: a chain id keeps only the run id's last four
/// characters (`o-<h4>`), so a new id is redrawn while its suffix is a chain's in the
/// table, or a run's own chain. With every suffix taken, no id can be drawn.
#[test]
fn a_run_id_never_takes_a_chains_suffix() {
    use crate::manager::{GitRoots, ManagerConfig, WindowManager};
    use crate::run::chain::{Chain, ChainState};
    use crate::run::driver::RunService;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    struct NoRoots;
    impl GitRoots for NoRoots {
        fn register(&self, _: PathBuf) {}
        fn unregister(&self, _: &Path) {}
    }
    let dir = tempfile::tempdir().unwrap();
    let config = ManagerConfig::for_tests(dir.path().join("d.sock"), "/bin/sh".into());
    let (manager, _events) = WindowManager::new(config);
    let service = RunService::for_manager(&manager, dir.path().join("data"), Arc::new(NoRoots));
    assert!(service.pick_id("add login", &[]).is_ok());

    let chain = |id: String| Chain {
        id,
        project: dir.path().into(),
        runs: Vec::new(),
        window_id: 1,
        runtime: proto::Runtime::Claude,
        model: "m".into(),
        state: ChainState::Idle,
        ended: false,
    };
    {
        let mut state = crate::lock(&service.state);
        for n in 0..=u16::MAX {
            let c = chain(format!("o-{n:04x}"));
            state.chains.insert(c.id.clone(), c);
        }
    }
    assert_eq!(
        service.pick_id("add login", &[]),
        Err("could not pick a free run id".to_string())
    );

    // A run's own chain counts too: a chain dropped from the table keeps its id on its
    // runs, which a restart's rebuild would merge with a new run of the same suffix.
    let mut state = crate::lock(&service.state);
    state.chains.clear();
    let base = crate::run::orch::test_support::run_of(1);
    for n in 0..=u16::MAX {
        let mut run = base.clone();
        run.id = format!("old-{n:04x}");
        run.chain = Some(format!("o-{n:04x}"));
        state.runs.insert(run.id.clone(), run);
    }
    drop(state);
    assert_eq!(
        service.pick_id("add login", &[]),
        Err("could not pick a free run id".to_string())
    );

    // Task 6b (6a re-review N1): an unchained run's own suffix counts too, since a
    // promotion would start `o-<its suffix>` (`engine/chains.rs::assign`).
    let mut state = crate::lock(&service.state);
    state.runs.clear();
    for n in 0..=u16::MAX {
        let mut run = base.clone();
        run.id = format!("plain-{n:04x}");
        run.chain = None;
        state.runs.insert(run.id.clone(), run);
    }
    drop(state);
    assert_eq!(
        service.pick_id("add login", &[]),
        Err("could not pick a free run id".to_string())
    );
}
