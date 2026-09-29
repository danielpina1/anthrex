//! Milestone 9.1 task M9.1.7: decisions 9 and 11's I/O. Each test works in its own
//! temporary directory with shell scripts it writes; nothing reaches the network
//! (`cargo metadata` runs `--offline --no-deps`).

use std::cell::RefCell;
use std::ffi::OsStr;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use super::*;
use crate::run::test_support::{PROFILE, plan_with, run_ok, task_toml};
use crate::run::tiers::affected::affected;
use crate::run::tiers::graph::note_once;
use crate::run::tiers::steps::plan;
use crate::run::tiers::{Affected, ModuleInfo, StepKind};

pub(super) const GIT: &str = "git";

pub(super) fn git(dir: &Path, args: &[&str]) {
    let out = Command::new(GIT)
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.name=t", "-c", "user.email=t@t"])
        .args(["-c", "commit.gpgsign=false"])
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {out:?}");
}

pub(super) fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

pub(super) fn names(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

/// The real `cargo` on this process's `PATH`, if any.
fn cargo_available() -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join("cargo"))
        .find(|candidate| candidate.is_file())
}

fn cargo_tiers() -> TierProfile {
    TierProfile {
        module_graph: GraphSource::Cargo,
        module_names: ModuleNames::Cargo,
        ..TierProfile::default()
    }
}

pub(super) fn command_tiers(command: &str) -> TierProfile {
    TierProfile {
        build_check: Some("sh build.sh".into()),
        module_test: Some("sh test.sh {module}".into()),
        module_graph: GraphSource::Command(command.into()),
        module_names: ModuleNames::Dir,
        ..TierProfile::default()
    }
}

/// Lines in a counter file a script appends to.
pub(super) fn runs(counter: &Path) -> Vec<String> {
    std::fs::read_to_string(counter)
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect()
}

#[test]
fn cargo_graph_is_read_offline_and_cached_by_manifest_hash() {
    let Some(real_cargo) = cargo_available() else {
        eprintln!("skipped: no cargo on PATH, so no cargo metadata to read");
        return;
    };
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    write(
        &repo.join("Cargo.toml"),
        "[workspace]\nmembers = [\"crates/*\"]\nresolver = \"2\"\n",
    );
    write(
        &repo.join("crates/a/Cargo.toml"),
        "[package]\nname = \"ax-a\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    );
    write(&repo.join("crates/a/src/lib.rs"), "");
    write(
        &repo.join("crates/b/Cargo.toml"),
        "[package]\nname = \"ax-b\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\nax-a = { path = \"../a\" }\n",
    );
    write(&repo.join("crates/b/src/lib.rs"), "");
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "base"]);
    // A `cargo` ahead of the real one on `PATH` that logs each call, then runs it.
    let bin = tmp.path().join("bin");
    let log = tmp.path().join("cargo-calls");
    write(
        &bin.join("cargo"),
        &format!(
            "#!/bin/sh\necho \"$*\" >> '{}'\nexec '{}' \"$@\"\n",
            log.display(),
            real_cargo.display()
        ),
    );
    std::fs::set_permissions(bin.join("cargo"), std::fs::Permissions::from_mode(0o755)).unwrap();
    let path = std::env::join_paths(std::iter::once(bin.clone()).chain(std::env::split_paths(
        &std::env::var_os("PATH").unwrap_or_default(),
    )))
    .unwrap();
    let env = vec![("PATH".to_string(), path.to_string_lossy().to_string())];
    let data = tmp.path().join("data");
    let modules = names(&["crates/*"]);
    let read = || {
        module_graph(
            OsStr::new(GIT),
            &repo,
            &data,
            &cargo_tiers(),
            &modules,
            &[],
            &env,
            None,
        )
    };

    let GraphState::Known(graph) = read() else {
        panic!("a cargo workspace has a known graph: {:?}", read());
    };
    let expected: BTreeMap<String, ModuleInfo> = [
        (
            "ax-a".to_string(),
            ModuleInfo {
                dir: Some("crates/a".into()),
                deps: Default::default(),
            },
        ),
        (
            "ax-b".to_string(),
            ModuleInfo {
                dir: Some("crates/b".into()),
                deps: ["ax-a".to_string()].into(),
            },
        ),
    ]
    .into();
    assert_eq!(graph.modules, expected);
    let calls = runs(&log);
    assert_eq!(calls.len(), 1, "{calls:?}");
    assert!(
        calls[0].contains("metadata") && calls[0].contains("--offline"),
        "{calls:?}"
    );
    assert!(calls[0].contains("--no-deps"), "{calls:?}");
    assert!(
        !repo.join("Cargo.lock").exists(),
        "--no-deps writes no lock file"
    );
    assert!(
        !repo.join(GRAPH_CACHE_FILE).exists(),
        "never in the checkout"
    );

    // Unchanged manifests: the cache answers and cargo does not run.
    assert_eq!(read(), GraphState::Known(graph.clone()));
    assert_eq!(runs(&log).len(), 1, "a cache hit runs nothing");
    assert_eq!(cache_entries(&data.join(GRAPH_CACHE_FILE)).len(), 1);

    // A changed Cargo.toml is a new key: cargo runs again.
    write(
        &repo.join("crates/a/Cargo.toml"),
        "[package]\nname = \"ax-a\"\nversion = \"0.2.0\"\nedition = \"2021\"\n",
    );
    assert_eq!(read(), GraphState::Known(graph));
    assert_eq!(runs(&log).len(), 2, "a changed manifest re-runs cargo");
    assert_eq!(cache_entries(&data.join(GRAPH_CACHE_FILE)).len(), 2);

    // Ruling C-8 (4): an untracked new crate is a new key too.
    write(
        &repo.join("crates/c/Cargo.toml"),
        "[package]\nname = \"ax-c\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    );
    write(&repo.join("crates/c/src/lib.rs"), "");
    let GraphState::Known(with_c) = read() else {
        panic!("the workspace with an untracked crate has a known graph");
    };
    assert!(with_c.modules.contains_key("ax-c"), "{with_c:?}");
    assert_eq!(runs(&log).len(), 3, "an untracked Cargo.toml re-runs cargo");
}

#[test]
fn the_graph_cache_keeps_the_newest_32_entries_and_replaces_a_corrupt_file() {
    let tmp = tempfile::tempdir().unwrap();
    let cache = tmp.path().join("data").join(GRAPH_CACHE_FILE);
    let graph = |n: usize| {
        let mut graph = ModuleGraph::default();
        graph.modules.insert(format!("m{n}"), ModuleInfo::default());
        graph
    };
    for n in 0..40 {
        cache_put(&cache, &format!("{n:016x}"), &graph(n));
    }
    let entries = cache_entries(&cache);
    assert_eq!(entries.len(), GRAPH_CACHE_MAX);
    assert_eq!(entries[0].key, format!("{:016x}", 40 - GRAPH_CACHE_MAX));
    assert_eq!(cache_get(&cache, &format!("{:016x}", 7)), None, "dropped");
    assert_eq!(cache_get(&cache, &format!("{:016x}", 39)), Some(graph(39)));
    // Storing a key again makes it the newest, not a second entry.
    cache_put(&cache, &format!("{:016x}", 10), &graph(10));
    let entries = cache_entries(&cache);
    assert_eq!(entries.len(), GRAPH_CACHE_MAX);
    assert_eq!(entries.last().unwrap().key, format!("{:016x}", 10));

    std::fs::write(&cache, "{ not json").unwrap();
    assert_eq!(cache_get(&cache, &format!("{:016x}", 39)), None);
    cache_put(&cache, "k", &graph(1));
    assert_eq!(cache_entries(&cache).len(), 1, "a corrupt file is replaced");
}

#[test]
fn command_graph_reads_json_from_the_command() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    for module in ["a", "b", "c"] {
        write(&repo.join(format!("mods/{module}/x.txt")), module);
    }
    write(&repo.join("deps.txt"), "one\n");
    git(&repo, &["init", "-q", "-b", "main"]);
    let counter = tmp.path().join("graph-runs");
    write(
        &repo.join("graph.sh"),
        &format!(
            "echo run >> '{}'\nprintf '%s\\n' '{{\"a\":[],\"b\":[\"a\"],\"c\":[\"b\"]}}'\n",
            counter.display()
        ),
    );
    let data = tmp.path().join("data");
    let modules = names(&["mods/*"]);
    let manifests = names(&["deps.txt"]);
    let read = || {
        module_graph(
            OsStr::new(GIT),
            &repo,
            &data,
            &command_tiers("sh graph.sh"),
            &modules,
            &manifests,
            &[],
            None,
        )
    };
    let GraphState::Known(graph) = read() else {
        panic!("the command's JSON is a known graph: {:?}", read());
    };
    assert_eq!(graph.modules.len(), 3);
    assert_eq!(graph.modules["c"].deps, ["b".to_string()].into());
    assert_eq!(graph.modules["a"].dir.as_deref(), Some("mods/a"));
    assert_eq!(runs(&counter).len(), 1);
    assert_eq!(read(), GraphState::Known(graph.clone()), "cached");
    assert_eq!(runs(&counter).len(), 1, "a cache hit runs nothing");
    // A `manifests` file, the command text and the module directories are the key.
    write(&repo.join("deps.txt"), "two\n");
    assert_eq!(read(), GraphState::Known(graph.clone()));
    assert_eq!(runs(&counter).len(), 2, "a changed manifest re-runs it");
    write(&repo.join("mods/d/x.txt"), "d");
    let _ = read();
    assert_eq!(runs(&counter).len(), 3, "a new module directory re-runs it");

    // Output that is not a graph is unknown, with the parser's reason, and not cached.
    write(&repo.join("bad.sh"), "echo '{\"a\":[\"zz\"]}'\n");
    let bad = module_graph(
        OsStr::new(GIT),
        &repo,
        &data,
        &command_tiers("sh bad.sh"),
        &modules,
        &manifests,
        &[],
        None,
    );
    assert_eq!(bad, GraphState::Unknown("zz is not a module".into()));
    assert_eq!(cache_entries(&data.join(GRAPH_CACHE_FILE)).len(), 3);
}

/// Whether `pid` (or, with `group`, the process group `pid`) no longer exists. Signal
/// 0 only asks; nothing is sent.
fn gone(pid: libc::pid_t, group: bool) -> bool {
    // SAFETY: signal 0 performs the permission and existence check only.
    let r = unsafe {
        if group {
            libc::killpg(pid, 0)
        } else {
            libc::kill(pid, 0)
        }
    };
    r == -1 && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
}

#[test]
fn a_graph_command_that_times_out_is_unknown() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    std::fs::create_dir_all(repo.join("mods/a")).unwrap();
    // Review C-8 (6): the pid first, then the group in a second step, each renamed
    // into place whole, so a read never sees half a file.
    let pid_file = tmp.path().join("pid");
    let pgid_file = tmp.path().join("pgid");
    write(
        &repo.join("slow.sh"),
        &format!(
            "echo $$ > '{p}.tmp' && mv '{p}.tmp' '{p}'\nps -o pgid= -p $$ > '{g}.tmp' && mv '{g}.tmp' '{g}'\nsleep 30\n",
            p = pid_file.display(),
            g = pgid_file.display()
        ),
    );
    let state = module_graph_with(
        OsStr::new(GIT),
        &repo,
        &tmp.path().join("data"),
        &command_tiers("sh slow.sh"),
        (&names(&["mods/*"]), &[]),
        &[],
        None,
        Duration::from_secs(3),
        &capture,
    );
    let GraphState::Unknown(reason) = &state else {
        panic!("a timed-out graph command is unknown: {state:?}");
    };
    assert!(
        reason.starts_with("the graph command timed out after"),
        "{reason}"
    );
    let id = |file: &Path| {
        std::fs::read_to_string(file)
            .ok()
            .and_then(|text| text.trim().parse::<libc::pid_t>().ok())
    };
    let deadline = Instant::now() + Duration::from_secs(10);
    let (pid, pgid) = loop {
        if let (Some(pid), Some(pgid)) = (id(&pid_file), id(&pgid_file)) {
            break (pid, pgid);
        }
        assert!(Instant::now() < deadline, "the script never wrote its ids");
        std::thread::sleep(Duration::from_millis(20));
    };
    // SAFETY: getpgrp has no preconditions.
    assert_ne!(pgid, unsafe { libc::getpgrp() }, "its own group, not ours");
    // The whole group was killed: wait for the kernel to finish reaping it.
    let deadline = Instant::now() + Duration::from_secs(10);
    while !(gone(pid, false) && gone(pgid, true)) {
        assert!(
            Instant::now() < deadline,
            "the script {pid} or its group {pgid} outlived the timeout"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn a_failing_graph_command_runs_check_and_notes_it_once() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    write(&repo.join("mods/a/x.txt"), "a");
    write(&repo.join("graph.sh"), "echo broken >&2\nexit 3\n");
    let data = tmp.path().join("data");
    let tiers = command_tiers("sh graph.sh");
    let modules = names(&["mods/*"]);
    let mut run = run_ok(&plan_with(
        PROFILE,
        &[task_toml("t1", "S", "[\"crates/a/**\"]", "")],
    ));
    assert_eq!(run.graph_note, None);
    let mut graph_note_logged = Vec::new();
    for _ in 0..2 {
        let graph = module_graph(
            OsStr::new(GIT),
            &repo,
            &data,
            &tiers,
            &modules,
            &[],
            &[],
            None,
        );
        assert_eq!(
            graph,
            GraphState::Unknown("the graph command exited 3: broken".into())
        );
        let set = affected(
            &names(&["mods/a/x.txt"]),
            &tiers,
            &[],
            &[],
            &modules,
            &graph,
        );
        assert!(matches!(set, Affected::Full(_)), "{set:?}");
        let tier1 = plan(1, &set, &tiers, Some("sh check.sh"));
        let steps: Vec<(&StepKind, &str)> = tier1
            .steps
            .iter()
            .map(|s| (&s.kind, s.command.as_str()))
            .collect();
        assert_eq!(steps, [(&StepKind::Tests, "sh check.sh")], "check alone");
        graph_note_logged.extend(note_once(&mut run.graph_note, &graph));
    }
    let note = "module graph unknown: the graph command exited 3: broken; every tier runs check";
    assert_eq!(graph_note_logged, [note.to_string()], "logged once");
    assert_eq!(run.graph_note.as_deref(), Some(note));
    assert!(
        !data.join(GRAPH_CACHE_FILE).exists(),
        "an unknown graph is never cached"
    );
    // A known graph notes nothing.
    let mut fresh = None;
    assert_eq!(
        note_once(&mut fresh, &GraphState::Known(Default::default())),
        None
    );
    assert_eq!(fresh, None);
}

fn spec() -> ConfineSpec {
    ConfineSpec {
        data_dir: PathBuf::from("/tmp/ax-data/runs/r"),
        common_dir: PathBuf::from("/tmp/ax-repo/.git"),
        cache_dirs: vec!["~/.cargo".into()],
        network: false,
        unix_sockets: Vec::new(),
        localhost_ports: Vec::new(),
        daemon_socket: PathBuf::from("/tmp/ax-sock"),
    }
}

fn ok(stdout: &str) -> (ShellOutcome, Option<String>) {
    let outcome = ShellOutcome {
        ok: true,
        code: Some(0),
        timed_out: false,
        tail: String::new(),
        secs: 0,
    };
    (outcome, Some(stdout.to_string()))
}

#[test]
fn toolchain_id_is_the_command_output_or_unknown() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    write(
        &dir.join("rustc.sh"),
        "echo 'rustc 1.92.0 (ded5c06cf 2025-12-08)'\n",
    );
    write(
        &dir.join("other.sh"),
        "echo 'rustc 1.93.0 (aaaaaaaaa 2026-01-22)'\n",
    );
    write(&dir.join("fails.sh"), "echo 'rustc 1.92.0'\nexit 1\n");
    write(&dir.join("slow.sh"), "sleep 30\n");
    let id = toolchain(dir, "sh rustc.sh", &[], None);
    let mut expected = Fnv1a64::new();
    expected.update(&0i32.to_le_bytes());
    expected.update(b"rustc 1.92.0 (ded5c06cf 2025-12-08)\n");
    assert_eq!(id, format!("{:016x}", expected.0));
    assert_eq!(toolchain(dir, "sh rustc.sh", &[], None), id, "stable");
    let other = toolchain(dir, "sh other.sh", &[], None);
    assert_ne!(other, id);
    assert_eq!(other.len(), 16);
    assert_eq!(toolchain(dir, "sh fails.sh", &[], None), TOOLCHAIN_UNKNOWN);
    let slow = toolchain_with(
        dir,
        "sh slow.sh",
        &[],
        None,
        Duration::from_secs(1),
        &capture,
    );
    assert_eq!(slow, TOOLCHAIN_UNKNOWN, "a timeout is unknown");

    // Confinement, the bound and the command reach the runner as given.
    let seen = RefCell::new(Vec::new());
    let stub = |d: &Path,
                command: &str,
                env: &[(String, String)],
                timeout: Duration,
                confine: Option<&ConfineSpec>| {
        seen.borrow_mut().push((
            d.to_path_buf(),
            command.to_string(),
            env.to_vec(),
            timeout,
            confine.cloned(),
        ));
        ok("rustc 1.92.0 (ded5c06cf 2025-12-08)\n")
    };
    let env = vec![("RUSTUP_TOOLCHAIN".to_string(), "stable".to_string())];
    let spec = spec();
    let stubbed = toolchain_with(dir, "rustc -V", &env, Some(&spec), TOOLCHAIN_TIMEOUT, &stub);
    assert_eq!(stubbed, id, "the id is the output's, whoever ran it");
    assert_eq!(
        seen.borrow().as_slice(),
        [(
            dir.to_path_buf(),
            "rustc -V".to_string(),
            env.clone(),
            TOOLCHAIN_TIMEOUT,
            Some(spec.clone())
        )]
    );
    // Output that could not be read is unknown too.
    let unread =
        |_: &Path, _: &str, _: &[(String, String)], _: Duration, _: Option<&ConfineSpec>| {
            (ok("").0, None)
        };
    assert_eq!(
        toolchain_with(dir, "rustc -V", &[], None, TOOLCHAIN_TIMEOUT, &unread),
        TOOLCHAIN_UNKNOWN
    );

    // The graph command is confined the same way.
    seen.borrow_mut().clear();
    let graph = |d: &Path,
                 command: &str,
                 env: &[(String, String)],
                 timeout: Duration,
                 confine: Option<&ConfineSpec>| {
        stub(d, command, env, timeout, confine);
        ok("{\"a\":[]}")
    };
    let state = module_graph_with(
        OsStr::new(GIT),
        dir,
        &dir.join("data"),
        &command_tiers("sh graph.sh"),
        (&[], &[]),
        &env,
        Some(&spec),
        GRAPH_TIMEOUT,
        &graph,
    );
    assert!(matches!(state, GraphState::Known(_)), "{state:?}");
    let calls = seen.borrow();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].1, "sh graph.sh");
    assert_eq!(calls[0].3, GRAPH_TIMEOUT);
    assert_eq!(calls[0].4, Some(spec));
}
