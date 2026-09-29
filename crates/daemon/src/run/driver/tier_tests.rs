//! Milestone 9.1 task M9.1.9: the tier executor, against a temporary git repository of
//! its own (its own `user.name`/`user.email`), shell-script commands, a 2-slot
//! scheduler and no daemon. The scripts append one line per run to `$TIER_LOG` and read
//! what to print, and how to exit, from files in `$TIER_STATE`, so no test depends on
//! timing. Every command runs unconfined here (the executor's confinement is M8a's,
//! unchanged); `ctx.confine` is `None`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use proto::ModuleNames;

use super::*;
use crate::run::driver::OpCtx;
use crate::run::exec::summary;
use crate::run::tiers::{GraphSource, TierProfile};

const BUILD: &str = r#"echo "build slots=$ANTHREX_TEST_SLOTS load=$ANTHREX_TEST_LOAD" >> "$TIER_LOG"
if [ -f "$TIER_STATE/build-fail" ]; then echo "build broke"; exit 1; fi
exit 0
"#;

const TEST: &str = r#"state="$TIER_STATE"
if [ "${1:-}" = "--one" ]; then
  echo "one $2 slots=$ANTHREX_TEST_SLOTS" >> "$TIER_LOG"
  if [ -f "$state/one-fail" ]; then echo "FAIL $2"; exit 1; fi
  echo "PASS $2"
  exit 0
fi
m="${1:-}"
n=$(( $(cat "$state/count-$m" 2>/dev/null || echo 0) + 1 ))
echo "$n" > "$state/count-$m"
echo "test $m run=$n slots=$ANTHREX_TEST_SLOTS load=$ANTHREX_TEST_LOAD filter=${3:-}" >> "$TIER_LOG"
if [ -f "$state/sleep-$m-$n" ]; then sleep 30; fi
if [ -f "$state/out-$m-$n" ]; then cat "$state/out-$m-$n"; fi
if [ -f "$state/code-$m-$n" ]; then exit "$(cat "$state/code-$m-$n")"; fi
exit 0
"#;

const CHECK: &str = r#"echo "check args=$* pwd=$(pwd) slots=$ANTHREX_TEST_SLOTS load=$ANTHREX_TEST_LOAD foo=${FOO:-}" >> "$TIER_LOG"
if [ -d "$TMPDIR" ]; then echo "present=yes"; fi
echo "tmp=$TMPDIR"
echo "sock=$ANTHREX_SOCKET"
echo "data=$ANTHREX_DATA_DIR"
for v in CARGO_BUILD_JOBS NEXTEST_TEST_THREADS RUST_TEST_THREADS ANTHREX_TEST_SLOTS ANTHREX_TEST_LOAD; do
  eval "echo $v=\${$v:-}"
done
if [ -f "$TIER_STATE/check-fail" ]; then exit 1; fi
exit 0
"#;

const GRAPH: &str = r#"echo '{"a":[],"b":["a"],"c":["b"]}'
"#;

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_COMMON_DIR")
        .env_remove("GIT_PREFIX")
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {out:?}");
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

/// libtest's output for `failed` names out of `failed + 1` tests (M9.1.1's shape).
fn libtest(failed: &[&str]) -> String {
    let mut out = format!(
        "running {} tests\ntest tests::fine ... ok\n",
        failed.len() + 1
    );
    for name in failed {
        out.push_str(&format!("test {name} ... FAILED\n"));
    }
    out.push_str("\nfailures:\n\n");
    for name in failed {
        out.push_str(&format!(
            "---- {name} stdout ----\n\nthread '{name}' panicked at src/lib.rs:1:1:\nboom\n\n"
        ));
    }
    out.push_str("\nfailures:\n");
    for name in failed {
        out.push_str(&format!("    {name}\n"));
    }
    out.push_str(&format!(
        "\ntest result: FAILED. 1 passed; {} failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s\n",
        failed.len()
    ));
    out
}

/// A repository with three modules (`c` depends on `b`, `b` on `a`), the scripts, a
/// base commit and a second commit that changes `mods/b`.
struct Rig {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    data: PathBuf,
    dir: PathBuf,
    state: PathBuf,
    log: PathBuf,
    base: String,
    head: String,
}

impl Rig {
    fn new() -> Rig {
        let tmp = tempfile::tempdir().unwrap();
        let top = tmp.path().canonicalize().unwrap();
        let root = top.join("repo");
        std::fs::create_dir_all(&root).unwrap();
        git(&root, &["init", "-q", "-b", "main"]);
        git(&root, &["config", "user.name", "Tier Test"]);
        git(&root, &["config", "user.email", "tier@test"]);
        git(&root, &["config", "commit.gpgsign", "false"]);
        for (path, text) in [
            ("build.sh", BUILD),
            ("test.sh", TEST),
            ("check.sh", CHECK),
            ("graph.sh", GRAPH),
            ("mods/a/lib.txt", "a\n"),
            ("mods/b/lib.txt", "b\n"),
            ("mods/c/lib.txt", "c\n"),
            ("docs/readme.md", "docs\n"),
        ] {
            write(&root.join(path), text);
        }
        git(&root, &["add", "-A"]);
        git(&root, &["commit", "-q", "-m", "base"]);
        let base = git(&root, &["rev-parse", "HEAD"]);
        write(&root.join("mods/b/lib.txt"), "b changed\n");
        git(&root, &["commit", "-q", "-am", "change b"]);
        let head = git(&root, &["rev-parse", "HEAD"]);
        let state = top.join("state");
        std::fs::create_dir_all(&state).unwrap();
        Rig {
            data: top.join("data/runs/r1"),
            dir: top.join("wt/runs/r1/t1.proof"),
            log: top.join("tier-log"),
            state,
            root,
            base,
            head,
            _tmp: tmp,
        }
    }

    fn ctx(&self) -> OpCtx {
        OpCtx {
            run_id: "r1".to_string(),
            project: self.root.clone(),
            data_dir: self.data.clone(),
            git_timeout: Duration::from_secs(30),
            check_timeout: Duration::from_secs(60),
            confine: None,
        }
    }

    fn state(&self, name: &str, text: &str) {
        write(&self.state.join(name), text);
    }

    fn log(&self) -> Vec<String> {
        std::fs::read_to_string(&self.log)
            .unwrap_or_default()
            .lines()
            .map(str::to_string)
            .collect()
    }

    fn spec(&self, profile: TierProfile, check: Option<&str>) -> TierSpec {
        TierSpec {
            tier: 1,
            stage: 1,
            root: self.root.clone(),
            dir: self.dir.clone(),
            scratch: Some(ScratchAt {
                root: self.root.clone(),
                commit: self.head.clone(),
                setup: None,
            }),
            diff_base: self.base.clone(),
            head: self.head.clone(),
            profile,
            check: check.map(str::to_string),
            hub: Vec::new(),
            source: vec!["mods/**".to_string()],
            modules: vec!["mods/*".to_string()],
            manifests: vec!["graph.sh".to_string()],
            single_test: Some("sh test.sh --one {test}".to_string()),
            timeout_secs: 60,
            env: vec![
                ("TIER_LOG".to_string(), self.log.display().to_string()),
                ("TIER_STATE".to_string(), self.state.display().to_string()),
                ("FOO".to_string(), "bar".to_string()),
            ],
            priority: Priority::Gate,
            critical: false,
            cache: None,
            toolchain: Some("none".to_string()),
            repo_dir: self.data.join("repo-data"),
        }
    }

    async fn tier(&self, op: OpId, spec: &TierSpec) -> TierOutcome {
        let (sched, queue) = (TestScheduler::new(2), GitQueue::new());
        match run_tier(&self.ctx(), &sched, &queue, OsStr::new("git"), op, spec).await {
            OpResult::Tier(outcome) => *outcome,
            other => panic!("not a tier outcome: {other:?}"),
        }
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        let repo = git::checkout_repo_dir(&self.data, &self.dir);
        let _ = git::remove_task_tmp(&repo);
    }
}

/// The tiered profile of the shared helpers: a graph command, `build_check`, a
/// per-module test command with a filter slot.
fn tiered() -> TierProfile {
    TierProfile {
        build_check: Some("sh build.sh".to_string()),
        module_test: Some("sh test.sh {module} {filter:--filter %}".to_string()),
        module_graph: GraphSource::Command("sh graph.sh".to_string()),
        module_names: ModuleNames::Dir,
        ..TierProfile::default()
    }
}

fn value<'a>(line: &'a str, key: &str) -> &'a str {
    let at = line
        .find(&format!("{key}="))
        .unwrap_or_else(|| panic!("no {key} in {line}"));
    let rest = &line[at + key.len() + 1..];
    rest.split(' ').next().unwrap_or("")
}

fn tail_value<'a>(tail: &'a str, key: &str) -> &'a str {
    tail.lines()
        .find_map(|l| l.strip_prefix(&format!("{key}=")))
        .unwrap_or_else(|| panic!("no {key} in {tail}"))
}

#[tokio::test]
async fn steps_run_in_plan_order_each_holding_its_slots() {
    let rig = Rig::new();
    let profile = TierProfile {
        timing_tests: Some("timing".to_string()),
        ..tiered()
    };
    let outcome = rig.tier(7, &rig.spec(profile, None)).await;
    assert!(outcome.ok, "{outcome:?}");
    assert_eq!(
        outcome.affected,
        Affected::Modules(["b", "c"].iter().map(|s| s.to_string()).collect())
    );
    let kinds: Vec<(StepKind, u32)> = outcome.steps.iter().map(|s| (s.kind, s.granted)).collect();
    assert_eq!(
        kinds,
        [
            (StepKind::Build, 1),
            (StepKind::Tests, 1),
            (StepKind::Tests, 1),
            (StepKind::Timing, 2),
            (StepKind::Timing, 2),
        ]
    );
    let log = rig.log();
    let seen: Vec<(String, String, String, String)> = log
        .iter()
        .map(|l| {
            let what = l.split(' ').take(2).collect::<Vec<_>>().join(" ");
            let filter = l.split_once("filter=").map_or("", |(_, f)| f).to_string();
            (
                what,
                value(l, "slots").into(),
                value(l, "load").into(),
                filter,
            )
        })
        .collect();
    let row = |what: &str, slots: &str, load: &str, filter: &str| {
        (what.into(), slots.into(), load.into(), filter.into())
    };
    assert_eq!(
        seen,
        [
            row("build slots=1", "1", "0.5", ""),
            row("test b", "1", "0.5", "not (timing)"),
            row("test c", "1", "0.5", "not (timing)"),
            // Decision 25: the timing step is exclusive, alone on every slot.
            row("test b", "2", "1.0", "(timing)"),
            row("test c", "2", "1.0", "(timing)"),
        ],
        "{log:#?}"
    );
}

/// The modules each `test <m>` line of the log names, in order.
fn tested(log: &[String]) -> Vec<String> {
    log.iter()
        .filter_map(|l| l.strip_prefix("test "))
        .map(|l| l.split(' ').next().unwrap_or("").to_string())
        .collect()
}

#[tokio::test]
async fn no_check_runs_every_modules_tests_on_a_full_set_with_a_known_graph() {
    // Ruling C-5, pinned. A full trigger with no `check` and a known graph: build, then
    // the tests of every module of the graph.
    let rig = Rig::new();
    let profile = TierProfile {
        full_triggers: vec!["mods/b/**".to_string()],
        ..tiered()
    };
    let outcome = rig.tier(1, &rig.spec(profile.clone(), None)).await;
    assert!(outcome.ok, "{outcome:?}");
    assert_eq!(
        outcome.affected,
        Affected::Modules(["a", "b", "c"].iter().map(|s| s.to_string()).collect())
    );
    let log = rig.log();
    assert!(log[0].starts_with("build "), "{log:#?}");
    assert_eq!(tested(&log), ["a", "b", "c"], "{log:#?}");

    // With `check`, the full set runs `check` alone.
    let rig = Rig::new();
    let outcome = rig.tier(1, &rig.spec(profile, Some("sh check.sh"))).await;
    assert!(outcome.ok, "{outcome:?}");
    assert!(matches!(outcome.affected, Affected::Full(_)), "{outcome:?}");
    let log = rig.log();
    assert_eq!(log.len(), 1, "{log:#?}");
    assert!(log[0].starts_with("check "), "{log:#?}");

    // No `check` and an unknown graph: build_check alone.
    let rig = Rig::new();
    let profile = TierProfile {
        module_graph: GraphSource::Command("exit 3".to_string()),
        ..tiered()
    };
    let outcome = rig.tier(1, &rig.spec(profile, None)).await;
    assert!(outcome.ok, "{outcome:?}");
    assert!(matches!(outcome.affected, Affected::Full(_)), "{outcome:?}");
    assert!(outcome.graph_note.is_some());
    let log = rig.log();
    assert_eq!(log.len(), 1, "{log:#?}");
    assert!(log[0].starts_with("build "), "{log:#?}");
}

#[tokio::test]
async fn step_environment_puts_the_socket_and_data_dir_under_its_tmpdir() {
    let rig = Rig::new();
    rig.state("check-fail", "");
    // A red untiered check: its tail (M8a's) carries what the command saw.
    let outcome = rig
        .tier(42, &rig.spec(TierProfile::default(), Some("sh check.sh")))
        .await;
    assert!(!outcome.ok);
    let tmp = super::super::tier_step::step_base(&rig.data, &rig.dir).join("s42-1");
    let tail = &outcome.tail;
    assert_eq!(tail_value(tail, "tmp"), tmp.display().to_string(), "{tail}");
    assert_eq!(
        tail_value(tail, "sock"),
        tmp.join("d.sock").display().to_string()
    );
    assert_eq!(
        tail_value(tail, "data"),
        tmp.join("data").display().to_string()
    );
    assert_eq!(tail_value(tail, "present"), "yes", "made before the step");
    assert!(!tmp.exists(), "removed after the step: {}", tmp.display());
    assert!(!tmp.starts_with(&rig.dir), "never inside the checkout");
    assert!(
        tmp.join("d.sock").as_os_str().len() < 104,
        "a Unix socket path fits"
    );
}

#[tokio::test]
async fn untiered_spec_runs_check_exactly_as_m8a() {
    let rig = Rig::new();
    rig.state("check-fail", "");
    // Placeholders are left alone for an untiered profile (decision 6, ruling C-4).
    let check = "sh check.sh {shard} {filter:-x %}";
    let outcome = rig
        .tier(3, &rig.spec(TierProfile::default(), Some(check)))
        .await;
    assert!(!outcome.ok);
    assert_eq!(outcome.steps.len(), 1);
    assert_eq!(outcome.steps[0].command, check);
    assert!(
        !outcome.steps[0].retried,
        "no flaky retry for an untiered profile"
    );
    let log = rig.log();
    assert_eq!(log.len(), 1, "{log:#?}");
    assert!(
        log[0].starts_with("check args={shard} {filter:-x %} "),
        "{log:#?}"
    );
    assert_eq!(value(&log[0], "pwd"), rig.dir.display().to_string());
    assert_eq!(value(&log[0], "foo"), "bar", "the profile's env");
    let slots: BTreeMap<&str, &str> = [
        "CARGO_BUILD_JOBS",
        "NEXTEST_TEST_THREADS",
        "RUST_TEST_THREADS",
        "ANTHREX_TEST_SLOTS",
        "ANTHREX_TEST_LOAD",
    ]
    .iter()
    .map(|v| (*v, tail_value(&outcome.tail, v)))
    .collect();
    assert_eq!(
        slots,
        BTreeMap::from([
            ("CARGO_BUILD_JOBS", "1"),
            ("NEXTEST_TEST_THREADS", "1"),
            ("RUST_TEST_THREADS", "1"),
            ("ANTHREX_TEST_SLOTS", "1"),
            ("ANTHREX_TEST_LOAD", "0.5"),
        ])
    );
    let tmp = super::super::tier_step::step_base(&rig.data, &rig.dir).join("s3-1");
    assert_eq!(tail_value(&outcome.tail, "tmp"), tmp.display().to_string());
    let head = git(&rig.dir, &["rev-parse", "HEAD"]);
    assert_eq!(
        head, rig.head,
        "M8a's scratch checkout at the claimed commit"
    );
}

#[tokio::test]
async fn test_at_checks_out_the_commit_and_runs_the_named_tests_twice_on_red() {
    let rig = Rig::new();
    // A test that fails at the second commit only, by a file in the tree.
    write(
        &rig.root.join("probe.sh"),
        "echo \"probe $(cat mods/b/lib.txt)\" >> \"$TIER_LOG\"\n\
         if [ \"$(cat mods/b/lib.txt)\" = \"b changed\" ]; then exit 1; fi\nexit 0\n",
    );
    git(&rig.root, &["add", "probe.sh"]);
    git(&rig.root, &["commit", "-q", "-m", "probe"]);
    let red_at = git(&rig.root, &["rev-parse", "HEAD"]);
    git(&rig.root, &["checkout", "-q", &rig.base]);
    write(
        &rig.root.join("probe.sh"),
        "echo \"probe $(cat mods/b/lib.txt)\" >> \"$TIER_LOG\"\nexit 0\n",
    );
    git(&rig.root, &["add", "probe.sh"]);
    git(&rig.root, &["commit", "-q", "-m", "probe at base"]);
    let green_at = git(&rig.root, &["rev-parse", "HEAD"]);
    git(&rig.root, &["checkout", "-q", "main"]);

    let dir = rig.dir.with_file_name(".full");
    let probe = |commit: &str| TestAtSpec {
        root: rig.root.clone(),
        dir: dir.clone(),
        commit: commit.to_string(),
        setup: None,
        commands: vec!["sh probe.sh".to_string()],
        timeout_secs: 60,
        env: rig.spec(tiered(), None).env,
    };
    let (sched, queue) = (TestScheduler::new(2), GitQueue::new());
    let ctx = rig.ctx();
    let git_program = OsStr::new("git");

    let red = run_test_at(&ctx, &sched, &queue, git_program, 5, &probe(&red_at)).await;
    let OpResult::TestAt {
        red: is_red,
        failing,
        show,
        ..
    } = red
    else {
        panic!("{red:?}");
    };
    assert!(is_red);
    assert_eq!(failing, ["sh probe.sh"]);
    assert_eq!(show, None);
    assert_eq!(git(&dir, &["rev-parse", "HEAD"]), red_at);
    assert_eq!(
        rig.log(),
        ["probe b changed", "probe b changed"],
        "run twice"
    );

    let green = run_test_at(&ctx, &sched, &queue, git_program, 6, &probe(&green_at)).await;
    assert!(
        matches!(&green, OpResult::TestAt { red: false, failing, .. } if failing.is_empty()),
        "{green:?}"
    );
    assert_eq!(git(&dir, &["rev-parse", "HEAD"]), green_at);
    assert_eq!(rig.log().len(), 3, "a green command runs once");

    // A command that fails once and then passes does not make the probe red.
    let flaky = r#"n=$(cat "$TIER_STATE/f" 2>/dev/null || echo 0); echo $((n+1)) > "$TIER_STATE/f"; [ "$n" -ge 1 ]"#;
    let mut once = probe(&red_at);
    once.commands = vec![flaky.to_string()];
    let result = run_test_at(&ctx, &sched, &queue, git_program, 7, &once).await;
    assert!(
        matches!(&result, OpResult::TestAt { red: false, .. }),
        "{result:?}"
    );
    assert_eq!(
        std::fs::read_to_string(rig.state.join("f")).unwrap().trim(),
        "2"
    );
    let repo = git::checkout_repo_dir(&rig.data, &dir);
    let _ = git::remove_task_tmp(&repo);
}

// Decision 33 and ruling C-7: the retry.
#[path = "tier_tests_retry.rs"]
mod retry;
