//! M8a final fix batch F1, fix round 5 (post-breaker): a worker process can outlive
//! its session (a `setsid`'d child, say) and keep writing its grant while the engine
//! hands the run head back, so every check the engine makes before a git command can
//! be raced. Here a real process, spawned by the test and killed by its exact pid,
//! keeps pointing the task worktree's `HEAD`, its `ORIG_HEAD` and the task's branch at
//! the base while hand-backs and aborts run, clean and conflicted. Whatever each of
//! them returns, the base, the run branch and every other ref must never move: no
//! engine command that writes a ref or a pseudo-ref runs in the worker's git dir.

mod support;

use daemon::run::git::{abort_merge, create_run_branch, hand_back, prepare_worktree};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use support::run_git::{T, commit_file, out, real_git, repo, wt_dir};

/// How long the racer may take to make its first round of writes: one `sh` start-up,
/// milliseconds even under a loaded test run (`docs/timing-budgets.md`).
const RACER_START: Duration = Duration::from_secs(30);

/// Hand-backs attempted while the racer runs; every third conflicts.
const ROUNDS: usize = 36;

/// The racer: shell builtins only (`printf`, `read`, `:`), so the one process the test
/// kills is the only one there is. Each pass points `HEAD`, `ORIG_HEAD` and the task's
/// branch file at `refs/heads/main`, then puts back what a worker's git would leave:
/// `HEAD` on the task's branch, a plain `ORIG_HEAD`, the branch's own commit. The
/// first pass creates `$4`.
const RACER: &str = r#"admin=$1 own_file=$2 own=$3 started=$4
spin() { i=0; while [ $i -lt $1 ]; do i=$((i+1)); done; }
while :; do
  cur=
  read -r cur < "$own_file" || :
  printf 'ref: refs/heads/main\n' > "$admin/HEAD"
  printf 'ref: refs/heads/main\n' > "$admin/ORIG_HEAD"
  printf 'ref: refs/heads/main\n' > "$own_file"
  spin 150
  case "$cur" in
    ''|ref:*) ;;
    *) printf '%s\n' "$cur" > "$own_file"
       printf '%s\n' "$cur" > "$admin/ORIG_HEAD" ;;
  esac
  printf 'ref: %s\n' "$own" > "$admin/HEAD"
  [ -e "$started" ] || : > "$started"
  spin 4000
done"#;

struct Racer(Child);

impl Drop for Racer {
    /// Killed by its own pid (`Child::kill`), and reaped.
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn start_racer(admin: &Path, own_file: &Path, own: &str, started: &Path) -> Racer {
    let child = Command::new("/bin/sh")
        .arg("-c")
        .arg(RACER)
        .arg("racer")
        .arg(admin)
        .arg(own_file)
        .arg(own)
        .arg(started)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let racer = Racer(child);
    let deadline = Instant::now() + RACER_START;
    while !started.exists() {
        assert!(Instant::now() < deadline, "the racer never started");
        std::thread::sleep(Duration::from_millis(5));
    }
    racer
}

fn refs(root: &Path, own: &str) -> String {
    out(root, &["for-each-ref", "--format=%(refname) %(objectname)"])
        .lines()
        .filter(|line| !line.starts_with(&format!("{own} ")))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn a_live_worker_racing_hand_backs_never_moves_the_base() {
    let repo = repo();
    let base = commit_file(&repo.root, "f.txt", "base\n", "base");
    let (_wt, wt_path) = wt_dir();
    let run = "lr01";
    let integration = wt_path.join(format!("runs/{run}/integration"));
    create_run_branch(
        real_git(),
        &repo.root,
        &format!("anthrex/{run}/integration"),
        &base,
        &integration,
        T,
    )
    .unwrap();
    let task = wt_path.join(format!("runs/{run}/t1"));
    let own = format!("refs/heads/anthrex/{run}/t1");
    prepare_worktree(
        real_git(),
        &repo.root,
        &format!("anthrex/{run}/t1"),
        &base,
        &task,
        T,
    )
    .unwrap();
    commit_file(&task, "f.txt", "task\n", "task edit");
    let admin = PathBuf::from(out(&task, &["rev-parse", "--absolute-git-dir"]));
    let common = PathBuf::from(out(
        &repo.root,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    ));
    let own_file = common.join(&own);
    // Every run head is made before the race, so the test's own git never meets it.
    let mut heads = Vec::new();
    for round in 0..ROUNDS {
        let (file, content) = if round % 3 == 2 {
            ("f.txt".to_string(), format!("run {round}\n"))
        } else {
            (format!("r{round}.txt"), "run\n".to_string())
        };
        heads.push(commit_file(&integration, &file, &content, "run work"));
    }
    let before = refs(&repo.root, &own);
    let tools = tempfile::tempdir().unwrap();

    let mut outcomes = Vec::new();
    {
        let _racer = start_racer(&admin, &own_file, &own, &tools.path().join("started"));
        for run_head in &heads {
            outcomes.push(format!("{:?}", hand_back(real_git(), &task, run_head, T)));
            let _ = abort_merge(real_git(), &task, T);
            let main = out(&repo.root, &["rev-parse", "main"]);
            assert_eq!(main, base, "the base moved; outcomes so far: {outcomes:#?}");
        }
    }

    assert_eq!(out(&repo.root, &["rev-parse", "main"]), base);
    assert_eq!(
        refs(&repo.root, &own),
        before,
        "a ref other than the task's own moved: {outcomes:#?}"
    );
    eprintln!("{outcomes:#?}");
}
