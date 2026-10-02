//! Decision 14's scripted CI. A [`CiRule`] names a check; the checks of a PR are the
//! distinct names of the rules, in order. The first time a check is seen on a head it
//! becomes one Actions run ([`CiRun`]): red with the rule's conclusion when the rule's
//! `fail_if` holds on that head (or it has none) and its `times` are not used up,
//! otherwise green. `gh run rerun --failed` re-decides a red run under the same id, so
//! a `times: Some(1)` rule is red once and green after a rerun. A run stays pending
//! (`IN_PROGRESS`) for its rule's `pending` views.

use serde::{Deserialize, Serialize};

use super::github::{Bare, FakeGithub, iso};
use crate::host::Conclusion;

/// Red only when `path` at the head contains `text`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileContains {
    pub path: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CiRule {
    pub check: String,
    pub fail_if: Option<FileContains>,
    pub conclusion: Conclusion,
    /// Printed as `--- FAIL: <name>` lines in the failed log.
    pub failing_tests: Vec<String>,
    pub log: String,
    /// At most this many runs are red; `None` is every one.
    pub times: Option<u32>,
    /// The run reads `IN_PROGRESS` for this many `gh pr view` calls first (and again
    /// after each rerun).
    #[serde(default)]
    pub pending: u32,
}

impl CiRule {
    /// A rule that turns every run of `check` to `conclusion`.
    pub fn new(check: &str, conclusion: Conclusion) -> CiRule {
        CiRule {
            check: check.to_string(),
            fail_if: None,
            conclusion,
            failing_tests: Vec::new(),
            log: String::new(),
            times: None,
            pending: 0,
        }
    }

    pub fn when(mut self, path: &str, text: &str) -> CiRule {
        self.fail_if = Some(FileContains {
            path: path.to_string(),
            text: text.to_string(),
        });
        self
    }

    pub fn log(mut self, log: &str) -> CiRule {
        self.log = log.to_string();
        self
    }

    pub fn failing(mut self, tests: &[&str]) -> CiRule {
        self.failing_tests = tests.iter().map(|t| t.to_string()).collect();
        self
    }

    pub fn times(mut self, times: u32) -> CiRule {
        self.times = Some(times);
        self
    }

    pub fn pending(mut self, views: u32) -> CiRule {
        self.pending = views;
        self
    }
}

/// One Actions run of one check on one head.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CiRun {
    pub id: u64,
    pub job: u64,
    pub head: String,
    pub check: String,
    pub attempt: u32,
    pub conclusion: Conclusion,
    /// `gh pr view` calls left before it completes.
    pub pending: u32,
    pub log: String,
    pub failing_tests: Vec<String>,
    pub started_at: u64,
}

impl CiRun {
    pub fn is_red(&self) -> bool {
        self.pending == 0 && self.conclusion.is_red()
    }
}

/// The verdict of the rules of `check` on `head`; a red one uses up one of its `times`.
fn decide(
    state: &mut FakeGithub,
    bare: &Bare<'_>,
    check: &str,
    head: &str,
) -> Result<Decision, String> {
    state.ci_used.resize(state.ci.len(), 0);
    for (i, rule) in state.ci.iter().enumerate() {
        if rule.check != check || rule.times.is_some_and(|t| state.ci_used[i] >= t) {
            continue;
        }
        let holds = match &rule.fail_if {
            None => true,
            Some(f) => bare
                .file(head, &f.path)?
                .is_some_and(|text| text.contains(&f.text)),
        };
        if holds {
            state.ci_used[i] += 1;
            return Ok(Decision {
                conclusion: rule.conclusion,
                log: rule.log.clone(),
                failing_tests: rule.failing_tests.clone(),
                pending: rule.pending,
            });
        }
    }
    let pending = state
        .ci
        .iter()
        .find(|r| r.check == check)
        .map_or(0, |r| r.pending);
    Ok(Decision {
        conclusion: Conclusion::Success,
        log: String::new(),
        failing_tests: Vec::new(),
        pending,
    })
}

struct Decision {
    conclusion: Conclusion,
    log: String,
    failing_tests: Vec<String>,
    pending: u32,
}

/// The runs `gh pr view` shows for `head`: one per check, created when first seen.
/// Each call is one view: a pending run gets one view closer to done.
pub fn runs_for(
    state: &mut FakeGithub,
    bare: &Bare<'_>,
    head: &str,
    now: u64,
) -> Result<Vec<CiRun>, String> {
    let mut checks: Vec<String> = Vec::new();
    for rule in &state.ci {
        if !checks.contains(&rule.check) {
            checks.push(rule.check.clone());
        }
    }
    let mut shown = Vec::new();
    for check in checks {
        let at = state
            .ci_runs
            .iter()
            .rposition(|r| r.head == head && r.check == check);
        let at = match at {
            Some(at) => at,
            None => {
                let d = decide(state, bare, &check, head)?;
                let id = state.next_run();
                let job = state.next_run();
                state.ci_runs.push(CiRun {
                    id,
                    job,
                    head: head.to_string(),
                    check: check.clone(),
                    attempt: 1,
                    conclusion: d.conclusion,
                    pending: d.pending,
                    log: d.log,
                    failing_tests: d.failing_tests,
                    started_at: now,
                });
                state.ci_runs.len() - 1
            }
        };
        let run = &mut state.ci_runs[at];
        shown.push(run.clone());
        run.pending = run.pending.saturating_sub(1);
    }
    Ok(shown)
}

/// `gh run rerun <id> --failed`: `Err` carries gh's stderr.
pub fn rerun(
    state: &mut FakeGithub,
    bare: &Bare<'_>,
    id: u64,
) -> Result<Result<(), String>, String> {
    let Some(at) = state.ci_runs.iter().position(|r| r.id == id) else {
        return Ok(Err(not_found(id)));
    };
    let run = &state.ci_runs[at];
    if run.pending > 0 {
        return Ok(Err(format!(
            "run {id} cannot be rerun; This workflow is already running"
        )));
    }
    if !run.is_red() {
        return Ok(Err(format!(
            "run {id} cannot be rerun; it has no failed jobs"
        )));
    }
    let (check, head) = (run.check.clone(), run.head.clone());
    let d = decide(state, bare, &check, &head)?;
    // GitHub gives a re-run's jobs new ids (its `detailsUrl` changes), which is how the
    // engine sees the red after a re-run (task M9.2.9).
    let job = state.next_run();
    let run = &mut state.ci_runs[at];
    run.job = job;
    run.attempt += 1;
    run.conclusion = d.conclusion;
    run.pending = d.pending;
    run.log = d.log;
    run.failing_tests = d.failing_tests;
    Ok(Ok(()))
}

/// `gh run view <id> --log-failed`: `Ok` is stdout, `Err` gh's stderr.
pub fn failed_log(state: &FakeGithub, id: u64) -> Result<String, String> {
    let Some(run) = state.ci_runs.iter().find(|r| r.id == id) else {
        return Err(not_found(id));
    };
    if run.pending > 0 {
        return Err(format!(
            "run {id} is still in progress; logs will be available when it is complete"
        ));
    }
    if !run.is_red() {
        return Ok(String::new());
    }
    // gh's line shape (M9.2.1 check 4): `<job>\t<step>\t<time> <text>`, a byte-order
    // mark before the first line's timestamp.
    let stamp = iso(run.started_at).replace('Z', ".0000000Z");
    let lines = run.log.lines().map(str::to_string).chain(
        run.failing_tests
            .iter()
            .map(|t| format!("--- FAIL: {t} (0.01s)")),
    );
    let mut out = String::new();
    for (i, line) in lines.enumerate() {
        let bom = if i == 0 { "\u{feff}" } else { "" };
        out.push_str(&format!(
            "{}\tUNKNOWN STEP\t{bom}{stamp} {line}\n",
            run.check
        ));
    }
    Ok(out)
}

fn not_found(id: u64) -> String {
    format!("failed to get run: HTTP 404: Not Found (actions/runs/{id})")
}
