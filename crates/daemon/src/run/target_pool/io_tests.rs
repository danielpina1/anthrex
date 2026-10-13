//! The pool's I/O with real files and real `cargo` (M9.14.5). Each test has its own
//! temporary data directory; `ConfineSpec` is `None` (the confined release is M9.14.6's,
//! C5). A test that runs `cargo` runs it only on a tiny temporary workspace with no
//! registry dependency, `--offline`, with `CARGO_TARGET_DIR` set explicitly.
//!
//! Timing: no wall-clock bound here. Each `cargo` test builds a two-crate workspace a
//! few times (measured about 1–3 s each on the implementer's Mac; see the task report).

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{Duration, Instant, SystemTime};

use super::*;

/// The rig: a temporary root holding the data directory, the run's checkouts directory
/// and the repository root, all spelled resolved (macOS `/var` → `/private/var`).
pub(super) struct Rig {
    /// Takes the task temporary directories a confined release makes (M9.12.13).
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub tmp: crate::run::git::TestDir,
    pub root: PathBuf,
    pub data: PathBuf,
    pub ctx: PoolCtx,
}

impl Rig {
    pub fn new() -> Self {
        let tmp = crate::run::git::TestDir::new();
        let root = tmp.path().canonicalize().unwrap();
        let data = root.join("data");
        let runs_dir = root.join("wt/runs/r1");
        std::fs::create_dir_all(&runs_dir).unwrap();
        std::fs::create_dir_all(root.join("repo")).unwrap();
        let ctx = PoolCtx {
            run_id: "r1".to_string(),
            repo_root: root.join("repo"),
            repo_dir: data.join("repos/repo-1"),
            integration: runs_dir.join("integration"),
            full: runs_dir.join(".full"),
            runs_dir,
        };
        Rig {
            tmp,
            root,
            data,
            ctx,
        }
    }

    /// A task checkout `<runs_dir>/<name>`, created.
    pub fn checkout(&self, name: &str) -> PathBuf {
        let dir = self.ctx.runs_dir.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    pub fn pool_root(&self) -> PathBuf {
        self.ctx.repo_dir.join(POOL_DIR_NAME)
    }

    pub fn pool_dir(&self, name: &str) -> PathBuf {
        self.pool_root().join(name)
    }

    pub fn leases(&self) -> LeasesFile {
        let bytes = std::fs::read(self.pool_root().join(POOL_LEASES_FILE)).unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    pub fn write_leases(&self, text: &str) {
        std::fs::create_dir_all(self.pool_root()).unwrap();
        std::fs::write(self.pool_root().join(POOL_LEASES_FILE), text).unwrap();
    }

    pub fn state(&self, pool: &TargetPool, id: DirId) -> Option<DirState> {
        pool.book(&self.ctx.repo_dir)
            .and_then(|book| book.dir(id).map(|d| d.state))
    }
}

/// A pool of `task_dirs` task directories and one engine directory.
pub(super) fn pool(task_dirs: u16) -> TargetPool {
    let config = config::TargetPool {
        task_dirs: Some(task_dirs),
        engine_dirs: 1,
        ..config::TargetPool::default()
    };
    TargetPool::new(&config, 3)
}

/// The env an op carries for its checkout: it sets `CARGO_TARGET_DIR`.
pub(super) fn env() -> Vec<(String, String)> {
    vec![(TARGET_DIR_VAR.to_string(), "/profile/target".to_string())]
}

pub(super) fn task(k: u16) -> DirId {
    DirId {
        class: PoolClass::Task,
        k,
    }
}

pub(super) fn is_empty(dir: &Path) -> bool {
    std::fs::read_dir(dir).unwrap().next().is_none()
}

/// Writes something into `dir`, standing for a build's artifacts.
pub(super) fn plant(dir: &Path) {
    std::fs::create_dir_all(dir.join("debug/.fingerprint/app-1")).unwrap();
    std::fs::write(dir.join("debug/app"), "binary").unwrap();
}

/// Waits, up to a deadline, for `done`. The 10 s deadline only bounds a failure: what it
/// waits for (the background unlink of a trashed link, `disk::remove`) has no timeout
/// of its own and takes milliseconds; no constant of the code is involved.
pub(super) fn eventually(what: &str, done: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// The F7 workspace (`exclude = ["vend"]`): `app` prints `<word> vend-<word>`.
fn write_ws(dir: &Path, word: &str) {
    let files = [
        (
            "Cargo.toml",
            "[workspace]\nmembers = [\"app\"]\nexclude = [\"vend\"]\nresolver = \"2\"\n"
                .to_string(),
        ),
        (
            "app/Cargo.toml",
            "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n\
             [dependencies]\nvend = { path = \"../vend\" }\n"
                .to_string(),
        ),
        (
            "app/src/main.rs",
            format!("fn main() {{ println!(\"{{}} {{}}\", \"{word}\", vend::v()); }}\n"),
        ),
        (
            "vend/Cargo.toml",
            "[package]\nname = \"vend\"\nversion = \"0.1.0\"\nedition = \"2021\"\n".to_string(),
        ),
        (
            "vend/src/lib.rs",
            format!("pub fn v() -> &'static str {{ \"vend-{word}\" }}\n"),
        ),
    ];
    for (path, text) in files {
        let path = dir.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
}

fn cargo(dir: &Path, target: &Path, args: &[&str]) -> Output {
    let output = Command::new("cargo")
        .args(args)
        .current_dir(dir)
        .env(TARGET_DIR_VAR, target)
        .env_remove("RUSTC_WRAPPER")
        .env_remove("RUSTC_WORKSPACE_WRAPPER")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "cargo {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

async fn cargo_in(dir: &Path, target: &Path, args: &'static [&'static str]) -> Output {
    let (dir, target) = (dir.to_path_buf(), target.to_path_buf());
    tokio::task::spawn_blocking(move || cargo(&dir, &target, args))
        .await
        .unwrap()
}

/// Copy A: a workspace with its lock file, built into its leased directory.
async fn built_a(rig: &Rig, pool: &TargetPool) -> (PathBuf, PathBuf) {
    let a = rig.checkout("t1");
    write_ws(&a, "first");
    let dir = pool.lease(&rig.ctx, &a, &env()).await.expect("A leases");
    assert_eq!(dir, rig.pool_dir("task-1"));
    cargo_in(&a, &dir, &["build", "--offline", "-q"]).await;
    (a, dir)
}

/// Sets every file's mtime under `dir` to `secs` seconds ago.
fn age(dir: &Path, secs: u64) {
    let when = SystemTime::now() - Duration::from_secs(secs);
    let mut stack = vec![dir.to_path_buf()];
    while let Some(next) = stack.pop() {
        for entry in std::fs::read_dir(next).unwrap().flatten() {
            if entry.file_type().unwrap().is_dir() {
                stack.push(entry.path());
            } else {
                let file = std::fs::File::options()
                    .write(true)
                    .open(entry.path())
                    .unwrap();
                file.set_modified(when).unwrap();
            }
        }
    }
}

fn fingerprints(dir: &Path, prefix: &str) -> Vec<String> {
    std::fs::read_dir(dir.join("debug/.fingerprint"))
        .unwrap()
        .flatten()
        .filter_map(|e| e.file_name().to_str().map(str::to_string))
        .filter(|name| name.starts_with(prefix))
        .collect()
}

#[tokio::test]
async fn a_release_cleans_path_packages_and_a_new_owner_runs_its_own_code() {
    let rig = Rig::new();
    let pool = pool(1);
    let (a, dir) = built_a(&rig, &pool).await;
    // Stands in for a registry dependency, which an offline test cannot fetch.
    let regdep = dir.join("debug/.fingerprint/regdep-0123456789abcdef");
    std::fs::create_dir_all(&regdep).unwrap();

    assert_eq!(
        pool.release(&rig.ctx, None, &a, true).await,
        ReleaseOutcome::Cleaned
    );

    let b = rig.checkout("t2");
    write_ws(&b, "second");
    std::fs::copy(a.join("Cargo.lock"), b.join("Cargo.lock")).unwrap();
    age(&b, 3600);
    let dir_b = pool.lease(&rig.ctx, &b, &env()).await.expect("B leases");
    assert_eq!(dir_b, dir);
    let out = cargo_in(&b, &dir_b, &["run", "--offline", "-q"]).await;
    assert_eq!(String::from_utf8_lossy(&out.stdout), "second vend-second\n");
    assert!(
        regdep.is_dir(),
        "a registry dependency's artifacts are kept"
    );
}

#[tokio::test]
async fn a_vendored_path_crate_is_cleaned() {
    let rig = Rig::new();
    let pool = pool(1);
    let (a, dir) = built_a(&rig, &pool).await;
    assert!(!fingerprints(&dir, "vend-").is_empty());

    assert_eq!(
        pool.release(&rig.ctx, None, &a, true).await,
        ReleaseOutcome::Cleaned
    );
    assert_eq!(fingerprints(&dir, "vend-"), Vec::<String>::new());
    assert_eq!(fingerprints(&dir, "app-"), Vec::<String>::new());
    assert_eq!(rig.state(&pool, task(1)), Some(DirState::Clean));
}

#[tokio::test]
async fn a_failed_clean_wipes_the_dir() {
    let rig = Rig::new();
    let pool = pool(1);
    let (a, dir) = built_a(&rig, &pool).await;
    std::fs::write(a.join("Cargo.toml"), "[workspace\nnot toml").unwrap();

    let outcome = pool.release(&rig.ctx, None, &a, true).await;
    let ReleaseOutcome::Wiped(reason) = outcome else {
        panic!("expected a wipe, got {outcome:?}");
    };
    assert!(
        reason.starts_with("cargo metadata failed: "),
        "reason: {reason}"
    );
    assert!(is_empty(&dir));
    assert_eq!(rig.state(&pool, task(1)), Some(DirState::Fresh));
}

#[tokio::test]
async fn a_dir_over_the_cap_is_wiped_not_cleaned() {
    let rig = Rig::new();
    let pool = pool(1).with_cap_bytes(1024);
    let (a, dir) = built_a(&rig, &pool).await;

    assert_eq!(
        pool.release(&rig.ctx, None, &a, true).await,
        ReleaseOutcome::Wiped("over the size cap".to_string())
    );
    assert!(is_empty(&dir));
    assert_eq!(rig.state(&pool, task(1)), Some(DirState::Fresh));
}

#[tokio::test]
async fn the_clean_ignores_a_planted_rustc_wrapper() {
    let rig = Rig::new();
    let pool = pool(1);
    let (a, _dir) = built_a(&rig, &pool).await;
    let marker = rig.root.join("wrapper-ran");
    let wrapper = rig.root.join("wrapper.sh");
    testexec::write_executable(
        &wrapper,
        format!("#!/bin/sh\ntouch '{}'\nexec \"$@\"\n", marker.display()),
    );
    std::fs::create_dir_all(a.join(".cargo")).unwrap();
    std::fs::write(
        a.join(".cargo/config.toml"),
        format!("[build]\nrustc-wrapper = \"{}\"\n", wrapper.display()),
    )
    .unwrap();

    assert_eq!(
        pool.release(&rig.ctx, None, &a, true).await,
        ReleaseOutcome::Cleaned
    );
    assert!(!marker.exists(), "the planted rustc wrapper ran");
}

/// C5: a release under the run's `ConfineSpec` runs its `cargo metadata` and `cargo
/// clean` confined, with the pool directory granted on the release's own copy
/// (`target_dir`), so it cleans; without the grant `(deny default)` refuses the
/// clean's writes and every confined release would wipe.
#[cfg(target_os = "macos")]
#[tokio::test]
async fn a_confined_release_cleans_and_does_not_wipe() {
    let rig = Rig::new();
    let pool = pool(1);
    let (a, dir) = built_a(&rig, &pool).await;
    let data_dir = rig.data.join("runs/r1");
    rig.tmp
        .also(crate::run::git::checkout_repo_dir(&data_dir, &a));
    let common = rig.root.join("repo/.git");
    std::fs::create_dir_all(&common).unwrap();
    let spec = crate::run::confine::ConfineSpec {
        data_dir,
        common_dir: common,
        cache_dirs: Vec::new(),
        network: false,
        unix_sockets: Vec::new(),
        localhost_ports: Vec::new(),
        daemon_socket: rig.root.join("sock/daemon.sock"),
        target_dir: None,
    };

    assert_eq!(
        pool.release(&rig.ctx, Some(&spec), &a, true).await,
        ReleaseOutcome::Cleaned
    );
    assert_eq!(fingerprints(&dir, "app-"), Vec::<String>::new());
    assert_eq!(rig.state(&pool, task(1)), Some(DirState::Clean));
}
