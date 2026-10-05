"""The fixtures of the design-flow smoke stage, stage 11k (milestone 9.6, task M9.6.21):
its bounds, its configuration, the documents its scripted agents submit, their scripts,
and its file and git helpers. Split from `scripts/pty_smoke_design.py`, which imports
them, for the brief's rule that a new file starts under 400 lines.

The documents are those of `crates/cli/tests/support/run_design.rs`, which pass the real
template, numbering and brief checks (`crates/cli/tests/run_design_fixtures.rs`); the
orchestrator's steps are `orch_design_steps` of `support/run_design_orch.rs` with the
changes at the spec gate of `e2e_changes_at_each_gate_makes_a_new_version`.
"""

import os
import subprocess

from pty_smoke_keep_going import REQUEST_WAIT, SCREEN_WAIT
from pty_smoke_run import RUN_WAIT, git_env

# `ORCH_WAIT` of `crates/cli/tests/support/run_orch.rs`: a gate, a review or a round of
# drafts after the step before it.
ORCH_WAIT = 120.0
# The documents commit (`DOCS_COMMIT_WAIT` of `run_e2e_design/common.rs`): the read-back
# (`IO_WAIT`, 10 s) and at most 30 git calls at the stage's `git_timeout_secs` (5 s).
GIT_TIMEOUT_SECS = 5
DOCS_COMMIT_WAIT = 10.0 + 30 * GIT_TIMEOUT_SECS
# A design run from its plan's approval to completion: the commit, then one task path.
DESIGN_RUN_WAIT = RUN_WAIT + DOCS_COMMIT_WAIT
# The screen steps of one gate: `C-b a`, its row, the menu, the screen, the key, its
# page or editor, the note, the confirm (one `SCREEN_WAIT` each).
GATE_STEPS = 8
# The scripted orchestrator's wait for a user's decision at a gate: the stage's wait for
# the gate, its screen steps, and the request.
USER_WAIT = ORCH_WAIT + GATE_STEPS * SCREEN_WAIT + REQUEST_WAIT

GOAL = "add a.txt"
LABELS = ("claude", "codex")
QUESTION = "Should a.txt end with a newline?"
ANSWER = "skip"
NOTE = "say what happens to an old line"
DRAFTS_IN = "both brainstorm drafts are in; read them with get_doc and submit the merged report"
SHELL = "design-smoke"
# The sidebar's columns at the smoke's 120 columns, its frame included.
SIDEBAR = 34
ORCH = "orchestrator-run-1"

CONFIG = """[orchestrator]
git_timeout_secs = {git}
wake_quiet_secs = 1
design.default = "off"
planners.timeout_secs = 120
{unconfined}
[orchestrator.profile]
check_timeout_secs = 10
"""

TRIAGE = {"answer": {"kinds": ["code"], "scale": "plan", "reason": "several modules", "task": None}}

# The fixture documents of `crates/cli/tests/support/run_design.rs`, which pass the real
# template, numbering and brief checks (`run_design_fixtures.rs`).
DRAFT = """## Understanding
Add a.txt holding one line ({label}'s reading). Success: a.txt is on the base branch.

## Assumptions
- (assumed) a.txt does not exist yet.

## Constraints found
- README:1 is the only tracked file.

## Approaches
### 1. Write the file directly
One task writes a.txt. Files: a.txt. Trade-offs: none. Risks: none. Size S.

### 2. Generate it from a script
A script writes a.txt. Files: gen.sh, a.txt. Trade-offs: more to review. Size M.

## Recommendation
Write the file directly: it is the smallest change.

## Questions for you
None.
"""

REPORT = """## Where they agree
Both write a.txt in one small task.

## Where they disagree
claude writes the file directly; codex also weighs a generator script. Judgment: write it directly.

## Approaches
### 1. Write the file directly [both]
One task writes a.txt. Size S.

### 2. Generate it from a script [codex]
A script writes a.txt. Size M.

## Recommendation
Write the file directly: it is the smallest change.

## Questions for you
None.
"""

SPEC = """# Add a.txt

## Goal and success criteria
a.txt exists on the base branch with one line.

## Non-goals
Nothing else changes.

## Approach
Write the file directly, as the brainstorm recommends.

## Design
One task writes a.txt in the repository root.

## Requirements
R1 a.txt exists at the repository root. Acceptance: `test -f a.txt` succeeds.
R2 a.txt holds exactly one line. Acceptance: `wc -l < a.txt` prints 1.

## Interfaces
None.

## Errors and edge cases
An existing a.txt is replaced.

## Testing
A check-mode task; its reviewer reads the file.

## Risks
None.

## Open questions
"""
SPEC_V2 = SPEC.replace(
    "An existing a.txt is replaced.", "An existing a.txt is replaced; its old line is dropped."
)

BRIEF = (
    "Write a.txt.\nFiles:\n- a.txt\nTests first:\n- none; a check-mode task\nSteps:\n"
    "- write one line to a.txt\nAcceptance:\n- a.txt holds one line\nVerify:\n- wc -l < a.txt\n"
)
TASK = {
    "id": "t1",
    "title": "Task t1",
    "brief": BRIEF,
    "acceptance": ["t1 is done"],
    "owns": ["a.txt"],
    "size": "S",
    "test_mode": "check",
    "test_mode_reason": "a text file",
    "covers": ["R1", "R2"],
}


def _call(tool, args):
    return {"mcp_call": {"tool": tool, "args": args}}


def _until(pointer, equals, timeout):
    return {
        "mcp_until": {
            "tool": "run_status",
            "args": {"wait_secs": 5},
            "until": {"pointer": pointer, "equals": equals},
            "timeout_ms": int(timeout * 1000),
        }
    }


def _expect(pointer, equals):
    return {"expect": {"pointer": pointer, "equals": equals}}


# The orchestrator, `orch_design_steps` of `support/run_design_orch.rs` with the changes
# at the spec gate of `e2e_changes_at_each_gate_makes_a_new_version`: a marker (a
# `run_status` with `wait_secs` 0, which no other step sends), the question, the user's
# typed `skip` and the brainstorm's start (answers left empty: the user skipped them);
# the drafts' wake, both drafts read, the merged report; the spec through review 1, then
# ready; the user's note, the spec v2; the plan through its review; then it waits for the
# run's completion and for a message that never comes, so its window stays live until
# the stage's daemon stops. Every user decision is awaited on the digest's state.
ORCHESTRATOR = [
    {"hook": "UserPromptSubmit", "payload": {"prompt": "plan"}},
    _call("run_status", {"wait_secs": 0}),
    {"print": QUESTION},
    {"read_message": {"expect": ANSWER}},
    _call("start_brainstorm", {"answers": ""}),
    {"read_message": {"expect": DRAFTS_IN}},
    *[_call("get_doc", {"kind": "brainstorm_draft", "from": label}) for label in LABELS],
    _call("submit_doc", {"kind": "brainstorm", "text": REPORT}),
    _until("/gate/state", "specifying", USER_WAIT),
    _call("submit_doc", {"kind": "spec", "text": SPEC, "ready": False}),
    _until("/gate/spec_review/running", False, ORCH_WAIT),
    _expect("/gate/spec_review/review", 1),
    _call("submit_doc", {"kind": "spec", "text": SPEC, "ready": True, "responses": []}),
    _until("/gate/doc_gate/revising", NOTE, USER_WAIT),
    _call("submit_doc", {"kind": "spec", "text": SPEC_V2, "ready": True, "responses": []}),
    _until("/gate/state", "planning", USER_WAIT),
    _call("edit_plan", {"edits": [{"op": "add_task", "task": TASK}], "submit": True}),
    _expect("/awaiting_review", True),
    _until("/gate/plan_review/running", False, ORCH_WAIT),
    _call("edit_plan", {"edits": [], "submit": True, "responses": []}),
    _expect("/awaiting_approval", True),
    _until("/gate/state", "approved", USER_WAIT),
    _until("/run/complete", True, DESIGN_RUN_WAIT),
    {"read_message": {}},
]


def _brainstormer(label):
    return [_call("submit_doc", {"kind": "brainstorm_draft", "text": DRAFT.format(label=label)})]


SPEC_REVIEWER = [
    _call("get_doc", {"kind": "spec", "draft": 1}),
    _call("submit_findings", {"findings": []}),
]
PLAN_REVIEWER = [
    _call("get_doc", {"kind": "plan"}),
    _call("get_doc", {"kind": "spec"}),
    _call("submit_findings", {"findings": []}),
]
WORKER = [
    {"git_commit": {"file": "a.txt", "content": "a\n", "message": "add a.txt"}},
    _call("task_done", {"summary": "added a.txt"}),
]


def _write(path, text):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w") as f:
        f.write(text)


# AGENTS.md rule 11's other variables, beyond the five `git_env` drops.
GIT_ENV_DROP_MORE = (
    "GIT_OBJECT_DIRECTORY",
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_LITERAL_PATHSPECS",
    "GIT_GLOB_PATHSPECS",
    "GIT_NOGLOB_PATHSPECS",
    "GIT_ICASE_PATHSPECS",
)


def _git_bytes(args, repo, fail):
    """`git <args>` in `repo` with the smoke's git environment, scrubbed as AGENTS.md rule
    11 says: its stdout, untrimmed."""
    env = {k: v for k, v in git_env().items() if k not in GIT_ENV_DROP_MORE}
    result = subprocess.run(
        ["git", "--no-optional-locks", *args],
        cwd=repo,
        env=env,
        stdin=subprocess.DEVNULL,
        capture_output=True,
        timeout=15,
    )
    if result.returncode != 0:
        fail(f"git {args!r} failed ({result.returncode}): {result.stderr!r}")
    return result.stdout
