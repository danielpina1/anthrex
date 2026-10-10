//! Milestone 8b decisions 4 and 7: the profile store in anthrex's data directory, on a
//! real repository (M8b.4).

mod support;

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::path::Path;
use std::time::Duration;

use daemon::profile::store::{
    self, FINGERPRINT_MAX_BYTES, Stored, delete_proposal, fingerprint, load, load_proposal, save,
    save_proposal, stale,
};
use proto::{ProfileMeta, ProposalOrigin, ProposalRecord, ProposalState, RepoProfile};
use support::git_output;
use support::run_git::repo as identified_repo;

const TIMEOUT: Duration = Duration::from_secs(10);

fn profile() -> RepoProfile {
    let mut env = BTreeMap::new();
    env.insert(
        "CARGO_TARGET_DIR".to_string(),
        "{worktree}/target".to_string(),
    );
    RepoProfile {
        languages: vec!["rust".into()],
        check: Some("cargo test".into()),
        single_test: Some("cargo test -- --exact {test}".into()),
        generated: vec!["Cargo.lock".into()],
        protected: vec![".cursor/**".into()],
        manifests: vec!["Cargo.toml".into()],
        env,
        ..Default::default()
    }
}

fn meta(fingerprint: BTreeMap<String, String>) -> ProfileMeta {
    ProfileMeta {
        confirmed_at: 1_700_000_000,
        report: Some("onboarding-1700000000".into()),
        verification: None,
        fingerprint,
        edited_keys: Vec::new(),
        project: None,
    }
}

#[test]
fn repo_dir_is_shared_by_linked_worktrees() {
    let repo = identified_repo();
    let linked = tempfile::tempdir().unwrap();
    let linked_path = linked.path().join("wt");
    repo.git(&[
        OsStr::new("worktree"),
        OsStr::new("add"),
        OsStr::new("-b"),
        OsStr::new("other"),
        linked_path.as_os_str(),
    ]);
    let git = OsStr::new("git");
    let main = daemon::run::git::preflight(git, &repo.root, TIMEOUT).unwrap();
    let other = daemon::run::git::preflight(git, &linked_path, TIMEOUT).unwrap();
    let data = Path::new("/tmp/ax-data");
    let dir = daemon::profile::repo_dir(data, &main.project);
    assert_eq!(dir, daemon::profile::repo_dir(data, &other.project));
    let basename = main.project.file_name().unwrap().to_string_lossy();
    assert_eq!(
        dir,
        data.join("repos").join(format!(
            "{basename}-{}",
            daemon::worktree::hash8(&main.project)
        ))
    );
    repo.git(&[
        OsStr::new("worktree"),
        OsStr::new("remove"),
        OsStr::new("--force"),
        linked_path.as_os_str(),
    ]);
}

/// Every temp file in `dir`, by name, sorted.
fn temps(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".tmp"))
        .collect();
    names.sort();
    names
}

#[test]
fn save_and_load_round_trip_atomically() {
    let data = tempfile::tempdir().unwrap();
    let dir = data.path().join("repos").join("r-00000000");
    std::fs::create_dir_all(&dir).unwrap();
    // Leftovers of crashed writes, in the old and the per-write naming.
    let leftovers = [
        "profile.toml.1.0.tmp",
        "profile.toml.tmp",
        "proposal.json.7.3.tmp",
    ];
    for name in leftovers {
        std::fs::write(dir.join(name), "garbage = [").unwrap();
    }
    let written = meta(BTreeMap::new());
    save(&dir, &profile(), &written).unwrap();
    assert_eq!(
        temps(&dir),
        leftovers,
        "a save leaves no temp file of its own"
    );

    // A read ignores the leftovers and never removes them: a concurrent writer's temp
    // file looks exactly like one.
    match load(&dir) {
        Stored::Found {
            profile: p,
            meta: m,
            path,
        } => {
            assert_eq!(p, profile());
            assert_eq!(m, written);
            assert_eq!(path, dir.join("profile.toml"));
        }
        other => panic!("expected the stored profile, got {other:?}"),
    }
    assert_eq!(load_proposal(&dir), Ok(None));
    assert_eq!(temps(&dir), leftovers, "a read removes nothing");

    // The sweep, run where no writer can be (daemon start), removes them all.
    store::sweep_leftovers(&dir).unwrap();
    assert!(temps(&dir).is_empty());
    assert!(matches!(load(&dir), Stored::Found { .. }));
    assert!(matches!(load(&data.path().join("nothing")), Stored::Absent));
    store::sweep_leftovers(&data.path().join("nothing")).unwrap();
}

#[test]
fn saves_succeed_while_other_threads_load() {
    let data = tempfile::tempdir().unwrap();
    let dir = data.path().join("repos").join("r-00000000");
    let versions: Vec<RepoProfile> = (0..200)
        .map(|i| RepoProfile {
            check: Some(format!("check {i}")),
            ..profile()
        })
        .collect();
    let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let readers: Vec<_> = (0..2)
        .map(|_| {
            let (dir, done, versions) = (dir.clone(), done.clone(), versions.clone());
            std::thread::spawn(move || {
                let mut found = 0u32;
                while !done.load(std::sync::atomic::Ordering::SeqCst) {
                    match load(&dir) {
                        Stored::Found { profile, .. } => {
                            assert!(versions.contains(&profile), "{profile:?}");
                            found += 1;
                        }
                        Stored::Absent => {}
                        other => panic!("a load during saves gave {other:?}"),
                    }
                    let proposal = load_proposal(&dir);
                    assert!(proposal.is_ok(), "{proposal:?}");
                }
                found
            })
        })
        .collect();
    // Two writers on the same files, as `run start`'s read and `profile confirm` or
    // detection's proposal writes are never serialised with each other.
    let writers: Vec<_> = (0..2)
        .map(|w| {
            let (dir, versions) = (dir.clone(), versions.clone());
            std::thread::spawn(move || {
                for (i, version) in versions.iter().enumerate().filter(|(i, _)| i % 2 == w) {
                    save(&dir, version, &meta(BTreeMap::new()))
                        .unwrap_or_else(|e| panic!("save {i} failed: {e}"));
                    save_proposal(&dir, &proposal(Path::new("/src/p")))
                        .unwrap_or_else(|e| panic!("proposal save {i} failed: {e}"));
                }
            })
        })
        .collect();
    for writer in writers {
        writer.join().unwrap();
    }
    done.store(true, std::sync::atomic::Ordering::SeqCst);
    for reader in readers {
        reader.join().unwrap();
    }
    assert!(matches!(load(&dir), Stored::Found { .. }));
    assert!(temps(&dir).is_empty(), "{:?}", temps(&dir));
}

#[test]
fn a_missing_meta_reads_as_confirmed_at_zero_and_never_stale() {
    let data = tempfile::tempdir().unwrap();
    let dir = data.path().to_path_buf();
    std::fs::write(dir.join("profile.toml"), "check = \"cargo test\"\n").unwrap();
    match load(&dir) {
        Stored::Found { profile, meta, .. } => {
            assert_eq!(profile.check.as_deref(), Some("cargo test"));
            assert_eq!(meta.confirmed_at, 0);
            assert_eq!(meta.report, None);
            assert!(meta.fingerprint.is_empty());
            assert!(stale(data.path(), &meta).is_empty());
        }
        other => panic!("expected Found, got {other:?}"),
    }
    // A meta file without a profile is no stored profile.
    let alone = tempfile::tempdir().unwrap();
    std::fs::write(alone.path().join("profile.meta.json"), "{}").unwrap();
    assert!(matches!(load(alone.path()), Stored::Absent));
}

#[test]
fn a_corrupt_meta_is_unparseable_naming_the_meta_file() {
    let data = tempfile::tempdir().unwrap();
    let dir = data.path().to_path_buf();
    std::fs::write(dir.join("profile.toml"), "check = \"cargo test\"\n").unwrap();
    std::fs::write(dir.join("profile.meta.json"), "{").unwrap();
    match load(&dir) {
        Stored::Unparseable { path, error } => {
            assert_eq!(path, dir.join("profile.meta.json"));
            assert!(error.contains("EOF"), "{error}");
        }
        other => panic!("expected Unparseable, got {other:?}"),
    }
}

#[test]
fn an_unparseable_profile_is_reported_with_its_path() {
    let data = tempfile::tempdir().unwrap();
    let dir = data.path().to_path_buf();
    std::fs::write(dir.join("profile.toml"), "check = [").unwrap();
    match load(&dir) {
        Stored::Unparseable { path, error } => {
            assert_eq!(path, dir.join("profile.toml"));
            assert!(!error.is_empty());
        }
        other => panic!("expected Unparseable, got {other:?}"),
    }
}

#[test]
fn a_profile_with_a_confinement_key_does_not_parse() {
    let data = tempfile::tempdir().unwrap();
    let dir = data.path().to_path_buf();
    std::fs::write(
        dir.join("profile.toml"),
        "check = \"cargo test\"\ncache_dirs = [\"/\"]\n",
    )
    .unwrap();
    match load(&dir) {
        Stored::Unparseable { path, error } => {
            assert_eq!(path, dir.join("profile.toml"));
            assert!(error.contains("cache_dirs"), "{error}");
        }
        other => panic!("expected Unparseable, got {other:?}"),
    }
}

#[test]
fn fingerprint_changes_with_content_length_and_absence() {
    let project = tempfile::tempdir().unwrap();
    let file = project.path().join("Cargo.toml");
    let paths = vec!["Cargo.toml".to_string()];
    std::fs::write(&file, "").unwrap();
    let empty = fingerprint(project.path(), &paths);
    // FNV-1a 64 of no bytes is its offset basis.
    assert_eq!(empty["Cargo.toml"], "cbf29ce484222325:0");

    std::fs::write(&file, "abc").unwrap();
    let first = fingerprint(project.path(), &paths)["Cargo.toml"].clone();
    assert!(first.ends_with(":3"), "{first}");
    std::fs::write(&file, "abd").unwrap();
    let same_length = fingerprint(project.path(), &paths)["Cargo.toml"].clone();
    assert_ne!(first, same_length);
    assert!(same_length.ends_with(":3"));
    std::fs::write(&file, "abcd").unwrap();
    let longer = fingerprint(project.path(), &paths)["Cargo.toml"].clone();
    assert!(longer.ends_with(":4"), "{longer}");
    std::fs::remove_file(&file).unwrap();
    assert_eq!(fingerprint(project.path(), &paths)["Cargo.toml"], "missing");

    // Only the first 4 MiB is hashed; the full length is still recorded.
    let big = vec![b'x'; FINGERPRINT_MAX_BYTES as usize + 10];
    std::fs::write(&file, &big).unwrap();
    let a = fingerprint(project.path(), &paths)["Cargo.toml"].clone();
    let mut changed_tail = big.clone();
    *changed_tail.last_mut().unwrap() = b'y';
    std::fs::write(&file, &changed_tail).unwrap();
    let b = fingerprint(project.path(), &paths)["Cargo.toml"].clone();
    assert_eq!(a, b);
    assert!(a.ends_with(&format!(":{}", big.len())), "{a}");

    // A path outside the project is never read.
    let outside = vec!["../escape".to_string(), "/etc/hosts".to_string()];
    let fp = fingerprint(project.path(), &outside);
    assert_eq!(fp["../escape"], "missing");
    assert_eq!(fp["/etc/hosts"], "missing");
}

#[test]
fn fingerprint_never_follows_a_symlink_out_of_the_project() {
    let project = tempfile::tempdir().unwrap();
    let elsewhere = tempfile::tempdir().unwrap();
    let secret = elsewhere.path().join("secret");
    std::fs::write(&secret, "abc").unwrap();
    std::fs::write(project.path().join("real"), "abc").unwrap();
    let secret_hash = fingerprint(project.path(), &["real".to_string()])["real"].clone();
    std::os::unix::fs::symlink(&secret, project.path().join("Cargo.toml")).unwrap();
    std::os::unix::fs::symlink(elsewhere.path(), project.path().join("sub")).unwrap();
    let paths = vec!["Cargo.toml".to_string(), "sub/secret".to_string()];
    let fp = fingerprint(project.path(), &paths);
    // A symlink is fingerprinted by its target's name, never its target's contents.
    let target = secret.as_os_str().len();
    assert_ne!(fp["Cargo.toml"], secret_hash);
    assert!(
        fp["Cargo.toml"].starts_with("link:"),
        "{}",
        fp["Cargo.toml"]
    );
    assert!(
        fp["Cargo.toml"].ends_with(&format!(":{target}")),
        "{}",
        fp["Cargo.toml"]
    );
    // A path through a symlinked directory is not read at all.
    assert_eq!(fp["sub/secret"], "missing");

    // Retargeting the link changes the fingerprint, so staleness still sees it.
    let before = fp["Cargo.toml"].clone();
    std::fs::remove_file(project.path().join("Cargo.toml")).unwrap();
    std::os::unix::fs::symlink("real", project.path().join("Cargo.toml")).unwrap();
    let after = fingerprint(project.path(), &paths)["Cargo.toml"].clone();
    assert_ne!(before, after);
    assert!(after.ends_with(":4"), "{after}");
}

#[test]
fn stale_lists_exactly_the_changed_files() {
    let project = tempfile::tempdir().unwrap();
    for name in ["a", "b", "c"] {
        std::fs::write(project.path().join(name), name).unwrap();
    }
    let paths: Vec<String> = ["a", "b", "c", "d"].iter().map(|s| s.to_string()).collect();
    let m = meta(fingerprint(project.path(), &paths));
    assert!(stale(project.path(), &m).is_empty());
    std::fs::write(project.path().join("b"), "changed").unwrap();
    std::fs::remove_file(project.path().join("c")).unwrap();
    assert_eq!(stale(project.path(), &m), vec!["b", "c"]);
    std::fs::write(project.path().join("d"), "new").unwrap();
    assert_eq!(stale(project.path(), &m), vec!["b", "c", "d"]);
}

fn proposal(project: &Path) -> ProposalRecord {
    ProposalRecord {
        project: project.to_path_buf(),
        state: ProposalState::Failed {
            reason: "the daemon restarted during detection; run anthrex profile detect".into(),
        },
        origin: ProposalOrigin::Edit {
            keys: vec!["check".into()],
        },
        started_at: 1,
        updated_at: 2,
        base_sha: "b".repeat(40),
        scout_id: None,
        window_id: None,
        profile: Some(profile()),
        verification: None,
        dropped: Vec::new(),
        proposed: Some(profile()),
        trusted_project: Vec::new(),
        unconfined_checks: false,
        auto_confirm: true,
        edit: None,
    }
}

#[test]
fn a_proposal_round_trips_and_is_deleted() {
    let data = tempfile::tempdir().unwrap();
    let dir = data.path().join("repos").join("r-00000000");
    assert_eq!(load_proposal(&dir), Ok(None));
    let record = proposal(Path::new("/src/p"));
    save_proposal(&dir, &record).unwrap();
    assert_eq!(load_proposal(&dir), Ok(Some(record)));
    delete_proposal(&dir).unwrap();
    assert_eq!(load_proposal(&dir), Ok(None));
    delete_proposal(&dir).unwrap();
    std::fs::write(dir.join("proposal.json"), "{").unwrap();
    assert!(load_proposal(&dir).is_err());
}

#[test]
fn nothing_is_written_inside_the_repository() {
    let repo = identified_repo();
    let data = tempfile::tempdir().unwrap();
    let git = OsStr::new("git");
    let pre = daemon::run::git::preflight(git, &repo.root, TIMEOUT).unwrap();
    let dir = daemon::profile::repo_dir(data.path(), &pre.project);
    let m = meta(fingerprint(&pre.project, &["README".to_string()]));
    save(&dir, &profile(), &m).unwrap();
    save_proposal(&dir, &proposal(&pre.project)).unwrap();
    assert!(matches!(load(&dir), Stored::Found { .. }));
    assert!(stale(&pre.project, &m).is_empty());
    store::write_atomic(&dir.join("extra.json"), b"{}").unwrap();
    assert!(dir.starts_with(data.path()));
    let status = git_output(
        &repo.root,
        &[
            OsStr::new("status"),
            OsStr::new("--porcelain"),
            OsStr::new("--ignored"),
        ],
    );
    assert!(status.status.success());
    assert_eq!(String::from_utf8_lossy(&status.stdout), "");
}
