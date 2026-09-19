# Milestone 4: Project tree view

## Header

| Field | Value |
|-------|-------|
| Status | `ready` |
| Depends on | Milestone 3 (agent status and sub-agent tracking from hooks) |
| Spec sections | `docs/superpowers/specs/2026-09-18-anthrex-product-design.md` sections 3, 4.4 (the `needs_permission` marker) and 5; section 10.1 for the protocol version and 10.3 for the keys |
| Branch | `m4-project-tree` |
| Protocol version | 3 (from 2). Rule (product spec 10.1): set `PROTO_VERSION` to one more than the value on `main` when you start; 3 assumes roadmap order. |

### Starting point

Milestone 3 is merged before this one starts. This brief uses the names of `docs/milestones/M3-agent-status.md`:

- `proto::WindowInfo` has `session_id: Option<String>` (replacing `has_session`), `model: Option<String>` and `subagents: Vec<SubagentInfo>`.
- `proto::SubagentInfo` has the fields of spec section 4.4: `id`, `parent_id`, `kind`, `label`, `model`, `state`, `tool`, `started_secs`, `ended_secs` and `needs_permission: bool`.
- `proto::SubagentState` has the variants `Running`, `Done` and `Failed`, serialized lowercase.
- The hidden `anthrex hook` command exists and forwards `HookEvent` to the daemon, which answers `Ack { request: "hook" }`.
- `crates/fake-agent` (package `anthrex-fake-agent`) builds the `fake-agent` binary, and `ManagerConfig::from_env` honours `ANTHREX_CLAUDE_BIN` and `ANTHREX_CODEX_BIN`. `WindowManager::new(ManagerConfig)` creates the manager.
- The end-to-end harness `crates/cli/tests/support/mod.rs` has `fake_agent_bin()` and `TestDaemon::start(script)`, which runs a real daemon with `fake-agent` as both runtimes.
- `proto::PROTO_VERSION` is 2.

If the merged code differs from these names, use the real names and record the mapping under "Implementation notes". Do not rename milestone-3 items.

## Goal

The sidebar becomes a tree. Windows are grouped under their project, and every Claude or Codex window shows its sub-agents below it, nested by parent, with live state and the current tool. A worktree agent appears under its repository's project. The user can walk, fold, filter and open the tree from the keyboard or the mouse, see a wide overview of it with `C-b T`, and change the sidebar width. `anthrex tree` prints the same tree as text or JSON for scripts.

## Scope

In scope:

- Project-root detection in the daemon, and `WindowInfo.project` on the wire. Protocol version 3.
- A pure client-side tree model in `crates/tui/src/tree.rs`: grouping, ordering, display names, sub-agent nesting, collapse, filter, selection, visible order and scrolling.
- Tree rendering in the sidebar at a default width of 34 columns, with render and hit-test sharing one geometry function.
- Tree mode (`C-b t`), the overview (`C-b T`), sidebar width (`C-b <`, `C-b >`), mouse clicks on tree rows, and wheel scrolling over the sidebar.
- `C-b j`, `C-b k` and `C-b 1..9` walk agent rows in the tree's visible order.
- `anthrex tree [--project <dir>] [--json]`.
- The milestone-4 follow-ups from `docs/superpowers/plans/2026-09-17-anthrex-foundation-followups.md`.

Out of scope:

- Run rows. They arrive with milestone 8. The tree model leaves an extension point for them (decision 26) and implements nothing of them.
- Persisting collapse state, sidebar width or the filter. The `ui.sidebar_width` and `ui.tree_keep_finished_secs` config keys belong to milestone 6, which wires them into this tree.
- Split panes (milestone 7). The overview draws into the single main area that exists today.
- Changing how milestone 3 computes status or sub-agents.

## Design decisions

### Project detection (daemon)

1. **Module.** Project-root detection lives in a new module, `crates/daemon/src/project.rs`. It has no dependency on the window manager.
2. **Algorithm.** Run `git -C <cwd> rev-parse --path-format=absolute --git-common-dir --show-toplevel` once. If git exits 0 with two lines of output, and the last component of the first line (the common dir) is `.git`, the root is the parent of the common dir. If git exits 0 and the common dir's last component is anything else, the root is the second line (the toplevel). In every other case, the root is the canonical `cwd`. The result is always passed through `std::fs::canonicalize`; if that fails, the uncanonicalized value is used.
3. **Why the toplevel fallback.** Verified with git 2.50.1: in a linked worktree the common dir is `<main>/.git`, so its parent is the main checkout, as spec section 3 requires. In a submodule the common dir is `<super>/.git/modules/<name>`, whose parent is not a checkout at all; the submodule's own toplevel is the right root there. In a bare repository and inside a `.git` directory, the command exits 128 because `--show-toplevel` needs a work tree, so those fall back to the canonical `cwd`.
4. **Git environment.** The git command runs with `stdin`, `stderr` set to null and `stdout` piped, and with `GIT_DIR`, `GIT_WORK_TREE`, `GIT_COMMON_DIR`, `GIT_INDEX_FILE` and `GIT_PREFIX` removed from its environment. A daemon started from inside a git hook would otherwise inherit them and resolve every window to that one repository.
5. **Timeout.** The blocking function spawns git and polls `Child::try_wait` every 10 ms until a 5-second deadline (`project::DETECT_TIMEOUT`). On timeout it calls `Child::kill`, then `Child::wait` to reap, and falls back to the canonical `cwd`. A missing `git` binary is a spawn error and also falls back.
6. **Detection runs before the window exists, off the lock.** The server resolves the project first, on `tokio::task::spawn_blocking`, and only then calls `WindowManager::create`, which receives the project as a parameter and never runs git. The window is therefore created with its final project, and `WindowInfo.project` never changes afterwards. The rejected alternative, creating the window under its canonical `cwd` and updating it later, would show every new window in a wrong project for a moment and then move its row, which breaks the selection and the `C-b 1..9` numbers the user is reading. It would also make "computed once" in spec section 3 false.
7. **The request loop does not wait for git.** The server's `CreateWindow` arm spawns a tokio task that awaits the detection, calls `manager.create`, and sends `Created` or `Error` through the connection's existing `out_tx`. The connection keeps serving `Input`, `Resize` and every other request while git runs. Normal detection takes milliseconds; the worst case, a hung git, delays only that `Created` reply by 5 seconds.
8. **Wire field.** `WindowInfo` gains `project: PathBuf`, placed directly after `cwd`. It is never empty. `PROTO_VERSION` becomes 3.

### Tree model (client)

9. **Pure module.** The tree model is `crates/tui/src/tree.rs`. It performs no I/O, like `app.rs` and `ui/` (`AGENTS.md` rule 5). The CLI reuses it through the `tui` crate, which `crates/cli` already depends on.
10. **Rows are rebuilt, never cached.** `tree::build(&windows, &state)` returns the visible rows every time it is called: once per frame by the renderer and once per update function that needs rows. Dozens of rows cost microseconds; a cache would be one more thing to invalidate.
11. **Identity is a key, not an index.** Selection and collapse state are stored as `NodeKey` values. Projects reorder when their urgency changes, so an index would jump to another row.
12. **Project display names.** The name is the root's last path component, or `/` for the root directory. If two or more projects share a name, each of them shows `<name> (<parent dir name>)`. If that still collides, each colliding project shows its full root path.
13. **Project order.** Projects are sorted by the urgency rank of their most urgent window, then by display name, then by root path. Urgency ranks: `Attention` 0, `Working` 1, `Starting` 2, `Done` 3, `Idle` 4, `Exited` 5. The project's status is the status with the lowest rank among its windows.
14. **Window order.** Inside a project, windows are sorted by `id` ascending. Ids are allocated in creation order, so this is creation order.
15. **Runtime counts.** A project row counts its windows per runtime and prints non-zero counts in the fixed order `cl`, `cx`, `sh`, joined by ` · `, for example `cl 4 · cx 3`. Spec section 5.1 item 1 lists the tags in this order; the sketch above it shows `cx` first, and the list wins.
16. **Sub-agent nesting.** Sub-agents of one window form a forest by `parent_id`. An entry whose `parent_id` is `None`, names an id not present in the list, or would close a cycle, is a root. Siblings are ordered oldest first: `started_secs` descending, then `id` ascending.
17. **Tree guides.** Each sub-agent row has a guide string: `│ ` for the window stem, then for every ancestor sub-agent level above it `│ ` if that ancestor has a later sibling, otherwise two spaces, then `├ ` if the row has a later sibling, otherwise `└ `. For example `│ ├ `, `│ └ `, `│ │ └ `.
18. **Collapse.** Project rows and window rows can be collapsed. Every node starts expanded. A collapsed project shows only its project row; a collapsed window hides its sub-agent rows. Sub-agent rows cannot be collapsed. While the filter is non-empty, collapse state is ignored so every match is visible.
19. **Filter.** The filter is a case-insensitive substring match against the project display name, the window name, and each sub-agent's `kind` and `label`. A project row is shown if the project, any of its windows or any of their sub-agents match. A window row is shown if its project matches, it matches, or any of its sub-agents match. A sub-agent row is shown if its project, its window, itself, an ancestor sub-agent or a descendant sub-agent matches.
20. **Sub-agent attention.** Spec section 4.4 rule 4: a sub-agent whose `SubagentInfo.needs_permission` is true is drawn with `◆` in `theme::status_color(Status::Attention)` in place of its state glyph, in the narrow and the wide format. The CLI text output prints `permission` in place of its state word, and the JSON output carries `needs_permission`. The window row shows `Attention` as milestone 3 computes it.
21. **Visible order.** Window rows are numbered 1, 2, 3 in the order they appear in the visible rows. That number is the position printed on the row and the one `C-b 1..9` uses. `C-b j` and `C-b k` move through the same order and wrap around. Windows in collapsed projects, or hidden by the filter, have no number and are skipped.
22. **Focused window hidden by collapse.** If the focused window is not in the visible order, `C-b j` focuses the first visible window whose position in the fully expanded order comes after the focused one, wrapping around; `C-b k` does the same backwards. If no window is visible, both do nothing.
23. **Selection repair.** After every rebuild caused by a window list update, a collapse or a filter change, if the selected key is no longer among the visible rows, the selection moves to the row now at the previously selected index, clamped to the last row, or to nothing when there are no rows. `TreeState` keeps that previous index.
24. **Scrolling.** Each tree view has a `Viewport { top, height }`. The height is written after every draw from the layout. After any change of selection, focus or rows, the anchor row is revealed: if its index is above `top`, `top` becomes the index; if it is at or below `top + height`, `top` becomes `index + 1 - height`. The anchor is the selected row in tree mode. Outside tree mode it is the focused window's row, or its project row when the project is collapsed. The wheel over the sidebar list scrolls `top` by 3 rows without moving the anchor.
25. **One geometry function.** `ui::tree_view::geometry(list, rows_len, top)` decides which rows are drawn where. The renderer draws exactly the rows it returns, and hit-testing maps a mouse position through the same value. Neither computes row positions on its own. This is the milestone-1 lesson from follow-up Task 15.
26. **Extension point for run rows.** `build` groups each project's members as a `Vec<ProjectChild>` before emitting rows. `ProjectChild` has one variant, `Window(&WindowInfo)`, and carries the comment `// Milestone 8 adds Run { .. }: run rows sit above plain windows and own their windows (spec 5.1 item 4).` `NodeKey` and `RowKind` carry the same comment. Every `Row` carries an `indent` in columns that the renderer applies without knowing the depth rule, so windows nested one level deeper under a run need no new render code. No `match` on `NodeKey`, `RowKind` or `ProjectChild` may use a `_` arm, so adding the run variant fails to compile everywhere it must be handled.

### Keys and modes

27. **Tree mode is a keymap mode.** `Keymap` gains a tree-mode flag. While it is set, the prefix key works exactly as before, and every other key that is not a release is returned as `KeyAction::Tree(key)` instead of being encoded for the PTY. `App` interprets those keys. The flag is changed only through `App::enter_tree` and `App::exit_tree`, which also set `App::tree_input`.
28. **New prefix commands.** `C-b t` toggles tree mode. `C-b T` toggles the overview. `C-b <` narrows and `C-b >` widens the sidebar. Matching uses `KeyCode` only, like the existing `C-b X` and `C-b Q`, so the Shift modifier some terminals report does not matter.
29. **Tree-mode keys, navigating.** `j` or `Down` moves the selection down, `k` or `Up` moves it up; neither wraps. `Enter` on a window row focuses that window and leaves tree mode. `Enter` on a sub-agent row focuses its window and leaves tree mode. `Enter` on a project row toggles its collapse. `Space` toggles the collapse of the selected project or window row, and does nothing on a sub-agent row. `/` starts filter input. `Esc` leaves tree mode, clears the filter and closes the overview. Every other key does nothing.
30. **Tree-mode keys, filter input.** Printable characters without Control or Alt append to the filter. `Backspace` removes the last character. `Enter` keeps the filter and returns to navigating. `Esc` clears the filter and returns to navigating. A paste appends its text with newlines removed. After each change, if the selected row is no longer visible, the first visible window row becomes selected, or the first row when no window row is visible.
31. **Paste in tree mode while navigating** is ignored. Text must never reach an agent while the user thinks they are driving the tree.
32. **Entering tree mode** makes the sidebar visible if it was hidden and selects the focused window's row when it is visible, otherwise the first row.
33. **Mouse.** A left click on a window row focuses it. A click on a sub-agent row focuses its window. A click on a project row toggles its collapse. In tree mode a click also selects the clicked row. A click in the overview behaves like `Enter` on that row.

### Overview and sidebar width

34. **Overview.** `C-b T` shows the tree in the main area in place of the terminal and enters tree mode. `Esc` or `C-b T` again closes it and leaves tree mode. The layout does not change, so the PTY is not resized. It uses its own `Viewport` and the wide row format in decision 38.
35. **Sidebar width.** The constant `SIDEBAR_WIDTH` in `crates/tui/src/ui/mod.rs` is replaced by `DEFAULT_SIDEBAR_WIDTH = 34`, `MIN_SIDEBAR_WIDTH = 24`, `MAX_SIDEBAR_WIDTH = 60` and `SIDEBAR_WIDTH_STEP = 4`. `App::sidebar_width` starts at 34. `C-b <` and `C-b >` change it by 4 within the limits and make the sidebar visible. The main area shrinks or grows, and the existing debounced `Resize` resizes the focused PTY.

### Rendering

36. **Sidebar.** The block keeps its rounded border and is titled ` agents `, as in the sketch in spec section 5.1. In tree mode the title is ` agents · tree ` and the border uses `theme::border_focused()`. The last inner row is the footer, the row above it is a spacer, and the rows above those are the tree list. The footer text is `N agent(s) · W working · A attention`, with zero parts left out; parts are dropped from the right until the text fits the width. The empty state keeps its two lines ` no agents yet` and ` C-b c opens a shell`.
37. **Narrow row formats**, at list width `W`. Widths are measured with `unicode-width`. A truncated field ends in `…`. There is at least one space between the left and right parts; when there is not enough room, the right part loses fields in the order given.
    - Project row: `{▾|▸} {display name}` on the left, bold. On the right, `{status glyph}  {counts}`, with the glyph in its status colour and the counts dimmed. If it does not fit, the counts go first.
    - Window row: `indent - 2` spaces, then the focus bar (`▎` in the accent colour for the focused window, else a space), then the collapse marker (`▸` when the window is collapsed and has sub-agents, else a space), then `{status glyph} {position} {name}`. The position is right-aligned to the width of the largest position shown. The name is bold when focused. On the right, dimmed: `{tag}` (`cl`, `cx` or `sh`), then ` {short model}` when the model is known, then a space and the elapsed time right-aligned in 3 columns. Fields drop in this order: model, elapsed.
    - Sub-agent row: `indent` spaces, the guide string, the state glyph, a space, then `{kind}: {label}`, or `{kind}` when there is no label. On the right, dimmed, the current tool. The tool is dropped when it does not fit.
    - The selected row, in tree mode only, is drawn with `Modifier::REVERSED` across the full width.
38. **Wide row format (overview and CLI text).** Project row: `{marker} {display name}  {root, with the home directory shortened to ~}` on the left; `{status glyph} {status label}  {counts}` on the right. Window row: `{bar}{marker}{glyph} {position} {name padded to the longest window name, at most 24}  {runtime label, 6 wide}  {full model or -, padded to the longest model, at most 28}  {status label, 9 wide}  {elapsed, 4 wide right-aligned}  {tool}`. Sub-agent row: `{indent}{guides}{glyph} {kind}: {label}  {model or -}  {state label}  {duration}  {tool}`.
39. **Short model.** For a Claude window, `short_model` strips a leading `claude-`, then cuts at the first `-` that is followed by a digit: `claude-opus-5` gives `opus`, `claude-sonnet-4-5` gives `sonnet`. For every other runtime it is the model unchanged. Either way it is cut to 8 columns.
40. **Glyphs.** Window glyphs and colours stay `theme::status_glyph` and `theme::status_color`. Sub-agent glyphs are new: `Running` uses the spinner in the working colour, `Done` is `✓` in the done colour, `Failed` is `✕` in `Color::Red`. `needs_permission` overrides all three with `◆` in the attention colour (decision 20).
41. **Durations.** Elapsed times keep the existing `format_elapsed` rules, which move from `ui/sidebar.rs` to `tree.rs`. A running sub-agent's duration is `started_secs` plus the age of the last window list; a finished one's is `started_secs - ended_secs`, saturating.

### CLI

42. **`anthrex tree [--project <dir>] [--json]`** connects without auto-starting the daemon, like `anthrex ls`. It builds the rows with an empty `TreeState`, so nothing is collapsed or filtered. `--project <dir>` canonicalizes the directory and keeps only the project whose root is the longest root that `dir` starts with, compared by path components. With no windows, or no matching project, text output prints `no windows` and JSON prints `{"projects":[]}`; the exit code is 0 in both cases.
43. **Text output** uses plain characters, no colour and no glyphs: status and state words instead. Its lines are given in the Interfaces section.
44. **JSON output** is one pretty-printed object in the shape given in the Interfaces section. Sub-agents are nested through `children`, and each still carries its `parent_id`. Status and state values are lowercase words. Milestone 8 adds a `runs` array to each project object.

## Interfaces

### Protocol (`crates/proto`)

```rust
// crates/proto/src/lib.rs
pub const PROTO_VERSION: u32 = 3;

// crates/proto/src/types.rs, WindowInfo gains one field after `cwd`:
pub struct WindowInfo {
    pub id: u32,
    pub name: String,
    pub runtime: Runtime,
    pub cwd: PathBuf,
    /// New in protocol 3. The project root from spec section 3; never empty.
    pub project: PathBuf,
    // ...every other field exactly as milestone 3 left it...
}
```

No message is added or removed.

### Daemon

```rust
// crates/daemon/src/project.rs (new)
pub const DETECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Blocking. Never fails: falls back to the canonical `cwd`, or `cwd` itself.
pub fn detect_root(cwd: &std::path::Path) -> std::path::PathBuf;

/// Blocking, with the git program and the timeout injectable for tests.
pub fn detect_root_with(git: &std::ffi::OsStr, cwd: &std::path::Path, timeout: std::time::Duration) -> std::path::PathBuf;

/// Runs `detect_root` on `spawn_blocking`. A `JoinError` falls back to `cwd` unchanged.
pub async fn resolve_root(cwd: std::path::PathBuf) -> std::path::PathBuf;
```

```rust
// crates/daemon/src/manager.rs, changed signature:
pub fn create(&self, spec: WindowSpec, project: PathBuf, cols: u16, rows: u16) -> anyhow::Result<WindowInfo>;
// `Entry` gains `project: PathBuf`; `Entry::info` copies it into `WindowInfo.project`.
```

`crates/daemon/src/lib.rs` gains `pub mod project;`.

### Tree model (`crates/tui/src/tree.rs`, new)

```rust
use proto::{Runtime, Status, SubagentInfo, WindowInfo};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum NodeKey {
    Project(PathBuf),
    Window(u32),
    Subagent { window_id: u32, id: String },
    // Milestone 8 adds Run(..): see decision 26.
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RuntimeCounts { pub claude: usize, pub codex: usize, pub shell: usize }

pub enum ProjectChild<'a> {
    Window(&'a WindowInfo),
    // Milestone 8 adds Run { .. }: run rows sit above plain windows and own their windows (spec 5.1 item 4).
}

#[derive(Debug, Clone, PartialEq)]
pub enum RowKind<'a> {
    Project { root: &'a Path, name: String, status: Status, counts: RuntimeCounts, collapsed: bool },
    Window { info: &'a WindowInfo, position: usize, has_subagents: bool, collapsed: bool },
    Subagent { window: &'a WindowInfo, info: &'a SubagentInfo, guides: String },
    // Milestone 8 adds Run { .. }.
}

#[derive(Debug, Clone, PartialEq)]
pub struct Row<'a> {
    pub key: NodeKey,
    /// Columns of indentation before the row's own content. Project 0, window 2,
    /// sub-agent 2 (its guide string starts with the window stem).
    pub indent: u16,
    pub kind: RowKind<'a>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Viewport { pub top: usize, pub height: u16 }

impl Viewport {
    pub fn reveal(&mut self, index: usize);          // decision 24
    pub fn scroll(&mut self, delta: isize, len: usize); // clamps to 0..=len.saturating_sub(height)
}

#[derive(Debug, Clone, Default)]
pub struct TreeState {
    pub collapsed: HashSet<NodeKey>,
    pub filter: String,
    pub selected: Option<NodeKey>,
    pub sidebar: Viewport,
    pub overview: Viewport,
    selected_index: usize, // for selection repair, decision 23
}

impl TreeState {
    pub fn is_collapsed(&self, key: &NodeKey) -> bool;
    /// Toggles a Project or Window key; returns false and does nothing for any other key.
    pub fn toggle(&mut self, key: &NodeKey) -> bool;
    pub fn select(&mut self, rows: &[Row], key: NodeKey);
    /// Moves by `delta` rows, clamped, no wrap. Selects the first row when nothing is selected.
    pub fn move_selection(&mut self, rows: &[Row], delta: isize);
    pub fn repair_selection(&mut self, rows: &[Row]);
    pub fn selected_index(&self, rows: &[Row]) -> Option<usize>;
    /// Drops collapse keys whose project or window no longer exists.
    pub fn prune(&mut self, windows: &[WindowInfo]);
}

pub fn build<'a>(windows: &'a [WindowInfo], state: &TreeState) -> Vec<Row<'a>>;
pub fn agent_order(rows: &[Row]) -> Vec<u32>;
pub fn row_index(rows: &[Row], key: &NodeKey) -> Option<usize>;
pub fn display_names<'a>(roots: impl IntoIterator<Item = &'a Path>) -> std::collections::HashMap<PathBuf, String>;
pub fn urgency(status: Status) -> u8;
pub fn runtime_tag(runtime: Runtime) -> &'static str; // "cl", "cx", "sh"
pub fn short_model(runtime: Runtime, model: &str) -> String;
pub fn format_elapsed(secs: u64) -> String; // moved from ui/sidebar.rs, unchanged

pub struct SubagentNode<'a> { pub info: &'a SubagentInfo, pub children: Vec<SubagentNode<'a>> }
pub fn subagent_forest(subagents: &[SubagentInfo]) -> Vec<SubagentNode<'_>>; // decision 16
```

`build` uses `subagent_forest` to emit sub-agent rows, and the CLI uses it for JSON `children`.

### Client state and keys (`crates/tui`)

```rust
// crates/tui/src/keymap.rs
pub enum Command {
    // ...existing variants...
    ToggleTree,     // C-b t
    ToggleOverview, // C-b T
    NarrowSidebar,  // C-b <
    WidenSidebar,   // C-b >
}
pub enum KeyAction {
    // ...existing variants...
    /// A key pressed in tree mode; `App` interprets it.
    Tree(crossterm::event::KeyEvent),
}
impl Keymap {
    pub fn set_tree_mode(&mut self, on: bool);
    pub fn tree_mode(&self) -> bool;
}

// crates/tui/src/app.rs
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TreeInput { Navigate, Filter }

pub struct App {
    // ...existing fields...
    pub tree: tree::TreeState,
    /// None outside tree mode.
    pub tree_input: Option<TreeInput>,
    pub overview: bool,
    pub sidebar_width: u16,
}
impl App {
    pub fn rows(&self) -> Vec<tree::Row<'_>>;
    pub fn enter_tree(&mut self);
    pub fn exit_tree(&mut self); // also clears the filter and closes the overview
    /// Called after every draw with the list heights from the layout.
    pub fn set_tree_viewports(&mut self, sidebar_rows: u16, overview_rows: u16);
    /// A left click anywhere; hit-tests the sidebar list and, when shown, the overview.
    pub fn on_click(&mut self, column: u16, row: u16, layout: &crate::ui::Layout) -> Vec<Effect>;
    /// Seconds value from the last window list, plus the time since it arrived.
    pub fn age_secs(&self, secs: u64) -> u64;
}

// crates/tui/src/tree_input.rs (new, pure): tree-mode key and paste handling.
impl App {
    pub(crate) fn on_tree_key(&mut self, key: crossterm::event::KeyEvent) -> Vec<Effect>;
    pub(crate) fn on_tree_paste(&mut self, text: String) -> Vec<Effect>;
}
```

### Rendering (`crates/tui/src/ui`)

```rust
// crates/tui/src/ui/mod.rs
pub const DEFAULT_SIDEBAR_WIDTH: u16 = 34;
pub const MIN_SIDEBAR_WIDTH: u16 = 24;
pub const MAX_SIDEBAR_WIDTH: u16 = 60;
pub const SIDEBAR_WIDTH_STEP: u16 = 4;

pub struct Layout {
    pub sidebar: Rect,
    pub sidebar_inner: Rect,
    /// New: the rows the tree is drawn into (inner minus spacer and footer).
    pub sidebar_list: Rect,
    /// New: the footer row.
    pub sidebar_footer: Rect,
    pub main: Rect,
    pub main_inner: Rect,
    pub statusbar: Rect,
}
/// `sidebar_width` 0 means hidden.
pub fn layout(area: Rect, sidebar_width: u16) -> Layout;

// crates/tui/src/ui/tree_view.rs (new)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TreeGeometry { pub list: Rect, pub first: usize, pub count: usize }
/// first = min(top, rows_len - list.height) saturating; count = min(list.height, rows_len - first).
pub fn geometry(list: Rect, rows_len: usize, top: usize) -> TreeGeometry;
impl TreeGeometry {
    /// Row index under a screen position, or None outside the drawn rows.
    pub fn index_at(&self, column: u16, row: u16) -> Option<usize>;
}
pub fn narrow_line(app: &App, row: &tree::Row, width: u16, pos_width: usize, selected: bool) -> Line<'static>;
pub fn wide_line(app: &App, row: &tree::Row, width: u16, columns: &WideColumns, selected: bool) -> Line<'static>;

// crates/tui/src/ui/overview.rs (new): renders the overview into the main area.
pub fn render(frame: &mut Frame, app: &App, area: Rect);

// crates/tui/src/theme.rs, new:
/// `◆` when `info.needs_permission`, else the state glyph (decisions 20 and 40).
pub fn subagent_glyph(info: &SubagentInfo, spinner_frame: usize) -> &'static str;
pub fn subagent_color(info: &SubagentInfo) -> Color;
```

`ui::sidebar::hit_test`, `ui::sidebar::CARD_HEIGHT` and `ui::sidebar::format_elapsed` are removed.

### Keys

| Keys | Action |
|------|--------|
| `C-b t` | Enter tree mode; again to leave |
| `C-b T` | Open the overview in the main area; again to close |
| `C-b <` / `C-b >` | Sidebar width minus or plus 4 columns, between 24 and 60 |
| `j` / `Down`, `k` / `Up` (tree mode) | Move the selection |
| `Enter` (tree mode) | Window or sub-agent row: focus the window and leave tree mode. Project row: toggle collapse |
| `Space` (tree mode) | Toggle collapse of the selected project or window |
| `/` (tree mode) | Start filter input |
| `Esc` (tree mode) | Filter input: clear the filter. Navigating: leave tree mode |

The help overlay (`HELP` in `crates/tui/src/ui/modal.rs`) gains `C-b t` "tree mode (j/k, Enter, Space, /)", `C-b T` "tree overview", and `C-b < / >` "sidebar width". The statusbar `HINTS` become `C-b ? help`, `C-b c new shell`, `C-b t tree`, `C-b j/k switch`, `C-b d detach`.

### CLI

```
anthrex tree [--project <dir>] [--json]
```

Text output, for each project in tree order:

```
<display name>  <root>  <status>  <counts>
  <position>  <name>  <runtime>  <model or ->  <status>  <elapsed>[  <tool>]
      <guides><kind>[: <label>]  <state>  <duration>[  <tool>]
```

The position is right-aligned to 2 columns. The sub-agent guides are the model's guide string without the leading window stem `│ `. Example:

```
shop  /Users/me/repos/shop  attention  cl 4 · cx 3
   1  api-worker  claude  claude-opus-5  working  2m  Bash
      ├ Explore: map routes  running  1m  Read
      │ └ general-purpose: grep handlers  running  20s
      └ tests: run unit suite  done  45s
   2  billing  codex  -  attention  41s
```

JSON output:

```json
{
  "projects": [
    {
      "root": "/Users/me/repos/shop",
      "name": "shop",
      "status": "attention",
      "counts": { "claude": 4, "codex": 3, "shell": 0 },
      "windows": [
        {
          "id": 1,
          "position": 1,
          "name": "api-worker",
          "runtime": "claude",
          "model": "claude-opus-5",
          "status": "working",
          "since_secs": 120,
          "tool": "Bash",
          "cwd": "/Users/me/repos/shop",
          "branch": null,
          "session_id": "8f2c...",
          "subagents": [
            {
              "id": "a1",
              "parent_id": null,
              "kind": "Explore",
              "label": "map routes",
              "model": null,
              "state": "running",
              "tool": "Read",
              "started_secs": 90,
              "ended_secs": null,
              "needs_permission": false,
              "children": []
            }
          ]
        }
      ]
    }
  ]
}
```

`runtime` uses `Runtime::label()`, `status` uses `Status::label()`, and `state` is `running`, `done` or `failed`, mapped by the CLI itself rather than by serde attributes. In the text output a sub-agent with `needs_permission` prints `permission` in place of its state word (decision 20). Paths are printed with `Path::display()`. The structs are `TreeJson`, `ProjectJson`, `CountsJson`, `WindowJson` and `SubagentJson`, all new, in `crates/cli/src/tree_cmd.rs`, deriving `serde::Serialize`.

### Dependencies

- Workspace `Cargo.toml`: add `unicode-width = "0.2"` to `[workspace.dependencies]`. It is already in the tree at 0.2.2 through ratatui 0.30.2.
- `crates/tui/Cargo.toml`: add `unicode-width.workspace = true`.
- `crates/cli/Cargo.toml`: add `serde.workspace = true` to `[dependencies]` (milestone 3 already added `serde_json`), and `serde_json.workspace = true` to the `[dev-dependencies]` section that milestone 3 created with `tempfile` and `tokio`.

## Tasks

### M4.1 Project-root detection module

**Files.** Create `crates/daemon/src/project.rs` and `crates/daemon/tests/project.rs`. Modify `crates/daemon/src/lib.rs`.

**Tests first**, in `crates/daemon/tests/project.rs`. A helper `git(dir, args)` runs git with `-c user.name=t -c user.email=t@t -c commit.gpgsign=false -c init.defaultBranch=main`, the environment variable `GIT_CONFIG_NOSYSTEM=1`, and asserts success. A helper `repo()` creates a temporary directory, runs `git init`, and makes one empty commit. Every expected path is compared after `canonicalize()`, because macOS temp directories live behind the `/var` to `/private/var` symlink.

- `plain_directory_is_its_own_root`: a fresh temp directory that is not a repository. `detect_root(dir)` equals `dir.canonicalize()`.
- `repository_root_is_the_checkout`: `detect_root(repo)` equals the repo.
- `subdirectory_maps_to_the_repository_root`: create `repo/a/b`; `detect_root(repo/a/b)` equals the repo.
- `linked_worktree_maps_to_the_main_checkout`: `git worktree add -b feat <tmp>/wt` from the repo. `detect_root(<tmp>/wt)` and `detect_root(<tmp>/wt/sub)` (created) both equal the main repo, not the worktree.
- `submodule_maps_to_its_own_checkout`: create a second repo, add it to the first with `git -c protocol.file.allow=always submodule add <second> mods`. `detect_root(repo/mods)` equals `repo/mods`.
- `bare_repository_falls_back_to_the_directory`: `git init --bare x.git`; `detect_root(x.git)` equals `x.git` canonicalized.
- `missing_directory_is_returned_unchanged`: `detect_root("/definitely/missing/dir")` returns that path.
- `missing_git_falls_back`: `detect_root_with("/nonexistent/git", repo/a, DETECT_TIMEOUT)` equals `repo/a` canonicalized.
- `hanging_git_times_out`: write an executable script `#!/bin/sh` / `sleep 10` into a temp dir. `detect_root_with(script, repo, 300 ms)` returns the canonical repo path, and the call takes less than 2 seconds.
- `resolve_root_finds_the_repository`: a `#[tokio::test]` that awaits `resolve_root(repo/a)` and gets the repo root.
- `resolve_root_does_not_block_the_runtime`: a `#[tokio::test(flavor = "current_thread")]`. Spawn a task that sleeps 10 ms and then records `Instant::now()`. Await `resolve_root_with(hanging_script, repo, 500 ms)` and record when it returns. The sleeping task must have finished first. On a current-thread runtime that is only possible if the git wait ran off the runtime thread. `resolve_root_with(git, cwd, timeout)` is an async twin of `detect_root_with`; make it `pub` but `#[doc(hidden)]`, because integration tests cannot see `pub(crate)` items.

**Change.** Implement decisions 1 to 5 and the functions in the Interfaces section. Parse git's output as bytes with `std::os::unix::ffi::OsStrExt` so non-UTF-8 paths survive. Take `stdout` from the child, and read it to the end only after `try_wait` reports an exit. Log the fallback reason at debug level.

**Acceptance.** All eleven tests pass. `project.rs` never touches the manager. No call in it can block longer than the timeout plus the time to kill and reap git.

### M4.2 `WindowInfo.project` on the wire and in the daemon

**Files.** Modify `crates/proto/src/lib.rs`, `crates/proto/src/types.rs`, `crates/proto/src/messages.rs`, `crates/daemon/src/manager.rs`, `crates/daemon/src/server.rs`, `crates/daemon/tests/manager.rs`, `crates/daemon/tests/server.rs`, and every other file with a `WindowInfo { .. }` literal: today `crates/tui/src/app.rs`, `crates/tui/src/ui/mod.rs` and `crates/cli/src/client.rs`, plus whatever milestone 3 added. Find them with `rg -n 'WindowInfo \{' crates`.

**Tests first.**

- `crates/proto/src/types.rs`: extend `window_info_round_trips_through_json` to set `project: "/tmp/repo".into()` and assert the JSON contains `"project":"/tmp/repo"`.
- `crates/proto/src/messages.rs`: new `windows_changed_carries_the_project_through_messagepack`. A `DaemonMsg::WindowsChanged` with one `WindowInfo` whose `project` differs from its `cwd` round-trips through `rmp_serde::to_vec_named` unchanged.
- `crates/proto/src/lib.rs`: new `protocol_version_is_3`, asserting `PROTO_VERSION == 3`.
- `crates/daemon/tests/server.rs`: new `created_windows_carry_their_project_root`. Build a temp repo with a linked worktree as in M4.1 (copy the small helper). Create a shell window with `cwd` at `repo/sub` and another at the worktree. Wait for a `WindowsChanged` that lists both. Assert both have `project == repo.canonicalize()`, and that their `cwd` values are unchanged from the specs.
- `crates/daemon/tests/server.rs`: new `a_window_outside_any_repository_is_its_own_project`, with a plain temp directory.
- `crates/daemon/tests/manager.rs`: new `create_records_the_given_project`. `m.create(spec("p"), "/some/root".into(), 80, 24)` returns an info with `project == "/some/root"`, and `m.list()` agrees.

**Change.** Add the field and bump the version (decision 8). Add `project` to `Entry` and to `create` (Interfaces). In `server.rs`, change the `CreateWindow` arm as in decision 7: clone `manager` and `out_tx`, spawn a task that runs `project::resolve_root(spec.cwd.clone()).await`, then `manager.create(spec, project, cols, rows)`, and sends `Created` or `Error { request: "create", .. }`. The arm itself returns `None`. Update every existing `create` call in tests to pass a project, `std::env::temp_dir()` where nothing depends on it. Update every `WindowInfo` literal. The TUI, the CLI and `anthrex hook` all send `proto::PROTO_VERSION`, so rebuilding them is the client update `AGENTS.md` rule 4 asks for; check that no client hard-codes a version.

**Acceptance.** `cargo test --workspace` passes. A client built before this change is refused by the handshake with the existing mismatch message.

### M4.3 Tree model: grouping, ordering and nesting

**Files.** Create `crates/tui/src/tree.rs`. Modify `crates/tui/src/lib.rs` (`pub mod tree;`), `crates/tui/src/ui/sidebar.rs` (use `tree::format_elapsed`, delete its own), `crates/tui/src/ui/mod.rs` (its test calls `tree::format_elapsed`), the workspace `Cargo.toml` and `crates/tui/Cargo.toml` (`unicode-width`).

**Tests first**, as unit tests in `tree.rs`. A fixture function `example()` builds the user's example from spec section 5.1:

- Project `/r/shop`, windows in id order:
  1. `api-worker`, Claude, model `claude-opus-5`, `Working`, `since_secs` 120, sub-agents: `a1` kind `Explore`, label `map routes`, `Running`, tool `Read`, `started_secs` 90; `a2` kind `general-purpose`, label `grep handlers`, parent `a1`, `Running`, started 20; `a3` kind `tests`, label `run unit suite`, `Done`, started 60, ended 15.
  2. `billing`, Codex, `Attention`, 41.
  3. `search`, Codex, `Idle`, 300.
  4. `frontend`, Claude, model `claude-sonnet-4-5`, `Working`, 60, sub-agents: `b1` kind `general-purpose`, label `style pass`, `Running`, tool `Edit`, started 50; `b2` kind `Explore`, label `find tokens`, parent `b1`, `Running`, started 30; `b3` kind `Explore`, label `list files`, parent `b2`, `Done`, started 25, ended 5.
  5. `docs`, Claude, `Done`, 180.
  6. `infra`, Codex, `Idle`, 720.
  7. `perf`, Claude, `Starting`, 2.
- Project `/r/blog`: window 8 `notes`, Claude, `Idle`, 30.

That is three Codex and four Claude windows in one project, two of them with nested sub-agents.

- `example_rows_in_order`: `build(&example(), &TreeState::default())` yields keys in exactly this order: `Project(/r/shop)`, `Window(1)`, `Subagent(1,a1)`, `Subagent(1,a2)`, `Subagent(1,a3)`, `Window(2)`, `Window(3)`, `Window(4)`, `Subagent(4,b1)`, `Subagent(4,b2)`, `Subagent(4,b3)`, `Window(5)`, `Window(6)`, `Window(7)`, `Project(/r/blog)`, `Window(8)`.
- `project_row_has_status_counts_and_name`: the shop row has name `shop`, status `Attention`, counts `claude 4, codex 3, shell 0`. The blog row has status `Idle`, counts `claude 1`.
- `positions_follow_visible_order`: window rows 1 to 8 have positions 1 to 8, and `agent_order` is `[1, 2, 3, 4, 5, 6, 7, 8]`.
- `guides_draw_the_nesting`: `a1` has guides `│ ├ `, `a2` has `│ │ └ `, `a3` has `│ └ `, `b1` has `│ └ `, `b2` has `│   └ `, `b3` has `│     └ `. Every row's `indent` is 0 for projects and 2 for windows and sub-agents.
- `projects_sort_by_urgency_then_name`: three projects whose most urgent statuses are `Idle` (`/x/alpha`), `Working` (`/x/zeta`) and `Working` (`/x/beta`) come out as `beta`, `zeta`, `alpha`. A project containing only `Exited` windows sorts last.
- `windows_sort_by_id_inside_a_project`: windows given in the order 5, 2, 9 come out as 2, 5, 9.
- `orphans_and_cycles_become_roots`: a sub-agent whose `parent_id` names a missing id, and two that name each other, all appear, each exactly once, at root level.
- `siblings_sort_oldest_first`: three roots with `started_secs` 10, 50, 30 come out 50, 30, 10.
- `display_names_disambiguate`: roots `/w/work/api`, `/w/oss/api`, `/w/web` give `api (work)`, `api (oss)`, `web`. Roots `/a/x/api` and `/b/x/api` give their full paths. Root `/` gives `/`.
- `short_model_shortens_claude_ids_only`: `claude-opus-5` gives `opus`, `claude-sonnet-4-5` gives `sonnet`, `claude-haiku-4-5-20251001` gives `haiku`; for Codex, `gpt-5.1-codex-max` gives `gpt-5.1-` (cut to 8 columns).
- `urgency_order_matches_the_spec`: ranks as in decision 13.

**Change.** Implement the pure functions, the `NodeKey`, `ProjectChild`, `RowKind` and `Row` types, and `build` without collapse or filter handling yet. `TreeState` can start with only the fields `build` reads. Follow decisions 11 to 17 and 26.

**Acceptance.** The tests pass. `tree.rs` has no `use` of `std::fs`, `std::process`, `tokio` or `crate::connection`.

### M4.4 Tree model: collapse, filter, selection and scrolling

**Files.** Modify `crates/tui/src/tree.rs`.

**Tests first**, unit tests in `tree.rs`, using `example()`.

- `collapsed_project_shows_only_its_row`: collapse `Project(/r/blog)`. The blog project row is present with `collapsed: true`, `Window(8)` is gone, and `agent_order` is `[1..=7]`.
- `collapsed_window_hides_its_subagents`: collapse `Window(1)`. Rows `a1`, `a2`, `a3` are gone; `Window(1)` has `collapsed: true` and `has_subagents: true`; positions are unchanged.
- `toggle_only_accepts_projects_and_windows`: `toggle` returns true and flips state for a project and a window key, and returns false without changing anything for a sub-agent key.
- `filter_matches_subagents_and_keeps_their_ancestors`: filter `STYLE`. Rows are exactly `Project(/r/shop)`, `Window(4)`, `Subagent(4,b1)`, `Subagent(4,b2)`, `Subagent(4,b3)`. `Window(4)` has position 1 and `agent_order` is `[4]`.
- `filter_on_a_project_shows_all_its_windows`: filter `blog` gives `Project(/r/blog)` and `Window(8)`.
- `filter_ignores_collapse`: collapse `Project(/r/shop)`, set filter `billing`. `Window(2)` is visible.
- `move_selection_clamps_without_wrapping`: from nothing, `move_selection(+1)` selects the first row; from the last row, `+1` keeps it; from the first, `-1` keeps it.
- `selection_is_repaired_by_index`: select `Window(3)` (index 6). Rebuild with window 3 removed. `repair_selection` selects the row now at index 6, which is `Window(4)`. With every window removed, the selection becomes `None`.
- `selection_follows_its_key_when_projects_reorder`: select `Window(8)`. Change window 8 to `Attention` and window 2 to `Idle`, so blog sorts first. After repair, the selection is still `Window(8)`, at index 1.
- `viewport_reveal_scrolls_minimally`: a viewport with height 5 and top 0. `reveal(3)` keeps top 0. `reveal(7)` sets top 3. `reveal(1)` sets top 1.
- `viewport_scroll_clamps`: height 5, 16 rows. `scroll(+30)` gives top 11. `scroll(-30)` gives top 0.
- `prune_drops_vanished_keys`: collapse `Window(8)` and `Project(/r/blog)`, prune with window 8 removed. Both keys are gone.

**Change.** Implement decisions 18, 19, 21, 23 and 24 inside `build`, `TreeState` and `Viewport`. Positions are assigned while emitting rows, so they always match the visible order.

**Acceptance.** The tests pass, and the M4.3 tests still pass unchanged.

### M4.5 Sidebar rendering, geometry and hit-testing

**Files.** Create `crates/tui/src/ui/tree_view.rs`. Modify `crates/tui/src/ui/mod.rs`, `crates/tui/src/ui/sidebar.rs`, `crates/tui/src/theme.rs`, `crates/tui/src/app.rs`, `crates/tui/src/lib.rs`, `scripts/pty-smoke.py`.

**Tests first**, in the `tests` module of `crates/tui/src/ui/mod.rs`. Render with `ratatui::backend::TestBackend` as the existing `render` helper does. Make the M4.3 fixture `#[cfg(test)] pub(crate) fn example_windows()` in `tree.rs` so the modules share it. An `example_app()` helper builds `App::new(example_windows(), "/tmp".into(), Keymap::default_prefix())` and calls `set_terminal_size(80, 24)`, which focuses window 1.

- `layout_uses_the_sidebar_width`: `layout(Rect(0,0,120,40), 34)` has `sidebar.width == 34`, `main.x == 34`, `sidebar_list.height == 38 - 2 - 2`, and `sidebar_footer.y == 38 - 1 - 1`. `layout(.., 0)` has an empty sidebar and a 120-wide main.
- `sidebar_renders_the_example_tree_at_the_default_width`: render at 120 by 30 with the default width. The output contains these exact strings, each 32 columns, the full inner width:
  - `▾ shop            ◆  cl 4 · cx 3`
  - `▎ ⠋ 1 api-worker     cl opus  2m`
  - `  │ ├ ⠋ Explore: map routes Read`
  - `▾ blog                   ○  cl 1` (19 spaces between `blog` and `○`).

  It also contains `│ └ ✓ tests: run unit suite`, ` ◆ 2 billing`, and the footer `8 agents · 2 working`, and it does not contain `attention` in the footer row.
- `a_sub_agent_asking_for_permission_shows_a_diamond`: set `needs_permission` on `a1` in the example. The sidebar contains `│ ├ ◆ Explore: map routes`, the `◆` cell's foreground is `theme::status_color(Status::Attention)`, and the `a3` row still shows `✓`. With `needs_permission` false again, `a1` shows the spinner.
- `long_names_are_truncated_with_an_ellipsis`: a window named `a-very-long-window-name-that-overflows` renders one line ending in the right part `sh  0s`, and the name ends in `…`.
- `hit_test_uses_the_render_geometry`: with the example at a height where the list has 10 rows, call `set_tree_viewports(10, ..)`, then set `app.tree.sidebar.top = 4` directly and render. For every list row `y`, `geometry(..).index_at(x, y)` returns `Some(4 + (y - list.y))`, and the rendered text on that screen row belongs to the row with that index. Rows in the spacer, the footer, the border, and columns outside the list return `None`.
- `sidebar_scrolls_to_keep_the_focused_window_visible`: 20 shell windows in one project, terminal height 14. Focus window 20. After `set_tree_viewports` and a render, the output contains `20 shell-20` and does not contain ` 1 shell-1 `.
- `wheel_over_the_sidebar_scrolls_the_tree`: with 20 windows and a small height, `app.on_scroll(false, x, y, ..)` with the position inside `sidebar_list` increases `tree.sidebar.top` by 3, and a position inside `main_inner` still goes to the terminal as before.
- `a_click_on_a_row_focuses_toggles_or_focuses_the_parent`: via `App::on_click` with the rendered layout: a click on the `api-worker` row emits nothing new because it is focused; on the `billing` row it emits `Subscribe` for window 2; on the `a1` row, after focusing window 2, it emits `Subscribe` for window 1; on the blog project row it collapses blog.
- Replace `sidebar_lists_cards_with_glyph_runtime_status_and_elapsed`, `sidebar_hit_test_maps_rows_to_cards`, `sidebar_hit_test_ignores_footer_and_undrawn_cards` and `layout_splits_sidebar_main_and_statusbar` with the tests above. Keep `empty_state_and_hidden_sidebar`, `statusbar_shows_prefix_state_and_toast` and `modals_render_on_top` passing.

**Change.** Implement decisions 20, 25, 35 (constants and `App::sidebar_width` only; the keys come in M4.7), 36, 37, 39, 40 and 41. `layout` computes `sidebar_list` and `sidebar_footer`. `sidebar::render` builds rows with `app.rows()`, calls `tree_view::geometry(layout.sidebar_list, rows.len(), app.tree.sidebar.top)`, and draws exactly `first..first + count`. Add `App::set_tree_viewports` and call it from `draw` in `crates/tui/src/lib.rs` right after `set_terminal_size`, with `sidebar_list.height` and `main_inner.height`. Add `App::on_click` and use it for `MouseEventKind::Down(MouseButton::Left)` in `event_loop`, in place of the `ui::sidebar::hit_test` call. Extend `App::on_scroll` for the sidebar list. After `focus` and after `replace_windows`, reveal the anchor (decision 24). In `scripts/pty-smoke.py`, stages 1, 6 and 7 wait for the text `anthrex`, which was the sidebar title; change those waits to `agents`.

**Acceptance.** The tests pass. `python3 scripts/pty-smoke.py` passes. No code outside `tree_view.rs` computes which tree row sits on which screen line.

### M4.6 Navigation in visible order

**Files.** Modify `crates/tui/src/app.rs`. Create `crates/tui/src/app_tests.rs`.

`app.rs` is 595 lines before this milestone. First move its `#[cfg(test)] mod tests` into `crates/tui/src/app_tests.rs`, included from `app.rs` with `#[cfg(test)] #[path = "app_tests.rs"] mod tests;`, without changing any test. Then add the new tests there.

**Tests first**, in `crates/tui/src/app_tests.rs`.

- `next_and_previous_follow_the_tree_order`: windows 1 (project `/p/b`, `Idle`), 2 (project `/p/a`, `Idle`), 3 (project `/p/b`, `Idle`). The tree order is 2, 1, 3. Starting on window 2, `C-b j` focuses 1, then 3, then wraps to 2. `C-b k` from 2 focuses 3.
- `number_keys_use_visible_positions`: in the same setup, `C-b 1` focuses 2 and `C-b 3` focuses 3. After collapsing `/p/a`, `C-b 1` focuses 1.
- `first_focus_is_the_first_visible_window`: `App::new` with the windows above, then `set_terminal_size`, subscribes to window 2.
- `next_from_a_hidden_focused_window_goes_forward`: focus window 1, collapse `/p/b`. The visible order is only 2. `C-b j` focuses 2. Expand `/p/b`, focus 3, collapse `/p/b`: the visible order is only 2, and `C-b k` focuses 2. Collapse `/p/a` as well: `C-b j` and `C-b k` do nothing.
- `removing_the_focused_window_focuses_the_same_position`: focus the window at position 2, remove it from the list. The window now at position 2 is focused, or the last one when it was last.
- The existing tests `keys_go_to_the_focused_window_and_prefix_switches` and `removed_focused_window_moves_focus_to_a_neighbour` must still pass. All their windows share the project `/tmp` in the fixture, so the tree order equals the id order.

**Change.** Use `tree::agent_order(&self.rows())` in `focus_relative`, `Command::FocusIndex`, `ensure_focus` and the neighbour rule in `replace_windows`. Implement decision 22. Call `self.tree.prune(&self.windows)` and `repair_selection` in `replace_windows`.

**Acceptance.** The tests pass. `app.rs` is under 600 lines.

### M4.7 Keymap tree mode, new prefix commands and sidebar width

**Files.** Modify `crates/tui/src/keymap.rs`, `crates/tui/src/app.rs`, `crates/tui/src/app_tests.rs`.

**Tests first.**

- In `keymap.rs`: `prefix_opens_tree_overview_and_width_commands`. `C-b t`, `C-b T` (with and without `SHIFT`), `C-b <` and `C-b >` return `Run(ToggleTree)`, `Run(ToggleOverview)`, `Run(NarrowSidebar)` and `Run(WidenSidebar)`.
- In `keymap.rs`: `tree_mode_returns_keys_instead_of_bytes`. With `set_tree_mode(true)`, `j`, `Enter`, `Esc` and `/` return `KeyAction::Tree` with that key; a release still returns `Nothing`; `C-b` still returns `AwaitPrefix`, `C-b j` still returns `Run(NextWindow)`, and `C-b C-b` still sends `0x02`. With `set_tree_mode(false)`, `j` returns `Send(b"j")` again.
- In `app_tests.rs`: `tree_toggle_enters_and_leaves_tree_mode`. `C-b t` sets `tree_input == Some(Navigate)`, `keymap.tree_mode()`, and selects the focused window's row. A second `C-b t` clears all three. With the sidebar hidden by `C-b s`, `C-b t` shows it.
- In `app_tests.rs`: `sidebar_width_steps_by_four_within_limits`. From 34, `C-b <` gives 30, then 26, 24, 24. `C-b >` from 56 gives 60, then 60. A change with the sidebar hidden shows it.
- In `app_tests.rs`: `keys_in_tree_mode_never_reach_the_pty`. In tree mode, `j`, `q`, `Enter` and a paste produce no `Input` effect.

**Change.** Implement decisions 27, 28, 32 and 35. `App::on_key` routes `KeyAction::Tree(key)` to `on_tree_key`, which in this task only handles `Esc` (leave tree mode); M4.8 completes it. `App::on_paste` routes to `on_tree_paste` while `tree_input` is set. Create `crates/tui/src/tree_input.rs` with those two methods and declare it in `lib.rs`.

**Acceptance.** The tests pass. The existing keymap tests pass unchanged.

### M4.8 Tree-mode interaction

**Files.** Modify `crates/tui/src/tree_input.rs`, `crates/tui/src/app_tests.rs`, `crates/tui/src/ui/statusbar.rs`, `crates/tui/src/ui/modal.rs`, `crates/tui/src/ui/mod.rs` (tests).

**Tests first**, in `app_tests.rs` unless noted, with the example windows.

- `j_and_k_move_the_selection_without_wrapping`: in tree mode on window 1, `j` selects `a1`, `k` twice selects the shop project row, `k` again keeps it.
- `enter_on_a_window_focuses_it_and_leaves_tree_mode`: select `Window(2)`, `Enter`. The effects are `Subscribe` for window 2; `tree_input` is `None`.
- `enter_on_a_subagent_focuses_its_window`: focus window 2, enter tree mode, select `Subagent(4,b2)`, `Enter`. `Subscribe` for window 4.
- `enter_and_space_toggle_projects_space_toggles_windows`: `Enter` on the blog row collapses it and stays in tree mode. `Space` on `Window(1)` collapses it. `Space` on `Subagent(1,a1)` changes nothing.
- `filter_input_edits_and_selects_the_first_match`: `/`, then `s`, `t`, `y`, `l`, `e`. `tree_input == Some(Filter)`, `tree.filter == "style"`, and the selection is `Window(4)`. `Backspace` gives `styl`. `Enter` returns to `Navigate` and keeps the filter. `/`, then `Esc`, clears the filter and returns to `Navigate`.
- `escape_while_navigating_leaves_tree_mode_and_clears_the_filter`.
- `paste_goes_to_the_filter_only_while_typing_it`: in filter input, pasting `"to\nkens"` makes the filter `tokens`. While navigating, a paste changes nothing and emits nothing.
- `selection_stays_visible_while_moving`: with 20 windows and a sidebar viewport height of 5 (via `set_tree_viewports(5, 20)`), entering tree mode on window 1 (row index 1) and pressing `j` 12 times selects row 13 and leaves `tree.sidebar.top == 9`.
- In `ui/mod.rs`: `tree_mode_shows_the_badge_the_title_and_the_selection`. In tree mode, the statusbar contains ` TREE `, the sidebar title contains `agents · tree`, and the selected row's cells have `Modifier::REVERSED` in the `TestBackend` buffer. In filter input, the statusbar contains ` FILTER ` and `/sty`.
- In `ui/mod.rs`: extend `modals_render_on_top` to assert the help overlay contains `tree mode` and `sidebar width`.

**Change.** Implement decisions 29, 30, 31 and 33 in `tree_input.rs`. When a filter or collapse change hides the focused window, focus does not change. In `statusbar.rs`, show ` TREE ` or ` FILTER ` badges in the accent colour when the prefix is not pending, then in tree mode the hints `j/k move  ⏎ focus  space fold  / filter  esc back`, and in filter input `/{filter}`. Update `HELP` and `HINTS` as in the Interfaces section.

**Acceptance.** The tests pass. In tree mode the PTY receives nothing but prefix-command effects.

### M4.9 Overview

**Files.** Create `crates/tui/src/ui/overview.rs`. Modify `crates/tui/src/ui/mod.rs`, `crates/tui/src/ui/tree_view.rs`, `crates/tui/src/app.rs`, `crates/tui/src/tree_input.rs`, `crates/tui/src/app_tests.rs`.

**Tests first.**

- In `ui/mod.rs`: `overview_replaces_the_terminal_with_the_wide_tree`. Feed window 1 a `Snapshot` with `TERMINAL-TEXT`. After `C-b T`, render at 160 by 30. The output contains `claude-opus-5`, `claude-sonnet-4-5`, `/r/shop`, `general-purpose: grep handlers`, `running` and `done`, and does not contain `TERMINAL-TEXT`. After `Esc`, it contains `TERMINAL-TEXT` again.
- In `app_tests.rs`: `overview_opens_tree_mode_and_closes_with_it`. `C-b T` sets `overview` and `tree_input == Some(Navigate)`. `C-b T` again clears both. `Esc` from the overview clears both.
- In `app_tests.rs`: `overview_does_not_resize_the_pty`. Toggling the overview emits no `Resize` and does not change the size given to `set_terminal_size`.
- In `app_tests.rs`: `a_click_in_the_overview_acts_like_enter`. With the overview open, a click on the `billing` row inside `main_inner` emits `Subscribe` for window 2, closes the overview and leaves tree mode.
- In `ui/mod.rs`: `overview_geometry_matches_its_hit_test`, the same check as in M4.5, with `main_inner` as the list rect and `tree.overview.top`.

**Change.** Implement decisions 34 and 38. `ui::draw` calls `overview::render` instead of `terminal::render` when `app.overview` is set. The main block's title is ` tree overview `. `App::on_click` checks the overview first when it is open. Reveal the selection in both viewports.

**Acceptance.** The tests pass. `ui::terminal::render` is untouched.

### M4.10 `anthrex tree`

**Files.** Create `crates/cli/src/tree_cmd.rs`. Modify `crates/cli/src/main.rs` and `crates/cli/Cargo.toml`.

**Tests first**, as unit tests in `tree_cmd.rs`, on a fixture shaped like `example()` (the CLI cannot use the `#[cfg(test)]` helper from another crate, so build a smaller copy: shop with windows 1, 2 and 4 and their sub-agents, and blog with window 8).

- `text_lists_projects_windows_and_nested_subagents`: `tree_text(&windows, None)` has the project line `shop  /r/shop  attention  cl 2 · cx 1` first. It contains the exact window line `   2  billing  codex  -  attention  41s`, and window 4 has position 3. Sub-agent lines that start with six spaces and contain `├ Explore: map routes  running` and `│ └ general-purpose: grep handlers  running`.
- `json_has_the_documented_shape`: `serde_json::to_value(tree_json(&windows, None))` has `projects[0].name == "shop"`, `projects[0].counts.claude == 2`, `projects[0].windows[0].position == 1`, `projects[0].windows[0].subagents[0].id == "a1"`, `...subagents[0].children[0].id == "a2"` with `parent_id == "a1"`, `state == "running"`, and `projects[1].name == "blog"`.
- `project_filter_picks_the_longest_containing_root`: with projects `/r` and `/r/shop`, `--project /r/shop/src` selects only `/r/shop`, and `--project /r/other` selects only `/r`. `--project /elsewhere` gives `no windows` and `{"projects":[]}`.
- `empty_list_prints_no_windows`.
- `permission_is_printed_and_serialized`: with `needs_permission` set on `a1`, its text line contains `Explore: map routes  permission`, and its JSON object has `"needs_permission": true`.

**Change.** Add `Tree { #[arg(long)] project: Option<PathBuf>, #[arg(long)] json: bool }` to `Command` in `main.rs`, documented as "Print the project tree". The arm connects with `client::CliClient::connect`, canonicalizes `--project` with the existing `resolve_dir` when it is given, and prints `tree_text` or `serde_json::to_string_pretty(&tree_json(..))`. Implement decisions 20 (CLI part), 42, 43 and 44 with the pure functions `pub fn tree_text(windows: &[WindowInfo], project: Option<&Path>) -> String` and `pub fn tree_json(windows: &[WindowInfo], project: Option<&Path>) -> TreeJson`, both built on `tui::tree::build`, `tui::tree::subagent_forest` and `tui::tree::format_elapsed`.

**Acceptance.** The tests pass. `anthrex tree --help` shows both flags. `anthrex tree` with no daemon running fails with the same "cannot reach the daemon" error as `anthrex ls`.

### M4.11 End to end with fake agents, and the smoke stage

**Files.** Create `crates/cli/tests/tree.rs`. Modify `crates/cli/Cargo.toml` (dev-dependencies) and `scripts/pty-smoke.py`.

**Tests first**, in `crates/cli/tests/tree.rs`.

- Setup: declare `mod support;` and start the daemon with milestone 3's `support::TestDaemon::start(&script)`, which runs a real daemon under `/tmp` with `fake-agent` (found by `support::fake_agent_bin()`) as both runtimes and `FAKE_AGENT_SCRIPT` set to the script. Run CLI commands with `TestDaemon::anthrex(args)`. Dropping the `TestDaemon` stops the daemon.
- The script, one JSON step per line: `{"hook":"SessionStart","payload":{}}`; `{"hook":"PreToolUse","payload":{"tool_name":"Agent","tool_input":{"subagent_type":"Explore","prompt":"map routes\nin detail"}}}`; `{"hook":"SubagentStart","payload":{"agent_id":"a1","agent_type":"Explore"}}`; `{"hook":"PreToolUse","payload":{"agent_id":"a1","tool_name":"Agent","tool_input":{"subagent_type":"general-purpose","prompt":"grep handlers"}}}`; `{"hook":"SubagentStart","payload":{"agent_id":"a2","agent_type":"general-purpose"}}`; `{"hook":"PreToolUse","payload":{"agent_id":"a1","tool_name":"Read","tool_input":{}}}`; `{"read_line":true}`.
- `tree_json_groups_worktree_agents_and_nests_subagents`: build a temp git repo with one commit, a subdirectory `sub`, and a linked worktree at `<tmp>/wt`, with the helper from M4.1. Run `anthrex new --runtime claude --name main-agent --dir <repo>/sub`, `anthrex new --runtime claude --name wt-agent --dir <tmp>/wt`, and `anthrex new --runtime shell --name loose --dir <tmp>/plain`. Poll `anthrex tree --json` every 100 ms for up to 10 seconds until both Claude windows have `a2` among the `children` of `a1`. Then assert: exactly two projects; one has `root == repo.canonicalize()` and contains `main-agent` and `wt-agent`; the other has `root == plain.canonicalize()` and contains `loose`. In `main-agent`, `subagents` has one entry `a1` with `kind == "Explore"`, `label == "map routes"`, `tool == "Read"`, and `children` holding `a2` with `parent_id == "a1"` and `label == "grep handlers"`.
- `tree_text_and_project_filter`: in the same setup, `anthrex tree --project <tmp>/wt` prints the repo's project line and both Claude windows, and does not print `loose`.

**Change.** Add a smoke stage to `scripts/pty-smoke.py`, named `stage 8c: project tree`, after milestone 3's stage 8b and before stage 9. It creates a temp git repository with a linked worktree using `subprocess` and the same `-c` flags as M4.1. It runs `anthrex new --runtime shell --name tree-a --dir <repo>/sub` and `anthrex new --runtime shell --name tree-b --dir <wt>`. It parses `anthrex tree --json` and fails unless one project has `root == os.path.realpath(repo)` and holds both `tree-a` and `tree-b`. It then attaches with `PtyProc([BIN])`, sends `\x02t` and waits for ` TREE `, sends `/tree-b` and waits for ` FILTER `, sends `\r` twice (confirm the filter, then focus), and waits for `tree-b · shell` in the main title. It sends `\x02T`, waits for ` tree overview ` and the repo path, sends `\x1b`, then detaches with `\x02d` and checks exit status 0. It runs `anthrex rm tree-a` and `anthrex rm tree-b`, so later stages see only `shell-1` to `shell-4`, and removes the temp repository at the end.

**Acceptance.** Both tests pass under `cargo test --workspace`. The smoke script passes. `pgrep -fl "anthrex daemon"` shows no daemon left by the tests.

## Verification

Run every command from `AGENTS.md`:

```bash
cargo build --workspace --all-targets
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
python3 scripts/pty-smoke.py
```

Milestone-specific checks:

1. `rg -n 'SIDEBAR_WIDTH\b|CARD_HEIGHT|sidebar::hit_test' crates` finds nothing.
2. `rg -n '_ =>' crates/tui/src/tree.rs crates/tui/src/ui/tree_view.rs crates/tui/src/ui/overview.rs crates/cli/src/tree_cmd.rs` finds no arm that matches on `NodeKey`, `RowKind` or `ProjectChild` (decision 26).
3. `rg -n 'std::fs|std::process|tokio' crates/tui/src/tree.rs crates/tui/src/tree_input.rs crates/tui/src/app.rs crates/tui/src/ui` finds nothing.
4. `wc -l crates/tui/src/app.rs` is under 600.
5. The smoke output contains `stage 8c: project tree` followed by its `ok:` line.

## Manual check

Use a scratch environment for every step, and stop the daemon at the end:

```bash
export ANTHREX_SOCKET=/tmp/ax-m4.sock ANTHREX_DATA_DIR=/tmp/ax-m4-data
cargo build
BIN=$PWD/target/debug/anthrex
```

1. Pick a real repository `R` with a clean tree. Run `git -C R worktree add /tmp/ax-m4-wt -b ax-m4-test`.
2. Start `$BIN` from `R`. Create four Claude windows with `$BIN new --runtime claude --dir R` and three Codex windows with `$BIN new --runtime codex --dir R`, one of the Claude windows with `--dir /tmp/ax-m4-wt`. Create one shell with `--dir /tmp`.
3. Check that the sidebar is 34 columns, that `R`'s project row shows `cl 4 · cx 3`, that the worktree Claude window sits under `R`'s project, and that `/tmp` is a separate project below it unless it is more urgent.
4. In one Claude window, ask for work that uses sub-agents, for example "use two Explore sub-agents in parallel to list the modules and the tests of this repo". Watch sub-agent rows appear under that window with a spinner, a label and a tool on the right, then turn to `✓`. If one sub-agent spawns another, check the nesting guides.
5. Press `C-b t`. Move with `j`/`k`, fold the project with `Space`, check that `C-b 1..9` numbers skip the folded windows, unfold it, filter with `/` and part of a sub-agent label, and press `Enter` on the sub-agent row: its window takes focus.
6. Press `C-b T`. Check the full model names, labels and durations, click a Codex row, and check that it takes focus and the overview closes. Check that the agent's screen did not redraw at a new size when the overview opened or closed.
7. Press `C-b >` three times and `C-b <` five times. Check that the agent in the main area redraws at each new width and that the width stops at 60 and 24.
8. Make the terminal short enough that the tree overflows. Move the selection past the bottom in tree mode and check that the list scrolls. Scroll with the mouse wheel over the sidebar. Click a row after scrolling and check that the row under the pointer is the one that reacts.
9. Run `$BIN tree`, `$BIN tree --json | python3 -m json.tool`, and `$BIN tree --project /tmp/ax-m4-wt`.
10. Detach with `C-b d`, run `$BIN daemon stop`, `git -C R worktree remove /tmp/ax-m4-wt`, `git -C R branch -D ax-m4-test`, and check that `pgrep -fl "anthrex daemon"` shows nothing.

## Risks and gotchas

1. **Temp paths on macOS are symlinks.** `tempfile` returns `/var/folders/...`, git reports `/private/var/folders/...`. Tests that compare raw paths fail only on macOS. Always compare canonicalized paths.
2. **The developer's git config leaks into tests.** `init.defaultBranch`, `commit.gpgsign` or a signing hook can make the fixture commits fail or prompt. Pass the `-c` flags from M4.1 and `GIT_CONFIG_NOSYSTEM=1` in every test and in the smoke stage.
3. **A temp directory inside a repository.** If `TMPDIR` points inside a git checkout, `plain_directory_is_its_own_root` fails because the directory really is inside a repository. If you see that, the test is right and the environment is wrong; record it rather than weakening the test.
4. **Borrowing rows while mutating the app.** `App::rows()` borrows `self`. Take what you need from the rows, such as an id or a `NodeKey`, drop the rows, then call `self.focus(..)`. Where you must mutate `self.tree` while rows are alive, build the rows from `&self.windows` with `tree::build(&self.windows, &self.tree)` so the borrow covers only the `windows` field.
5. **Unicode widths.** `◆`, `▾`, `│` and the braille spinner are one column in `unicode-width` 0.2.2, which ratatui 0.30.2 also uses. A terminal set to render East Asian ambiguous characters as wide will misalign rows. That is a terminal setting; do not work around it.
6. **Selection jumping when projects reorder.** If a test sees the selection move to another row after a status change, something is storing an index instead of a `NodeKey`.
7. **Hit-test drift.** If a click selects the row above or below the one under the pointer, render and hit-test have stopped sharing `tree_view::geometry`. Check that neither applies its own offset, especially after scrolling.
8. **Git slower than expected.** On a large repository with a cold cache, `rev-parse` can take hundreds of milliseconds. That only delays `Created`; the connection keeps serving other requests (decision 7). If the TUI seems frozen while a window is created, check that the `CreateWindow` arm really spawns its task.
9. **`fake-agent` not built.** `cargo test -p anthrex` alone does not build it; `support::fake_agent_bin()` then panics with the command to run. CI and the documented commands use `--workspace`, which builds it.
10. **Milestone-3 names.** The "Starting point" names come from the milestone-3 brief. If the merged code differs, the Interfaces here follow the real code. Record every such mapping under "Implementation notes".
11. **The smoke script waits for the old sidebar title.** Stages 1, 6 and 7 waited for `anthrex`. M4.5 changes those waits. If the smoke script times out in stage 1, that is the cause.

## Follow-ups handled

From `docs/superpowers/plans/2026-09-17-anthrex-foundation-followups.md`, "Assignment to milestones", row M4:

- **Sidebar overflow: the tree scrolls to keep the selection visible.** Closed by decision 24 and tasks M4.4, M4.5 and M4.8. This also closes the Task 15 ledger minor "no scrolling or '+N more' indicator when cards overflow the sidebar (focused card can be off-screen)".
- **Hit-testing shares geometry with rendering.** Closed by decision 25 and tasks M4.5 and M4.9, which test that the two agree while scrolled, in both the sidebar and the overview.

## Implementation notes

The implementer fills in this section with every deviation, surprise and decision made during implementation.

### M4.5 sidebar rendering

- Corrected the layout test's arithmetic, preserving the existing one-row statusbar: a 120×40 area has a 39-row body, a 37-row sidebar inner area, a 35-row list, and footer at y=37. The brief's list height 34/footer y=36 would leave an extra unused body row and contradict decision 36 (inner minus spacer and footer). The controller explicitly approved these corrected expectations. All four narrow golden strings remain exactly as written and occupy 32 columns.
- Moved the existing app tests to `crates/tui/src/app_tests.rs` in M4.5, bringing forward the file-only split already planned for M4.6. The starting app file was 881 lines, not the brief's stale 595-line count; the resulting production file is 566 lines. Existing test behavior is unchanged; the wheel test now supplies a layout whose main inner rectangle equals its previous rectangle to match the expanded scrolling API.
- Exposed the existing M4.3 fixture through the requested test-only `tree::example_windows()` wrapper; its original implementation lives in `tree/tests.rs`.
- Updated all four old-sidebar-title smoke waits, including the additional stage-8b attach omitted from the brief's inventory. The old stage-8b card assertions also proved incompatible with decision 37: window tree rows omit the textual status and current tool. With controller approval, stage 8b now checks the visible `fake-claude` row/runtime tag, polls isolated-daemon JSON within the existing three-second working period for `working`/`Bash`, and retains the completion toast and JSON done/session checks, additionally checking that the tool cleared. No smoke stage was added.
- The task packet's “later-compatible wide_line” wording was clarified by the controller: decision 38 and the actual wide-line API/behavior remain M4.9 work. M4.5 adds shared geometry and narrow rendering, without an untested wide-format stub.
- `set_tree_viewports` reveals the anchor only when viewport heights change, so a repeated draw does not undo wheel scrolling. Focus/list changes still reveal it. Zero-height viewports do not reveal an anchor before their first usable size report.
- Review correction: production drawing now reports terminal size and tree viewport heights before rendering. Reporting them afterward could change the scroll top between a rendered frame and its next click after a terminal-height shrink. A regression runs the actual production draw path with a test terminal and a real temporary Unix socket: shrinking 120×14 to 120×10 previously displayed shell 12 while clicking that row subscribed to shell 16. The rendered frame and click now agree; `ui::draw` still takes `&App`, and the rendered layout is returned unchanged to event handling.
