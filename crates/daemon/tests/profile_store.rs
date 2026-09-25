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
use support::{TempRepo, git_output};

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
    }
}

#[test]
fn repo_dir_is_shared_by_linked_worktrees() {
    let repo = TempRepo::new();
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

#[test]
fn save_and_load_round_trip_atomically() {
    let data = tempfile::tempdir().unwrap();
    let dir = data.path().join("repos").join("r-00000000");
    let tmp = dir.join("profile.toml.tmp");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(&tmp, "garbage = [").unwrap();
    let written = meta(BTreeMap::new());
    save(&dir, &profile(), &written).unwrap();
    assert!(!tmp.exists(), "the save's own temp file is renamed away");

    // A leftover from a crash between write and rename is ignored and removed.
    std::fs::write(&tmp, "garbage = [").unwrap();
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
    assert!(!tmp.exists(), "a leftover temp file is removed");
    assert!(matches!(load(&data.path().join("nothing")), Stored::Absent));
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
    let repo = TempRepo::new();
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
