//! Milestone 9.1 task M9.1.21: a tiered test repository, its profile, and the log its
//! scripts write (brief, Interfaces "Shared test helpers").
//!
//! Three modules `mods/a`, `mods/b`, `mods/c` (`c` depends on `b`, `b` on `a`), a
//! `docs/` directory, and shell scripts at the root. Every script appends one JSON line
//! per run to `$TIER_LOG`, `<RunHarness::cache_dir()>/tier-log.jsonl`, the one
//! directory outside the checkout a confined command may write. A test makes a module
//! fail with `mods/<m>/FAIL`, or flake once with `mods/<m>/FLAKY` (its first run fails,
//! every later one passes: a counter file beside the log remembers), both committed by
//! a scripted worker, so no test depends on timing.
//!
//! The scripts are POSIX `sh` (dash on Linux) and run as `sh <script>`, never executed
//! directly, so a freshly written file cannot fail with ETXTBSY. Times come from `perl`,
//! present on macOS and on Linux CI, with microseconds, so a test can compare steps.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::Value;

use super::run_harness::{RUN_WAIT, RunHarness};

/// A tiered test's wait (`docs/timing-budgets.md`, ruling C-25): M8a's (`RUN_WAIT`),
/// plus the tier commands the test can run at the harness's `check_timeout_secs = 10`.
/// A green script never retries, so a task runs at most 10: tier 1 and tier 2 each a
/// `build`, three module tests and a timing step. The largest test here has three
/// tasks, whose tier-2 jobs run one at a time through the width-1 merge queue, plus one
/// tier-3 job (`check` and its timing step): 32 commands, 320 s. A test with a flake
/// runs one task and adds two retries. The idle tier 3 waits `full_idle_secs` (120 s)
/// of an idle stage, which none of these runs reach before completion starts its own.
/// Plus the tier jobs' own git calls (diffs, materialising, cache reads): at most 10
/// at the harness's 5 s `git_timeout_secs`, 50 s.
pub const TIER_WAIT: Duration = Duration::from_secs(RUN_WAIT.as_secs() + 320 + 50);

/// Shared by every script: the clock, JSON quoting, one module's verdict, the log line.
const LIB_SH: &str = r#"ax_now() { perl -MTime::HiRes=time -e 'printf "%.6f", time'; }
ax_json() { printf '%s' "$1" | sed -e 's/\\/\\\\/g' -e 's/"/\\"/g' | tr '\n' ' '; }
# Prints libtest-shaped output for module $1 and fails when it fails.
ax_module() {
  if [ -f "mods/$1/FAIL" ]; then
    ax_failed "$1::works"; return 1
  fi
  if [ -f "mods/$1/FLAKY" ] && [ -n "${TIER_LOG-}" ]; then
    seen="$(dirname "$TIER_LOG")/flaky-$1"
    if [ ! -f "$seen" ]; then
      : > "$seen"; ax_failed "$1::flaky"; return 1
    fi
  fi
  echo "test $1::works ... ok"
  echo "test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out"
}
ax_failed() {
  echo "test $1 ... FAILED"
  echo
  echo "failures:"
  echo "    $1"
  echo
  echo "test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out"
}
# ax_log <script> <start> <args...>
ax_log() {
  [ -n "${TIER_LOG-}" ] || return 0
  script=$1; start=$2; shift 2
  end=$(ax_now)
  args=""
  for a in "$@"; do args="$args${args:+,}\"$(ax_json "$a")\""; done
  env=""
  for v in CARGO_BUILD_JOBS NEXTEST_TEST_THREADS RUST_TEST_THREADS ANTHREX_TEST_SLOTS \
      ANTHREX_TEST_LOAD TMPDIR ANTHREX_SOCKET ANTHREX_DATA_DIR; do
    eval "val=\${$v-}"
    env="$env${env:+,}\"$v\":\"$(ax_json "$val")\""
  done
  printf '{"script":"%s","args":[%s],"cwd":"%s","start":%s,"end":%s,"env":{%s}}\n' \
    "$script" "$args" "$(ax_json "$PWD")" "$start" "$end" "$env" >> "$TIER_LOG"
}
"#;

const GRAPH_SH: &str = r#". ./lib.sh
start=$(ax_now)
printf '{"a":[],"b":["a"],"c":["b"]}\n'
ax_log graph.sh "$start" "$@"
"#;

const BUILD_SH: &str = r#". ./lib.sh
start=$(ax_now)
echo "build ok"
ax_log build.sh "$start" "$@"
"#;

/// `test.sh <module> [--filter <expr>]`, or `test.sh --one <module>::<test>`.
const TEST_SH: &str = r#". ./lib.sh
start=$(ax_now)
if [ "$1" = "--one" ]; then
  module=${2%%::*}
else
  module=$1
fi
sleep 0.2
ax_module "$module"
status=$?
if [ "$1" = "--one" ] && [ $status -eq 0 ]; then echo "PASS $2"; fi
ax_log test.sh "$start" "$@"
exit $status
"#;

/// `check.sh [--filter <expr>] [--shard <k>/<n>]`: every module.
const CHECK_SH: &str = r#". ./lib.sh
start=$(ax_now)
sleep 0.2
status=0
for dir in mods/*/; do
  m=$(basename "$dir")
  ax_module "$m" || status=1
done
ax_log check.sh "$start" "$@"
exit $status
"#;

/// The tiered repository's files, for [`RunHarness::with_config`]'s base commit.
pub fn tier_repo_files() -> Vec<(&'static str, &'static str)> {
    vec![
        ("lib.sh", LIB_SH),
        ("graph.sh", GRAPH_SH),
        ("build.sh", BUILD_SH),
        ("test.sh", TEST_SH),
        ("check.sh", CHECK_SH),
        ("mods/a/src.txt", "a\n"),
        ("mods/b/src.txt", "b\n"),
        (
            "mods/b/tests/t.rs",
            "#[test]\nfn works() {\n    assert!(true);\n}\n",
        ),
        ("mods/c/src.txt", "c\n"),
        ("docs/guide.md", "guide\n"),
    ]
}

/// Where the scripts log: `<cache_dir>/tier-log.jsonl`.
pub fn tier_log_path(h: &RunHarness) -> PathBuf {
    h.cache_dir().join("tier-log.jsonl")
}

/// The plan's `[profile]` block for the tiered repository, logging to `log`, plus
/// `extra` lines.
pub fn tier_profile(log: &Path, extra: &str) -> String {
    format!(
        "[profile]\n\
         check = \"sh check.sh {{filter:--filter %}}\"\n\
         modules = [\"mods/*\"]\n\
         module_graph = \"sh graph.sh\"\n\
         module_names = \"dir\"\n\
         build_check = \"sh build.sh\"\n\
         module_test = \"sh test.sh {{module}} {{filter:--filter %}}\"\n\
         single_test = \"sh test.sh --one {{test}}\"\n\
         test_passed = \"PASS {{test}}\"\n\
         source = [\"mods/**\"]\n\
         env = {{ TIER_LOG = {:?} }}\n\
         {extra}\n",
        log.display().to_string()
    )
}

/// A plan on the tiered profile with `extra` profile lines and `tasks`.
pub fn tier_plan(h: &RunHarness, extra: &str, tasks: &[String]) -> String {
    format!(
        "goal = \"Add a\"\n\n{}\n{}",
        tier_profile(&tier_log_path(h), extra),
        tasks.concat()
    )
}

/// Every line the scripts logged so far (none when the log does not exist).
pub fn tier_log(h: &RunHarness) -> Vec<Value> {
    std::fs::read_to_string(tier_log_path(h))
        .unwrap_or_default()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap_or_else(|e| panic!("{e}: {line}")))
        .collect()
}

/// The logged runs of `script`.
pub fn runs_of<'a>(log: &'a [Value], script: &str) -> Vec<&'a Value> {
    log.iter().filter(|l| l["script"] == script).collect()
}

/// A logged line's arguments as strings.
pub fn args(line: &Value) -> Vec<&str> {
    line["args"]
        .as_array()
        .map(|a| a.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default()
}

/// Whether a logged line ran in task `task`'s tier-1 checkout (`<task>.proof`).
pub fn in_proof_of(line: &Value, task: &str) -> bool {
    line["cwd"]
        .as_str()
        .is_some_and(|cwd| cwd.ends_with(&format!("/{task}.proof")))
}

/// Whether a logged line ran in the integration worktree (tier 2).
pub fn in_integration(line: &Value) -> bool {
    line["cwd"]
        .as_str()
        .is_some_and(|cwd| cwd.ends_with("/integration"))
}
