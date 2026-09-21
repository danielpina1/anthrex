# anthrex design: the node inspector

Date: 2026-09-21. Status: binding. Implemented by milestone 4.7.

Amends `docs/superpowers/specs/2026-09-20-graph-overview-design.md` §4.4 and §4.5, which gave the overview a one-line footer. Section 6 lists what it supersedes.

## 1. What changes

Selecting a node in the graph overview — with the keys or by clicking it — shows everything anthrex knows about that node in a panel below the canvas: what it is, what model it runs, who spawned it, which checkout it is standing in and what that checkout's git state is, and how long it has been doing whatever it is doing.

The one-line footer becomes a bordered box. Nothing else about the overview changes.

**No protocol change.** Every field already crosses the wire: `WindowInfo` carries `cwd`, `worktree`, `branch`, `model`, `session_id`, `tool` and `status`; `SubagentInfo` carries `parent_id`, `kind`, `label`, `model`, `state`, `tool` and its timestamps; `GitState` arrives per worktree root from milestone 4.5. This milestone reads what is already there.

## 2. Terms

- **inspector** — the panel below the canvas showing the selected node.
- **field** — one labelled value in the inspector, such as `model  opus`.

## 3. What it shows

The inspector's title is the node's own name, bold, after its status glyph in the status colour. Below it, fields in labelled pairs, laid out in columns across the available width.

**A project:**

| Field | Value |
|---|---|
| path | the project root, with `$HOME` shortened to `~` |
| status | the rolled-up status, as the tree computes it |
| agents | the runtime counts, as the tree shows them (`cl 2 · sh 1`) |
| branch | the branch, when every window in the project shares one worktree; otherwise the count, `3 worktrees` |
| changes | dirty and untracked counts for that worktree, when there is exactly one |

**An agent window:**

| Field | Value |
|---|---|
| runtime | `claude`, `codex` or `shell` |
| model | the model, or `-` |
| status | the status word, and the tool when one is running |
| for | how long it has been in that status |
| dir | its working directory, `~`-shortened |
| worktree | its worktree root, when that differs from `dir` |
| branch | from its worktree's `GitState`, with the dirty and ahead/behind counts |
| session | the session id, when the runtime reports one |
| sub-agents | how many, and how many are still running |

**A sub-agent:**

| Field | Value |
|---|---|
| kind | its `kind`, such as `Explore` |
| task | its label in full, wrapped rather than elided — the box exists so that nothing has to be truncated |
| model | the model, or `-` |
| state | `running`, `done` or `failed`, and the tool when one is running |
| for | elapsed if running, otherwise how long it ran |
| spawned by | the parent sub-agent's label when `parent_id` names one, otherwise the owning window's number and name |
| depth | how deep below its window it sits |

The "spawned by" field is the one that cannot be read off the tree: a sub-agent three levels down looks the same as one directly under its window unless you trace the guides by eye.

## 4. Layout

The overview's interior splits into the canvas and the inspector, which is `INSPECTOR_HEIGHT` = 8 rows — a rounded border, a title row, five field rows — pinned to the bottom and spanning the full width.

Fields flow into as many columns as fit: each column is as wide as its widest label plus its widest value plus two spaces of gutter, and columns are packed left to right until the width runs out, then wrap to the next row. A value too long for its column is elided with `…`, except the sub-agent's `task`, which wraps across the remaining rows because it is the field most worth reading in full.

When the terminal is too short for both — fewer than `INSPECTOR_HEIGHT + 6` rows of interior — the inspector collapses to the single line milestone 4.6 had, so a small terminal degrades rather than losing the canvas.

`i` toggles the inspector while the overview is open. Off, the single line returns. The choice is remembered for the session.

## 5. Interaction

Unchanged from milestone 4.6: the keys and the mouse select nodes, and selection reveals. The inspector is display-only and never takes focus. Clicking a sub-agent selects it and fills the inspector, which is what "see it when I click it" means here.

A sub-agent's *output* is not shown: a Claude sub-agent runs inside its parent session as a tool call, with no terminal of its own and no input channel, so there is nothing to attach to. Reading the session transcript is possible and is left to its own milestone; genuine interaction requires anthrex to have spawned the agent itself, which is milestone 8.

## 6. What this supersedes

| In `2026-09-20-graph-overview-design.md` | Status |
|---|---|
| §4.4's "the footer shows the selected node in full" | replaced by §3 and §4 here |
| §4.5's footer row | replaced by the inspector; the key table gains `i` |
| everything else | unchanged and still binding |

## 7. Testing

- The field list for each node kind, asserted as exact label/value pairs, including a window with no model, a sub-agent with no label, and a sub-agent whose `parent_id` names another sub-agent.
- "Spawned by" resolving to a parent sub-agent, to the owning window, and to the window when `parent_id` names a sub-agent that is no longer in the list.
- Git fields present when the worktree has a `GitState` and absent when it does not.
- The column packing: exact rendered strings at a wide width, at a width where fields wrap to a second row, and at a width where a value is elided.
- The sub-agent `task` field wrapping rather than eliding.
- The collapse to one line on a short terminal, and `i` toggling.
- No panic at any terminal size, as milestone 4.6's guard does.

## 8. Risks

- **The inspector eats canvas height**, which is the axis the tree grows along. Eight rows out of fifty is acceptable; on a short terminal the collapse rule is what stops it being a problem.
- **Column packing is fiddly** in the same way the edge junctions were: the cases are "fits", "wraps", "elides" and "wraps and elides", and each needs its own exact-string test.
- **`GitState` is keyed by worktree root, not project root.** A project whose windows span several worktrees has no single branch to show, which is why that field degrades to a count rather than picking one arbitrarily.
