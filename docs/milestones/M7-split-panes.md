# Milestone 7: Split panes

## Header

| Field | Value |
|-------|-------|
| Status | `blocked` |
| Depends on | 4 |
| Spec sections | Product spec (`docs/superpowers/specs/2026-09-18-anthrex-product-design.md`) section 7, the protocol row for version 4 in section 10.1, the pane keys in section 10.3, and `panes.max` in section 10.4. Core spec (`docs/superpowers/specs/2026-09-17-anthrex-design.md`) sections 3.3, 4 and 6.2 where they still govern PTY I/O, subscriptions and mouse input. |
| Branch | `m7-split-panes` |
| Protocol version | 4 (`proto::PROTO_VERSION` goes from 3 to 4). Rule (product spec 10.1): set it to one more than the value on `main` when you start; 4 assumes roadmap order. |

Read `AGENTS.md` first. Its rules apply to every task here and are not repeated.

## Goal

The user can show several agents at once. `C-b |` and `C-b -` split the focused pane and ask which window to show in the new pane. Each pane has its own title, border and live terminal. The user moves focus between panes with `C-b o` and `C-b` plus an arrow, moves dividers with `C-b` plus Shift+arrow, zooms one pane with `C-b z`, and closes a pane with `C-b w` without stopping its window. Switching windows with the existing keys shows the chosen window in the focused pane. One client connection now holds one subscription per visible window.

Three milestone-1 follow-ups close here: full-screen apps repaint when a client attaches (the SIGWINCH jiggle), the mouse wheel scrolls alternate-screen apps that did not enable mouse reporting, and wheel reports use the encoding the app asked for instead of always SGR.

## Scope

### Starting point

Milestones 3 and 4 are done when this milestone starts. This brief uses the names of their briefs:

- Milestone 3: `WindowInfo.session_id`, `model` and `subagents`; `WindowManager::focus(id)` increments the window's viewer count and the new `WindowManager::unfocus(id)` decrements it (M3 decision 26); `crates/fake-agent`.
- Milestone 4: `WindowInfo.project`; `proto::PROTO_VERSION == 3`; the tree model `crates/tui/src/tree.rs` (`build`, `agent_order`, `TreeState`), its renderer `crates/tui/src/ui/tree_view.rs` (`narrow_line`, `wide_line`, `geometry`), tree mode in `crates/tui/src/tree_input.rs` (`C-b t`), the overview (`C-b T`, `App.overview`, `crates/tui/src/ui/overview.rs`), sidebar width keys (`C-b <`, `C-b >`, `App.sidebar_width`). `ui::Layout` has `sidebar`, `sidebar_inner`, `sidebar_list`, `sidebar_footer`, `main`, `main_inner` and `statusbar`. `App::on_click(column, row, &ui::Layout)` handles left clicks on the sidebar and the overview. `App::on_scroll` also scrolls the sidebar list. `App::set_tree_viewports(sidebar_rows, overview_rows)` runs after every draw. `App::rows()` gives the visible tree rows. `App`'s tests live in `crates/tui/src/app_tests.rs`.

In roadmap order milestones 5 and 6 are also done. The roadmap allows this milestone before them, so tasks that depend on them say so explicitly ("if milestone 6 is done"). With milestone 6, `app.rs` is `crates/tui/src/app/mod.rs` and its tests are in `crates/tui/src/app/tests.rs`; read every `app.rs` and `app_tests.rs` below as those files. With milestone 5, the main title of a worktree window names its project root and branch (M5 decision 33), and `C-b c` opens the new-agent form.

If the merged code differs from the names above, use the real names and record the mapping in "Implementation notes".

What exists today and must be migrated:

- `crates/tui/src/app.rs`: `App` holds exactly one `parser: vt100::Parser`, one `focused: Option<u32>`, one `scroll_offset: usize`, one `term_size: (u16, u16)` and one `pending_resize: Option<Instant>`. `App::focus(id)` replaces the parser and emits a single `ClientMsg::Subscribe`. `App::set_terminal_size(cols, rows)` is called by `crates/tui/src/lib.rs::draw` with `ui::Layout.main_inner`. `App::on_scroll(up, column, row, main_inner)` always encodes wheel reports as SGR.
- `crates/daemon/src/server.rs::handle_client`: one `subscription: Option<JoinHandle<()>>` per connection. `Subscribe` aborts and awaits the old forwarder, then resizes, attaches, queues the snapshot and spawns `forward_output`. `Unsubscribe` is a unit variant.
- `crates/tui/src/keymap.rs`: `Keymap::handle` matches `Char` keys without looking at modifiers and ignores arrows after the prefix. `encode_key` already encodes Shift+arrow as `ESC [ 1 ; 2 <A-D>` for passthrough.

### In scope

1. Protocol version 4: many subscriptions per connection, `Unsubscribe { window_id }`, a limit of 16.
2. The SIGWINCH jiggle when a client subscribes at the window's current size and the window shows its alternate screen.
3. A pure layout module, `crates/tui/src/panes.rs`, with the pane tree and every geometric rule.
4. A per-window view module, `crates/tui/src/view.rs`, with the client parser, scrollback offset, resize debounce and wheel encoding.
5. Migrating `App` from one parser to one view per visible window.
6. The pane key bindings, the window picker, per-pane rendering, mouse click and wheel per pane.
7. A smoke-script stage that splits the screen and checks two windows are visible side by side.

### Out of scope

- Saving or restoring layouts across detach. The layout lives in the client and is lost on detach (spec 7.1).
- Showing one window in two panes. A window appears in at most one pane per client.
- Dragging dividers with the mouse. Forwarding mouse clicks to the program.
- Per-client PTY sizes. Two clients showing one window at different sizes still resolve by last resize wins.
- Creating a new window from the picker. `C-b c` (or milestone 5's dialog) creates windows; they then show in the focused pane.

## Design decisions

1. **Protocol version 4.** `PROTO_VERSION` becomes 4. `ClientMsg::Unsubscribe` changes from a unit variant to `Unsubscribe { window_id: u32 }`. No other message changes shape.
2. **Subscribe adds or refreshes.** `Subscribe { window_id, cols, rows }` for a window the connection is not subscribed to adds a subscription. For a window it is already subscribed to, it aborts and awaits that window's forwarder, then resizes, attaches and re-sends a `Snapshot`, exactly like milestone 1's replacement. Other windows' forwarders are never touched.
3. **Server bookkeeping.** `handle_client` keeps `subscriptions: HashMap<u32, JoinHandle<()>>` in place of `subscription`. Before every limit check it drops handles whose task `is_finished()`. A forwarder ends by itself when its window is removed, because the broadcast channel closes.
4. **Limit of 16.** A new constant `daemon::server::MAX_SUBSCRIPTIONS: usize = 16`. A `Subscribe` for a new window while 16 live subscriptions exist is answered with `Error { request: "subscribe", message: "too many subscriptions on one connection (limit 16)" }` and changes nothing. Refreshing an existing subscription is always allowed.
5. **Unsubscribe.** `Unsubscribe { window_id }` aborts, then awaits, that window's forwarder and replies `Ack { request: "unsubscribe" }`. It also replies `Ack` when the connection holds no subscription for that id, so a client may unsubscribe from a removed window without an error toast. On disconnect every remaining forwarder is aborted.
6. **Viewing and status.** Every subscribed window counts as viewed. A `Subscribe` that adds a subscription calls `WindowManager::focus(window_id)`, which increments that window's viewer count. A refresh of an existing subscription does not call it again, so each subscription counts once. `Unsubscribe` calls `WindowManager::unfocus(window_id)` when the connection held that subscription, and disconnect calls it for every remaining subscription. Milestone 3's rule "unfocus when the subscription moves to another window" no longer applies, because subscriptions no longer move.
7. **SIGWINCH jiggle.** Linux and macOS send SIGWINCH only when a PTY's size actually changes (Linux `tty_do_resize`, XNU `ttioctl` for `TIOCSWINSZ`). So when a `Subscribe` asks for the size the window already has and the daemon's parser shows the alternate screen, the daemon resizes the PTY master to `rows - 1` rows and immediately back to `rows` (if `rows < 2`, it uses `cols - 1` instead; if both are 1 it does nothing). The daemon's own parser is not resized by the jiggle. Primary-screen windows are never jiggled, so shells do not reprint their prompt on every attach. The jiggle happens before `attach`, so the snapshot is taken at the final size and the repaint arrives as ordinary `Output`.
8. **Pane tree.** `crates/tui/src/panes.rs` (new) holds `Pane::Leaf { id: PaneId, window_id: u32 }` and `Pane::Split { axis, ratio, first, second }`. The leaf carries a `PaneId` in addition to spec 7.1's `window_id`, so focus survives the focused pane switching to another window. `PaneId` values come from a counter in the tree starting at 1 and are never reused within one tree.
9. **Axis naming.** `Axis::Row` lays its children out in a row, side by side: `first` is left, `second` is right. `C-b |` creates it. `Axis::Column` stacks them: `first` is top, `second` is bottom. `C-b -` creates it.
10. **Ratio math.** `ratio: u8` is the percent of the split's own area given to `first`, always within `10..=90`. For a `Row` split of rect `r`, `first` gets width `r.width as u32 * ratio as u32 / 100` (integer division, computed in `u32`), and `second` gets the rest. `Column` does the same with heights. A new split starts at 50. Panes touch with no gap; each draws its own border.
11. **Sizes.** `MAX_PANES: usize = 16` leaves is the hard limit in `panes.rs`, equal to the subscription limit. The App's limit is `DEFAULT_MAX_PANES: usize = 6`, or `panes.max` (decision 34). A pane's inner area is at least `MIN_PANE_COLS = 20` columns by `MIN_PANE_ROWS = 4` rows; with the 1-cell border the outer rect is at least 22 by 6. `PaneTree::fits(area)` is true when every leaf's outer rect in `full_layout(area)` meets that minimum.
12. **Too small.** When `fits(main_area)` is false, `layout` returns only the focused pane with the whole area, as if zoomed, without changing the `zoomed` flag. When the area grows back, the full layout returns. The title shows `[zoom]` in both cases.
13. **Split.** A split replaces the focused leaf with `Split { axis, ratio: 50, first: <old leaf>, second: <new leaf> }` and focuses the new leaf. Splitting while zoomed clears `zoomed` first. The App refuses a split, with a toast and no picker, in this order: `"at most {limit} panes"` (`"at most 6 panes"` by default) when the pane limit of decision 34 is reached; `"no room for another pane"` when the tree after the split would not `fit` the current main area; `"every window is already shown"` when no window is left to pick.
14. **Close.** Closing a leaf replaces its parent split with the sibling subtree, so the sibling takes the whole space. If the closed leaf was focused, focus moves to the first leaf, in depth-first order, of that sibling subtree. Closing also clears `zoomed`. Closing the only leaf is refused with `PaneError::LastPane`; `C-b w` then toasts `"this is the only pane"`. The closed pane's window keeps running.
15. **Focus next.** `C-b o` focuses the next leaf in depth-first order (`first` before `second`), wrapping from the last to the first.
16. **Focus by direction.** Computed on `full_layout(main_area)`, never on the zoomed layout. Let `F` be the focused pane's outer rect. For `Left`, candidates are panes `P` with `P.x + P.width <= F.x` whose rows overlap `F`'s rows by at least one row. Pick the smallest distance `F.x - (P.x + P.width)`, then the largest overlap, then the smallest `P.y`, then the smallest `P.x`. `Right` uses `P.x >= F.x + F.width` and distance `P.x - (F.x + F.width)`. `Up` and `Down` are the same with rows and columns swapped, breaking ties by smallest `P.x`, then smallest `P.y`. With no candidate nothing happens; focus does not wrap.
17. **Zoom and focus.** `zoomed` belongs to the tree, not to a pane. While zoomed, `C-b o` and `C-b` plus arrow still move focus through the full layout, and the newly focused pane is the one shown zoomed. `C-b z` toggles `zoomed`.
18. **Divider resize.** `C-b` plus Shift+arrow picks a split on the arrow's axis (`Row` for Left and Right, `Column` for Up and Down) among the focused leaf's ancestors. It prefers the nearest ancestor whose divider is on the arrow's side of the focused pane: for Left and Up, an ancestor where the focused leaf is inside `second`; for Right and Down, inside `first`. If none exists, it takes the nearest ancestor on that axis at all. Left and Up subtract `RESIZE_STEP = 5` from its ratio; Right and Down add 5; the result is clamped to `10..=90`. If the new ratio would make the tree not `fit` the main area, the ratio is left unchanged. With no ancestor on that axis nothing happens.
19. **One layout function.** `PaneTree::layout(area) -> Vec<(PaneId, Rect)>` returns the visible panes' outer rects in depth-first order. Rendering, mouse hit-testing and PTY sizing all call it with the same main area, so what is drawn, what is clicked and what the PTY is sized to cannot disagree. The pane's inner rect is always `panes::inner(outer)`, a 1-cell inset.
20. **Views.** `App.views: BTreeMap<u32, WindowView>` holds one view per window that is in the pane tree, including windows in panes hidden by zoom. A view has its own `vt100::Parser` with the scrollback length given to `WindowView::new` (milestone 6's `app.settings.scrollback_lines`, or `SCROLLBACK_LINES` (5000) without milestone 6), its own `scroll_offset`, its own size and its own resize debounce. Windows not in the tree have no view and no subscription.
21. **Reconciling.** One private function, `App::reconcile`, turns the tree into effects after every change to the tree, the window list or the main area. It first emits `Unsubscribe { window_id }` for every view whose window left the tree, and drops those views. Then, for each pane in `layout(main_area)` in order, it creates a view at the pane's inner size and emits `Subscribe` if the window has none, or marks a pending resize if the view's size differs from the pane's inner size. Hidden panes keep their last size. Unsubscribes are always emitted before subscribes.
22. **Resize debounce per window.** A size change sets the parser's size at once and marks the view's `pending_resize`. `on_tick` emits `Resize { window_id, cols, rows }` for each view whose pending resize is at least `RESIZE_DEBOUNCE` (30 ms) old, in `views` key order. A new view never waits: its `Subscribe` carries the size.
23. **Switching windows.** `App::focus(id)` keeps its name and is still the single entry point for `C-b j`, `C-b k`, `C-b 1..9`, `Created`, `attach <name>`, sidebar clicks and every milestone-4 tree action. If `id` is already in a pane, focus moves to that pane and nothing is sent. Otherwise the focused pane switches to `id`: `reconcile` then emits `Unsubscribe` for the old window and `Subscribe` for the new one at the pane's inner size. With no pane tree yet, `focus` creates one with a single leaf.
24. **Window picker.** `C-b |` and `C-b -` open `Modal::PanePicker { axis, candidates, selected: 0 }`. `candidates` are the ids of windows not in the tree, in the order `C-b j` walks windows: `tree::agent_order(&app.rows())`. Keys: `j` or Down moves down, `k` or Up moves up, both wrapping; `1` to `9` pick that listed entry directly; `Enter` picks the selected entry; `Esc` cancels with no split. The split is performed only when a window is picked.
25. **Removed and exited windows.** A window that exits stays in its pane, showing its last screen with the `✕` glyph, until the user closes the pane or switches it. When a window disappears from the list: if it is in one of two or more panes, that pane closes as in decision 14; if it is in the only pane, that pane switches to milestone 4's neighbour choice (the window now at the removed window's old position in `tree::agent_order`, clamped); if no window remains, the tree becomes `None`.
26. **Input.** Keys, pastes and the snap-back to the live screen go to the focused pane's window, using its view's `application_cursor()` and `bracketed_paste()`.
27. **Mouse click.** A left click inside a pane's outer rect focuses that pane. It is not forwarded to the program. Sidebar clicks keep their milestone-4 behaviour.
28. **Mouse wheel.** A wheel event inside a pane's inner rect first focuses that pane, then acts on its view: (a) if `mouse_protocol_mode()` is not `None`, it sends a wheel report with button 64 (up) or 65 (down) at 1-based coordinates relative to the pane's inner rect, encoded by `mouse_protocol_encoding()`: `Sgr` as `ESC [ < b ; x ; y M`; `Default` as `ESC [ M` then the three bytes `32 + b`, `32 + x`, `32 + y`, and nothing at all when `x` or `y` exceeds 223; `Utf8` as `ESC [ M` then the characters `32 + b`, `32 + x`, `32 + y` each UTF-8 encoded, and nothing when `x` or `y` exceeds 2015. (b) Otherwise, if `alternate_screen()` is true, it sends `WHEEL_LINES = 3` Up or Down arrow keys encoded with `keymap::encode_key` and the view's `application_cursor()`. (c) Otherwise it scrolls the view's local scrollback by 3 lines, as milestone 1 does. Wheel events over borders, the sidebar or the status bar do nothing.
29. **Toasts.** The Attention and Done toasts are skipped for every window that is in a visible pane, not only the focused one, because the user can see it.
30. **Pane title.** Each pane is a rounded block titled `" {glyph} {name} · {runtime}{ model} · {cwd}{ (branch)}{ [zoom]} "`. `{glyph}` is `theme::status_glyph` coloured by `theme::status_color`. `{ model}` is a space and `WindowInfo.model` when it is `Some`. `{cwd}` goes through `ui::terminal::shorten_home`. With milestone 5, a worktree window shows milestone 5's `{shortened project root} ({branch}, worktree)` in place of `{cwd}{ (branch)}`. ratatui truncates the title to the border width.
31. **Pane border.** The focused pane's border uses `theme::border_focused()` (accent) when no modal is open; every other pane, and the focused pane while a modal is open, uses `theme::border()`. The hardware cursor is placed only in the focused pane, only when its view's `scroll_offset` is 0 and no modal is open.
32. **Keys.** After the prefix: `|` (any modifiers) `SplitRow`; `-` `SplitColumn`; `o` `NextPane`; `z` `ZoomPane`; `w` `ClosePane`; Left, Right, Up, Down with no modifiers `FocusPane(dir)`; the same arrows with exactly `SHIFT` `ResizePane(dir)`; arrows with any other modifiers `Nothing`. None of these collide with milestone 1's bindings (`j k n p 1-9 c x X s d Q ?`, `Esc`, the prefix itself), milestone 4's (`t T < >`), milestone 6's (`,`, `R`, `r`) or milestone 9's (`O`); product spec 10.3 lists them all. `o` and `O` are different keys.
33. **Shift+arrow outside the prefix.** Shift+arrow without the prefix still goes to the program as `ESC [ 1 ; 2 <A-D>` through `encode_key`. Only the key right after the prefix is a resize command.
34. **`panes.max`.** If milestone 6 is done, the App enforces `app.settings.panes_max`, which milestone 6 parses from `panes.max` (default 6, range `1..=16`; an out-of-range value keeps the default and is reported as a config problem by milestone 6). If milestone 6 is not done, the App enforces `DEFAULT_MAX_PANES` and a line is added to the follow-ups file under milestone 7: "wire `panes.max` once configuration exists".
35. **Sidebar marks.** In milestone 4's `tree_view::narrow_line`, the focused window's row keeps its accent bar. Other windows that are in a visible pane get the same `▎` bar in `theme::DIM`.
36. **Milestone 4's overview and click and wheel handlers.** While `App.overview` is set, `ui::draw` draws the overview over the whole main area instead of the panes; the pane layout and the PTY sizes do not change. `App::on_click` keeps milestone 4's signature and order (overview, then sidebar) and then tries the panes (decision 27). `App::on_scroll` tries milestone 4's sidebar list first, then the panes (decision 28). `set_tree_viewports` receives `layout.main.height.saturating_sub(2)` for the overview, since `ui::Layout` loses `main_inner`.

## Interfaces

### Protocol (`crates/proto`)

```rust
// crates/proto/src/lib.rs
pub const PROTO_VERSION: u32 = 4;

// crates/proto/src/messages.rs, ClientMsg (only the changed variant shown)
Unsubscribe { window_id: u32 },
```

`Subscribe { window_id, cols, rows }` keeps its shape; its meaning is decisions 2 to 4. Replies:

| Request | Reply |
|---------|-------|
| `Subscribe` (new window, under the limit) | `Snapshot`, then live `Output` for that window |
| `Subscribe` (window already subscribed) | a fresh `Snapshot`, then live `Output`; no duplicate chunks |
| `Subscribe` (new window, 16 already held) | `Error { request: "subscribe", message: "too many subscriptions on one connection (limit 16)" }` |
| `Subscribe` (unknown window) | `Error { request: "subscribe", .. }`, as today |
| `Unsubscribe { window_id }` | `Ack { request: "unsubscribe" }`, whether or not it was subscribed |

### Daemon (`crates/daemon`)

```rust
// crates/daemon/src/server.rs
pub const MAX_SUBSCRIPTIONS: usize = 16; // new

// crates/daemon/src/window.rs, new methods on Window
/// True when the screen mirror shows the alternate screen.
pub fn alternate_screen(&self) -> bool;
/// Decision 7: resizes the PTY master one row smaller and straight back, without touching
/// the parser, when the alternate screen is active. Returns whether it jiggled.
pub fn nudge_redraw(&self) -> anyhow::Result<bool>;

// crates/daemon/src/manager.rs, new method on WindowManager
/// Resizes like `resize`; when the size was already (cols, rows), calls `nudge_redraw`.
pub fn resize_for_attach(&self, id: u32, cols: u16, rows: u16) -> anyhow::Result<()>;
```

### Client layout (`crates/tui/src/panes.rs`, new)

```rust
use ratatui::layout::Rect;

pub const MAX_PANES: usize = 16;        // hard limit of the tree
pub const DEFAULT_MAX_PANES: usize = 6; // the App's limit without milestone 6's panes.max
pub const MIN_PANE_COLS: u16 = 20; // inner
pub const MIN_PANE_ROWS: u16 = 4;  // inner
pub const RESIZE_STEP: u8 = 5;
pub const RATIO_MIN: u8 = 10;
pub const RATIO_MAX: u8 = 90;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PaneId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis { Row, Column }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction { Left, Right, Up, Down }

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pane {
    Leaf { id: PaneId, window_id: u32 },
    Split { axis: Axis, ratio: u8, first: Box<Pane>, second: Box<Pane> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaneError { TooManyPanes, LastPane, NoSuchPane }

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneTree { /* root: Pane, focused: PaneId, zoomed: bool, next_id: u32 */ }

impl PaneTree {
    pub fn new(window_id: u32) -> Self;                       // one leaf, focused, not zoomed
    pub fn root(&self) -> &Pane;
    pub fn focused(&self) -> PaneId;
    pub fn focused_window(&self) -> u32;
    pub fn zoomed(&self) -> bool;
    pub fn leaves(&self) -> Vec<(PaneId, u32)>;               // depth-first, first before second
    pub fn window_of(&self, pane: PaneId) -> Option<u32>;
    pub fn pane_of(&self, window_id: u32) -> Option<PaneId>;
    pub fn set_focus(&mut self, pane: PaneId) -> bool;        // false if no such pane
    pub fn set_window(&mut self, pane: PaneId, window_id: u32) -> Result<(), PaneError>;
    pub fn split(&mut self, axis: Axis, window_id: u32) -> Result<PaneId, PaneError>; // decision 13
    pub fn close(&mut self, pane: PaneId) -> Result<u32, PaneError>;                  // returns the closed window id
    pub fn focus_next(&mut self);                                                    // decision 15
    pub fn focus_direction(&mut self, dir: Direction, area: Rect) -> bool;           // decision 16
    pub fn resize(&mut self, dir: Direction, area: Rect) -> bool;                    // decision 18
    pub fn toggle_zoom(&mut self);
    pub fn full_layout(&self, area: Rect) -> Vec<(PaneId, Rect)>;                    // every leaf, ignores zoom
    pub fn fits(&self, area: Rect) -> bool;                                          // decision 11
    pub fn layout(&self, area: Rect) -> Vec<(PaneId, Rect)>;                         // decisions 12 and 19
    pub fn hit_test(&self, area: Rect, column: u16, row: u16) -> Option<PaneId>;     // outer rects from `layout`
}

/// The rect inside a pane's 1-cell border.
pub fn inner(outer: Rect) -> Rect;
```

`split` checks only `MAX_PANES` (16). The App checks its own limit, `DEFAULT_MAX_PANES` or `panes.max` (decision 34), before calling. The "does it fit" check lives in the App, which knows the area.

### Client views (`crates/tui/src/view.rs`, new)

```rust
pub const WHEEL_LINES: usize = 3;

pub struct WindowView {
    pub parser: vt100::Parser,
    pub scroll_offset: usize,
    /// (cols, rows) the pane last asked for.
    pub size: (u16, u16),
    pending_resize: Option<std::time::Instant>,
    scrollback: usize,
    /// Milestone 6's dropped-Subscribe repair, per window: false after a refused Subscribe.
    pub subscribed: bool,
}

impl WindowView {
    pub fn new(cols: u16, rows: u16, scrollback: usize) -> Self;        // parser with that scrollback (decision 20)
    pub fn apply_snapshot(&mut self, cols: u16, rows: u16, bytes: &[u8]); // fresh parser at the snapshot size, offset 0
    pub fn set_size(&mut self, cols: u16, rows: u16, now: std::time::Instant) -> bool; // false if unchanged
    pub fn take_due_resize(&mut self, now: std::time::Instant) -> Option<(u16, u16)>;
    pub fn scroll_local(&mut self, up: bool);                            // WHEEL_LINES lines
    pub fn scroll_to_live(&mut self);
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Wheel { Send(Vec<u8>), ScrollLocal, Ignore }

/// Decision 28 for one wheel notch at 1-based pane coordinates (x, y).
pub fn wheel(screen: &vt100::Screen, up: bool, x: u16, y: u16) -> Wheel;
```

### Client state (`crates/tui/src/app.rs`)

Removed from `App`: `parser`, `focused` (the field), `scroll_offset`, `term_size`, `pending_resize`, and `set_terminal_size`.

```rust
// New or changed on App
pub panes: Option<PaneTree>,
pub views: BTreeMap<u32, WindowView>,
main_area: Rect,                                   // (0, 0, 0, 0) until the first draw

pub fn focused(&self) -> Option<u32>;              // window in the focused pane
pub fn focused_view(&self) -> Option<&WindowView>;
pub fn visible_windows(&self) -> Vec<u32>;         // windows of `layout(main_area)`, in order
pub fn pane_layout(&self, area: Rect) -> Vec<(PaneId, Rect)>; // `layout` of the tree, empty when None
pub fn set_main_area(&mut self, area: Rect) -> Vec<Effect>;   // replaces set_terminal_size
pub fn focus(&mut self, id: u32) -> Vec<Effect>;   // decision 23
pub fn on_click(&mut self, column: u16, row: u16, layout: &ui::Layout) -> Vec<Effect>;          // milestone 4's, extended: decisions 27 and 36
pub fn on_scroll(&mut self, up: bool, column: u16, row: u16, layout: &ui::Layout) -> Vec<Effect>; // decisions 28 and 36
pub fn on_tick_at(&mut self, now: Instant) -> Vec<Effect>; // `on_tick` calls it with Instant::now()

// Modal gains one variant
PanePicker { axis: Axis, candidates: Vec<u32>, selected: usize },
```

### Keys (`crates/tui/src/keymap.rs`)

```rust
// New Command variants
SplitRow,                  // C-b |
SplitColumn,               // C-b -
NextPane,                  // C-b o
FocusPane(Direction),      // C-b <arrow>
ResizePane(Direction),     // C-b Shift+<arrow>
ZoomPane,                  // C-b z
ClosePane,                 // C-b w
```

`Direction` is imported from `crate::panes`.

### Rendering (`crates/tui/src/ui`)

- `ui::Layout` loses `main_inner`. `lib.rs::draw` calls `app.set_main_area(layout.main)`.
- `ui::terminal::render(frame, app, area)` draws every pane of `app.pane_layout(area)` through a new `ui::terminal::render_pane(frame, app, outer: Rect, pane: PaneId, window_id: u32)`, or the milestone-1 "No agents" hint when `app.panes` is `None`.
- `ui::modal::render` draws `Modal::PanePicker`:

```
╭ show in new pane ──────────────────╮
│ ▎1 ◆ billing · codex               │
│  2 ○ search · codex                │
│  3 ✓ docs · claude                 │
│                                    │
│ Enter show · j/k move · Esc cancel │
╰────────────────────────────────────╯
```

The selected row has the accent `▎` bar and a bold name; the others start with a space.

- `ui::modal::HELP` gains these rows, after the window rows:

| Key column | Text |
|------------|------|
| `C-b \| / -` | `split side by side / stacked` |
| `C-b o / arrows` | `next pane / pane in that direction` |
| `C-b S-arrows` | `move the divider` |
| `C-b z` | `zoom pane` |
| `C-b w` | `close pane (window keeps running)` |

Two panes side by side, focused pane on the right:

```
╭ agents ──────────╮╭ ○ api · claude · ~/shop ─╮╭ ⠹ tests · codex · ~/shop ─╮
│ ▾ shop           ││$ cargo build             ││ running 42 tests          │
│ ▎○ 1 api         ││   Compiling shop v0.1    ││ test a ... ok             │
│ ▎⠹ 2 tests       ││                          ││                           │
│   ◆ 3 billing    ││                          ││                           │
╰──────────────────╯╰──────────────────────────╯╰───────────────────────────╯
```

The right pane's border is in the accent colour. Both visible windows carry a bar in the sidebar; the focused one's bar is the accent colour.

## Tasks

### M7.1 Protocol version 4 and many subscriptions per connection

**Files**
- Modify `crates/proto/src/lib.rs`, `crates/proto/src/messages.rs`, `crates/proto/src/codec.rs` (its duplex test).
- Modify `crates/daemon/src/server.rs`.
- Modify `crates/daemon/tests/server.rs`.
- Modify `crates/tui/src/app.rs` (one line in `focus`, see Change).

**Tests first**
- `crates/proto/src/messages.rs`: extend `every_daemon_message_round_trips` with `ClientMsg::Subscribe { window_id: 3, cols: 100, rows: 30 }` and `ClientMsg::Unsubscribe { window_id: 3 }`. Add `unsubscribe_carries_its_window_id`: serialize `Unsubscribe { window_id: 7 }` with `rmp_serde::to_vec_named`, decode, assert equal, and assert the decoded value is not equal to `Unsubscribe { window_id: 8 }`.
- `crates/proto/src/codec.rs`: `frames_round_trip_over_a_duplex_stream` writes and expects `ClientMsg::Unsubscribe { window_id: 1 }`.
- `crates/daemon/tests/server.rs` (real daemon, real shells, deadline loops through the existing `recv_until`):
  - `two_subscriptions_on_one_connection_both_stream`: create windows `a` and `b`, subscribe to both at 80x24, receive a `Snapshot` for each. Send `echo sub-a-$((1+1))` to `a` and `echo sub-b-$((2+2))` to `b`. Collect `Output` per `window_id`. Assert `sub-a-2` arrives with `window_id == a` and `sub-b-4` with `window_id == b`, and never the other way round.
  - `resubscribe_sends_a_fresh_snapshot_without_duplicating_output`: subscribe to `a`, send `echo dup-$((2+3))`, wait for `dup-5` in output. Subscribe to `a` again at the same size and wait for its `Snapshot`; assert the snapshot contains `dup-5`. Send `echo once-$((3+4))`. Collect every `Output` for `a` after that snapshot until 500 ms pass with no message. Assert `once-7` occurs exactly once in the concatenated text.
  - `unsubscribe_stops_one_window_only`: subscribe to `a` and `b`. Send `Unsubscribe { window_id: a }` and wait for `Ack { request: "unsubscribe" }`. Send `echo gone-$((1+8))` to `a`, then `echo kept-$((1+9))` to `b`. Wait for `kept-10` from `b`. Keep reading until 500 ms pass with no message. Assert no `Output` with `window_id == a` arrived after the `Ack`.
  - `unsubscribe_of_an_unknown_window_is_acked`: `Unsubscribe { window_id: 99 }` gets `Ack { request: "unsubscribe" }`.
  - `a_seventeenth_subscription_is_refused`: create 17 windows, subscribe to the first 16 and wait for 16 snapshots. Subscribe to the 17th: assert `Error { request: "subscribe", message }` with `message` containing `limit 16`. Re-subscribe to the first: assert a `Snapshot` arrives. Unsubscribe the second, then subscribe to the 17th: assert its `Snapshot` arrives. Remove all 17 windows at the end.
  - `a_removed_window_frees_its_subscription_slot`: create 17 windows, subscribe to 16, `Remove` one of them and wait for its `Ack`. Its forwarder ends only once the PTY reader thread sees EOF and drops its broadcast sender, so retry `Subscribe` for the 17th every 100 ms, skipping the limit `Error` replies, until a `Snapshot` for it arrives; fail after a 5-second deadline. This proves finished forwarders are pruned before the limit check.

**Change**
- `PROTO_VERSION = 4`; `Unsubscribe { window_id: u32 }`.
- In `handle_client`, replace `subscription` with `subscriptions: HashMap<u32, JoinHandle<()>>` and add `pub const MAX_SUBSCRIPTIONS: usize = 16`. Implement decisions 2 to 5 in the `Subscribe` and `Unsubscribe` arms. In the `Subscribe` arm: prune finished handles; check the limit only when `window_id` is not a key; if it is a key, `remove` it, `abort()` it and `await` it before anything else; then keep milestone 1's sequence (resize, attach, `focus`, queue the snapshot, spawn `forward_output`) and insert the new handle. Keep the comment that explains why the await matters, now "per window". At the end of `handle_client`, abort every handle in the map. Apply decision 6 if milestone 3 added a release call.
- In `App::focus` (still single-parser at this point), emit `Effect::Send(ClientMsg::Unsubscribe { window_id: old })` before the `Subscribe` when a different window was focused, so the client does not pile up subscriptions. Update the milestone-1 tests in `app.rs` that assert the exact effects of a switch (`keys_go_to_the_focused_window_and_prefix_switches`, `focusing_the_already_focused_window_does_nothing`, `removed_focused_window_moves_focus_to_a_neighbour`, and any milestone-4 test that asserts a switch) to expect `[Unsubscribe { old }, Subscribe { new, .. }]`.

**Acceptance**
- All new tests pass; the old `create_subscribe_input_and_kill_flow` and `errors_are_reported_per_request` still pass.
- `grep -rn "Unsubscribe" crates` shows no unit-variant use.
- `anthrex ls` and the TUI both connect to a daemon built from this branch.

### M7.2 SIGWINCH jiggle on attach

**Files**
- Modify `crates/daemon/src/window.rs`, `crates/daemon/src/manager.rs`, `crates/daemon/src/server.rs`.
- Modify `crates/daemon/tests/window.rs`, `crates/daemon/tests/server.rs`.

**Tests first**
- `crates/daemon/tests/window.rs`:
  - `nudge_redraw_signals_an_alternate_screen_program`: spawn `sh -c 'trap "echo winch" WINCH; printf "\033[?1049h"; while :; do sleep 0.1; done'` at 80x24. Wait until `w.alternate_screen()` is true. Call `w.nudge_redraw()`; assert it returns `Ok(true)`. Wait until `w.screen_text()` contains `winch`. Assert `w.size() == (80, 24)` afterwards.
  - `nudge_redraw_leaves_the_primary_screen_alone`: spawn `sh -c 'trap "echo winch" WINCH; echo ready; while :; do sleep 0.1; done'`. Wait for `ready`. Assert `nudge_redraw()` returns `Ok(false)`. Poll `screen_text()` for 1 second with a deadline loop; assert it never contains `winch`.
- `crates/daemon/tests/server.rs`:
  - `resubscribing_at_the_same_size_repaints_an_alternate_screen_app`: create a shell window at 80x24 and subscribe at 80x24. Send the input `trap 'echo winch' WINCH; printf '\033[?1049h'; while :; do sleep 0.1; done` plus `\n`. Wait for an `Output` containing `?1049h`. Subscribe again at 80x24. Assert the new `Snapshot` starts with `\x1b[?1049h`. Assert an `Output` containing `winch` arrives after it, within the 8-second `recv_until` limit.
  - `resubscribing_a_shell_at_the_same_size_does_not_signal_it`: same, without the `printf`, and with `echo armed` after the trap. Wait for `armed`, re-subscribe at 80x24, then read for 1 second; assert no `Output` contains `winch`.

**Change**
- `Window::alternate_screen` reads the mirror under `crate::lock(&self.parser)`.
- `Window::nudge_redraw` implements decision 7 with two `self.master.resize` calls. It reads the size and the alternate-screen flag under the parser lock, releases it, then resizes. It never calls `screen_mut().set_size`.
- `WindowManager::resize_for_attach` runs inside `with_entry`: remember `window.size()`, call `window.resize(cols.max(1), rows.max(1))`, and if the old size already equalled the new one, call `nudge_redraw`. Both calls are `ioctl`s and do not block, so holding the manager lock is allowed (the same as milestone 1's `resize`).
- The server's `Subscribe` arm calls `resize_for_attach` in place of `resize`.

**Acceptance**
- The four tests pass on macOS and Linux.
- A shell attached twice at an unchanged size does not print a second prompt.

### M7.3 The pane tree: split, close, layout, zoom, fit

**Files**
- Create `crates/tui/src/panes.rs`. Modify `crates/tui/src/lib.rs` (`pub mod panes;`).

**Tests first** (unit tests in `panes.rs`)
- `new_tree_has_one_focused_leaf`: `PaneTree::new(7)`: `leaves() == [(PaneId(1), 7)]`, `focused_window() == 7`, `!zoomed()`.
- `split_row_halves_the_width_and_focuses_the_new_leaf`: `new(1)`, `split(Axis::Row, 2)`. In `Rect::new(0, 0, 102, 26)`, `full_layout` is `[(PaneId(1), Rect(0,0,51,26)), (PaneId(2), Rect(51,0,51,26))]`; `focused_window() == 2`.
- `split_column_stacks_and_ratio_math_uses_integer_percent`: in `Rect::new(0, 0, 80, 101)`, `new(1)`, `split(Axis::Column, 2)`, then build the ratio-30 case by writing the tree literal directly in the test module (the fields are private but visible to the module's tests). Assert the top height is `101 * 30 / 100 = 30`, the bottom is 71, and the top rect is above the bottom one.
- `nested_layout_covers_the_area_without_overlap`: `new(1)`, split Row with 2, split Column with 3 (splits pane 2). In `Rect::new(10, 5, 100, 40)`: assert three rects, their areas sum to 4000, no two intersect, every rect is inside the area.
- `split_beyond_max_panes_is_refused`: fifteen splits succeed; the sixteenth `split` returns `Err(PaneError::TooManyPanes)` and the tree is unchanged.
- `close_gives_the_space_to_the_sibling_subtree`: tree `[A | [B / C]]` (ids 1, 2, 3; focus C). `close(PaneId(1))` returns `Ok(window of A)`; the root is now the Column split of B and C and fills the area; focus stays on C. Then `close(PaneId(3))`: root is leaf B, focus is B.
- `closing_the_focused_leaf_focuses_the_first_leaf_of_the_sibling`: tree `[[A / B] | C]` written as a literal (splits alone cannot build it), focus C, `close(C)`: focus is A. The same literal style serves every test below whose shape splits cannot produce.
- `closing_the_last_leaf_is_refused`: `new(1).close(PaneId(1)) == Err(PaneError::LastPane)`.
- `zoom_layout_is_the_focused_pane_over_the_whole_area`: two panes, `toggle_zoom()`, `layout(area) == [(focused, area)]`, `full_layout` still has two; `toggle_zoom()` again restores two.
- `split_and_close_clear_zoom`.
- `too_small_area_shows_only_the_focused_pane`: two Row panes in `Rect::new(0, 0, 40, 10)`: `fits` is false (outer width 20 < 22), `layout == [(focused, area)]`, `zoomed()` is still false. In `Rect::new(0, 0, 44, 10)`: `fits` is true and `layout` has two entries.
- `hit_test_uses_the_visible_layout`: two Row panes in `Rect::new(0, 0, 102, 26)`: column 10 hits pane 1, column 60 hits pane 2, column 102 hits nothing. Zoomed: column 60 hits the focused pane.
- `set_window_and_pane_of`: `set_window(PaneId(1), 9)` then `pane_of(9) == Some(PaneId(1))`, `pane_of(1) == None`.
- `inner_insets_by_one_cell`: `inner(Rect::new(0, 0, 22, 6)) == Rect::new(1, 1, 20, 4)`.

**Change**
- Implement the types and functions from Interfaces, decisions 8 to 14, 17 (the flag only) and 19. Layout recursion splits with decision 10's math in `u32`. `leaves`, `full_layout` and `layout` all walk `first` before `second`. Keep the module free of I/O and of `App`.

**Acceptance**
- `panes.rs` has no dependency on `app.rs` or `ui`. All tests pass. The file stays under about 600 lines including tests; if not, move the tests to `crates/tui/src/panes_tests.rs` with `#[cfg(test)] #[path = "panes_tests.rs"] mod tests;`.

### M7.4 The pane tree: focus movement and divider resize

**Files**
- Modify `crates/tui/src/panes.rs`.

**Tests first**
- `focus_next_walks_depth_first_and_wraps`: `[A | [B / C]]`, focus A: `focus_next` gives B, then C, then A.
- `focus_direction_picks_the_nearest_overlapping_pane`: `[A | [B / C]]` in `Rect::new(0, 0, 100, 40)`. From A, Right: B (B and C tie on distance and overlap; B has the smaller `y`). From C, Left: A. From B, Down: C. From C, Up: B. From A, Left and Up: returns false, focus unchanged.
- `focus_direction_prefers_more_overlap`: `[[A / B] | C]` with the left Column ratio 30 (A small, B large), focus C: Left gives B.
- `focus_direction_uses_the_full_layout_while_zoomed`: two Row panes, zoomed, focus pane 1: Right focuses pane 2; `zoomed()` stays true and `layout` now returns pane 2 over the whole area.
- `shift_left_moves_the_divider_on_the_left_side_first`: `[A | B]`, ratio 50, area width 100, focus B: `resize(Left)` makes the ratio 45 (B grows).
- `shift_left_at_the_left_edge_moves_the_right_divider`: same tree, focus A: `resize(Left)` makes the ratio 45 (A shrinks).
- `resize_prefers_the_divider_on_that_side_over_the_nearest`: `[A | [B | C]]`, focus B: `resize(Left)` changes the outer ratio from 50 to 45 and leaves the inner ratio at 50.
- `resize_uses_the_matching_axis`: `[A | [B / C]]`, focus C: `resize(Up)` changes the inner Column ratio to 45; `resize(Left)` changes the outer Row ratio to 45.
- `resize_clamps_to_ten_and_ninety`: area `Rect::new(0, 0, 400, 40)`, focus B in `[A | B]`: sixteen `resize(Right)` calls end at ratio 90; sixteen `resize(Left)` calls end at 10.
- `resize_that_breaks_the_minimum_is_refused`: `[A | B]` in width 50, focus B: `resize(Left)` gives 45 (first outer width 22); the next `resize(Left)` returns false and the ratio stays 45 (first outer width 20 would be too narrow).
- `resize_without_a_split_on_that_axis_does_nothing`: `[A / B]` (Column), `resize(Left)` returns false.

**Change**
- Implement decisions 15, 16 and 18. For resize, collect the focused leaf's ancestors from nearest to root with the side the leaf is on, choose the split by decision 18, change a copy of the ratio, and keep the change only if `fits(area)` holds afterwards.

**Acceptance**
- All tests pass. No function in `panes.rs` panics on a zero-sized area; add `zero_area_is_handled` asserting `layout(Rect::default())` returns one entry per visible pane with zero sizes and `fits` is false.

### M7.5 Window views and wheel encoding

**Files**
- Create `crates/tui/src/view.rs`. Modify `crates/tui/src/lib.rs` (`pub mod view;`).
- Modify `crates/tui/src/app.rs`: `on_scroll` uses `view::wheel` on the existing single parser.

**Tests first** (unit tests in `view.rs`)
- `sgr_wheel_report`: parser fed `\x1b[?1000h\x1b[?1006h`; `wheel(screen, false, 6, 6) == Wheel::Send(b"\x1b[<65;6;6M")`; up gives button 64.
- `default_encoding_wheel_report`: fed `\x1b[?1000h` only; `wheel(screen, true, 1, 2) == Send(vec![0x1b, b'[', b'M', 96, 33, 34])`.
- `default_encoding_out_of_range_is_ignored`: fed `\x1b[?1000h`; `wheel(screen, true, 224, 5) == Wheel::Ignore`; `x = 223` still sends, ending in byte 255.
- `utf8_encoding_wheel_report`: fed `\x1b[?1000h\x1b[?1005h`; `wheel(screen, true, 100, 5)` sends `ESC [ M`, then `char(96)`, `char(132)` UTF-8 encoded (`0xC2 0x84`), then `char(37)`; `x = 2016` gives `Ignore`.
- `alternate_screen_without_mouse_sends_arrows`: fed `\x1b[?1049h`; up gives `Send(b"\x1b[A\x1b[A\x1b[A")`, down gives three `\x1b[B`. After `\x1b[?1h` (application cursor), up gives three `\x1bOA`.
- `primary_screen_without_mouse_scrolls_locally`: fresh parser gives `Wheel::ScrollLocal`.
- `mouse_mode_wins_over_alternate_screen`: fed `\x1b[?1049h\x1b[?1000h\x1b[?1006h` gives an SGR report, not arrows.
- `view_scroll_and_snap_back`: `WindowView::new(80, 24, 5000)`, feed 40 lines, `scroll_local(true)` sets `scroll_offset == 3`; `scroll_to_live()` sets it to 0.
- `set_size_marks_a_debounced_resize`: `set_size(100, 30, t0)` returns true and resizes the parser to (30, 100); `take_due_resize(t0 + 10ms)` is `None`; `take_due_resize(t0 + RESIZE_DEBOUNCE)` is `Some((100, 30))`; a second call is `None`; `set_size(100, 30, t1)` returns false.
- `apply_snapshot_replaces_the_parser`: after `apply_snapshot(90, 20, b"hello")` the screen size is (20, 90), contents start with `hello`, `scroll_offset == 0`.
- `crates/tui/src/app.rs`: keep `wheel_scrolls_the_local_scrollback_and_any_key_snaps_back` passing, and add `wheel_in_an_alternate_screen_app_sends_arrow_keys` (feed `\x1b[?1049h` to `app.parser`, wheel up inside the main area, expect one `Input` with three `\x1b[A`).

**Change**
- Implement `view.rs` per Interfaces and decision 28. `RESIZE_DEBOUNCE` stays defined in `app.rs`; `view.rs` imports it. The App passes the scrollback length to `WindowView::new` (decision 20). The arrow bytes come from `crate::keymap::encode_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE), screen.application_cursor())`, repeated `WHEEL_LINES` times into one `Send`.
- `App::on_scroll` keeps its milestone-1 signature for now and replaces its SGR-only branch and local scroll with a `match view::wheel(..)`. `Ignore` returns no effects.

**Acceptance**
- The two wheel follow-ups are closed for the single-pane client. All tests pass.

### M7.6 App migration: pane tree and one view per visible window

**Files**
- Modify `crates/tui/src/app.rs` and `crates/tui/src/app_tests.rs` (milestone 4 already moved the tests there; with milestone 6 these are `app/mod.rs` and `app/tests.rs`).
- Modify `crates/tui/src/lib.rs` (`draw` and the mouse arms), `crates/tui/src/ui/mod.rs`, `crates/tui/src/ui/terminal.rs`, `crates/tui/src/ui/sidebar.rs` (or milestone 4's tree renderer, for `app.focused()`), and every other file that reads `app.parser`, `app.focused` or `app.scroll_offset` (`grep -rn "\.parser\|\.focused\b\|scroll_offset" crates/tui`).
- If milestone 6 is done: its reconnect code.

**Tests first** (in `app_tests.rs`. The helper `app_with` now calls `app.set_main_area(Rect::new(0, 0, 102, 26))`, which gives a single pane with a 100 by 24 inner area. Use that size in expectations. After a Row split in that area each pane's inner area is 49 by 24.)
- Rewrite the milestone-1 tests for the new shape and keep their intent: `first_size_report_subscribes_to_the_first_window` (`set_main_area(Rect::new(0, 0, 102, 32))` gives `[Subscribe { 4, 100, 30 }]` and `focused() == Some(4)`, the view's parser size is (30, 100)); `focus_requested_before_the_first_size_report_subscribes_once_sized`; `later_size_changes_are_debounced_into_a_resize` (use `on_tick_at` with explicit instants, no sleep); `snapshot_and_output_feed_the_focused_parser_only` becomes `output_for_a_window_without_a_view_is_ignored`; `keys_go_to_the_focused_window_and_prefix_switches` expects `[Unsubscribe { 1 }, Subscribe { 2, 100, 24 }]`; `focusing_the_already_focused_window_does_nothing`; `removed_focused_window_moves_focus_to_a_neighbour` expects `[Unsubscribe { 2 }, Subscribe { 3, 100, 24 }]` and then `[Unsubscribe { 3 }]` with `panes == None` when the list becomes empty; `paste_uses_bracketed_mode_when_the_program_asked_for_it`; `wheel_scrolls_the_local_scrollback_and_any_key_snaps_back` with the new `on_scroll(up, column, row, &layout)` signature and coordinates inside the pane's inner rect.
- New: `each_visible_window_has_its_own_parser`: build two panes directly (`app.panes.as_mut().unwrap().split(Axis::Row, 2)` followed by a call that reconciles, for example `set_main_area` with the same area; see Change for the helper), feed `Snapshot` and `Output` for windows 1 and 2, assert each view's contents hold only its own text.
- New: `switching_to_a_visible_window_moves_focus_to_its_pane`: two panes showing 1 and 2, focus on 2. `focus(1)` returns no effects and `focused() == Some(1)`.
- New: `switching_to_a_hidden_window_replaces_the_focused_pane`: same, then `focus(3)` returns `[Unsubscribe { 1 }, Subscribe { 3, 49, 24 }]` and pane 1 now shows 3.
- New: `resize_debounce_is_per_window`: two panes; change the main area; `on_tick_at(now)` right away returns nothing; `on_tick_at(now + RESIZE_DEBOUNCE)` returns one `Resize` per window, in window-id order, each with its pane's new inner size.
- New: `removed_window_in_a_split_closes_its_pane`: two panes, window 2 disappears from `WindowsChanged`: effects `[Unsubscribe { 2 }]`, one pane left, and after the debounce `[Resize { 1, 100, 24 }]`.
- New: `exited_window_keeps_its_pane`: two panes, window 2's status becomes `Exited`: no effects, two panes remain.
- New: `visible_windows_do_not_toast`: two panes showing 1 and 2, window 3 hidden; all three go to `Attention`; the toast names window 3 only.
- New: `new_window_opens_in_the_focused_pane_at_its_size`: `C-b c`'s `CreateWindow` carries the focused pane's inner size (or milestone 5's dialog does, if it replaced `C-b c`), and the `Created` window replaces the focused pane's window.

**Change**
- Replace the single-window fields with `panes`, `views` and `main_area` (Interfaces). Add `focused()`, `focused_view()`, `visible_windows()`, `pane_layout()`, `set_main_area()`, `on_tick_at()`.
- Implement `reconcile` (decision 21) and call it at the end of `focus`, `set_main_area`, `replace_windows` and every pane command added later. For tests that build trees directly, add `#[cfg(test)] pub(crate) fn sync(&mut self) -> Vec<Effect>` that just calls `reconcile`.
- `set_main_area(area)`: ignore zero-sized areas; on the first non-zero area resolve `pending_focus` or the first window as `ensure_focus` does today, building a single-leaf tree; afterwards store the area and reconcile.
- `focus(id)`: decision 23. Keep the milestone-1 rules for unknown ids and for calls before the first size report (`pending_focus`).
- `Snapshot` goes to `views[window_id].apply_snapshot`, `Output` to `views[window_id].parser.process`; both are ignored when there is no view.
- `replace_windows`: decisions 25 and 29.
- `on_key` and `on_paste`: decision 26. `NewWindow` uses the focused pane's inner size, or the main area's inner size when there is no pane.
- `on_scroll(up, column, row, layout)`: milestone 4's sidebar list first; otherwise find the pane with `hit_test`, require the position to be inside its inner rect, focus it, then apply `view::wheel` (decision 28).
- `on_click(column, row, layout)`: milestone 4's overview and sidebar handling, then decision 27.
- `lib.rs`: `draw` calls `set_main_area(layout.main)`. The left-click arm keeps calling `app.on_click(column, row, &layout)`, as milestone 4 made it. Scroll arms call `app.on_scroll(up, column, row, &layout)`. `set_tree_viewports` gets the overview height from decision 36.
- `ui/terminal.rs`: implement `render` and `render_pane` so every pane of `app.pane_layout(area)` is drawn with its own block and `PseudoTerminal` from its view. Plain titles are enough in this task; M7.8 finishes them. `ui::Layout` loses `main_inner`; update `layout_splits_sidebar_main_and_statusbar`.
- If milestone 6 is done, its reconnect path re-subscribes every window in `views` with each view's size and resets each view's parser, instead of the single focused window. Milestone 6's `App.subscribed: Option<u32>` becomes the per-view `WindowView::subscribed` flag: `on_send_failed` for a `Subscribe` clears that view's flag, and `on_tick` re-sends `Subscribe` for every view whose flag is false while connected. Add `reconnect_resubscribes_every_visible_window` and `a_refused_subscribe_is_retried_per_window` next to milestone 6's reconnect tests.

**Acceptance**
- `grep -rn "app\.parser\|\.scroll_offset\b" crates/tui/src` finds only uses through a `WindowView`.
- With one pane the client behaves as before: the smoke script passes unchanged.
- `app.rs` stays under about 600 lines; if not, move key and mouse handlers into `crates/tui/src/app_input.rs` as an `impl App` block.

### M7.7 Pane key bindings and the window picker

**Files**
- Modify `crates/tui/src/keymap.rs`, `crates/tui/src/app.rs`, `crates/tui/src/app_tests.rs`.
- If milestone 6 is done: the config module, for `panes.max`. Otherwise `docs/superpowers/plans/2026-09-17-anthrex-foundation-followups.md` (one line under milestone 7, decision 34).

**Tests first**
- `crates/tui/src/keymap.rs`:
  - `pane_commands_after_the_prefix`: `C-b |` with `NONE` and with `SHIFT` both give `Run(SplitRow)`; `C-b -` `SplitColumn`; `C-b o` `NextPane`; `C-b z` `ZoomPane`; `C-b w` `ClosePane`; `C-b O` (with `SHIFT`) is not `NextPane`.
  - `arrows_after_the_prefix_focus_and_shift_arrows_resize`: Left, Right, Up, Down with `NONE` give `FocusPane(..)` for each direction; with `SHIFT` give `ResizePane(..)`; Left with `CONTROL` gives `Nothing`; `pending()` is false after each.
  - `shift_arrow_without_the_prefix_passes_through`: `handle(Left + SHIFT)` gives `Send(b"\x1b[1;2D")`; Up with `SHIFT` gives `Send(b"\x1b[1;2A")`.
  - Change `prefix_twice_sends_literal_and_escape_cancels` to use an unbound key, `C-b y`, where it used `C-b z`.
- `crates/tui/src/app_tests.rs`:
  - `split_opens_the_picker_and_enter_splits`: windows 1, 2, 3; main area `Rect::new(0, 0, 102, 26)`; `C-b |` returns no effects and sets `Modal::PanePicker { axis: Row, candidates: [2, 3], selected: 0 }`. `Enter` returns `[Subscribe { 2, 49, 24 }]`, closes the modal, and `focused() == Some(2)`. `on_tick_at(now + RESIZE_DEBOUNCE)` returns `[Resize { 1, 49, 24 }]`.
  - `picker_keys_move_pick_and_cancel`: `j` moves to 1, `j` wraps to 0, `k` wraps to 1; `Esc` closes with no effects and one pane; reopening and pressing `2` picks the second candidate directly.
  - `split_is_refused_with_a_reason`: with every window visible, `C-b |` toasts `every window is already shown` and opens no modal; in `Rect::new(0, 0, 40, 26)` it toasts `no room for another pane`; with 6 panes in a large area it toasts `at most 6 panes`.
  - `next_pane_and_directional_focus_send_nothing`: two Row panes; `C-b o` and `C-b Left` change `focused()` and return no effects.
  - `resize_pane_sends_debounced_resizes_for_both_windows`: two Row panes, focus 2, `C-b Shift+Left`: no immediate effects; after the debounce `[Resize { 1, 43, 24 }, Resize { 2, 55, 24 }]` (outer widths `102 * 45 / 100 = 45` and 57).
  - `zoom_resizes_the_zoomed_window_and_unzoom_restores_it`: two panes, focus 2, `C-b z`: after the debounce `[Resize { 2, 100, 24 }]`; `C-b z` again: after the debounce `[Resize { 2, 49, 24 }]`. Window 1 gets no resize while hidden.
  - `close_pane_unsubscribes_and_keeps_the_window`: two panes, focus 2, `C-b w`: `[Unsubscribe { 2 }]` and no `Kill` or `Remove`; after the debounce `[Resize { 1, 100, 24 }]`. `C-b w` again toasts `this is the only pane` with no effects.
  - `too_small_area_auto_zooms_and_recovers`: two Row panes showing 1 and 2, focus 2; `set_main_area(Rect::new(0, 0, 40, 26))` then the debounce: exactly `[Resize { 2, 38, 24 }]`; window 1 is hidden and keeps 49 by 24. Back to `Rect::new(0, 0, 102, 26)` then the debounce: exactly `[Resize { 2, 49, 24 }]`.

**Change**
- `keymap.rs`: add the `Command` variants and decision 32's matching. Arrows are matched on `(code, modifiers)`; `Char` keys keep ignoring modifiers except that `O` and `o` stay distinct because they are different characters.
- `app.rs`: handle each command. `SplitRow` and `SplitColumn` run decision 13's checks, then open the picker. The picker's `Enter` or digit calls `split`, then `reconcile`. `NextPane`, `FocusPane` and `ZoomPane` change the tree and reconcile. `ResizePane` calls `PaneTree::resize(dir, main_area)` and reconciles. `ClosePane` calls `close(focused)` and reconciles, or toasts on `LastPane`. Apply decision 34.

**Acceptance**
- Every binding from spec 7.2 works in unit tests. No existing binding changed meaning.

### M7.8 Per-pane rendering, the picker modal and help

**Files**
- Modify `crates/tui/src/ui/terminal.rs`, `crates/tui/src/ui/modal.rs`, `crates/tui/src/ui/mod.rs` (tests), and milestone 4's tree row renderer (decision 35).

**Tests first** (`TestBackend`, in `crates/tui/src/ui/mod.rs` tests or a new `crates/tui/src/ui/panes_tests.rs` if that file grows past about 600 lines)
- `two_panes_render_side_by_side_with_their_own_titles_and_screens`: windows `left` (Shell, Idle) and `right` (Codex, Working); split Row so `right` is focused; snapshots `LEFT-TEXT` and `RIGHT-TEXT`; render at 140 by 30 with the sidebar visible. Assert both texts and both titles (`○ left · shell`, `right · codex`) are in the output. Using the returned `ui::Layout` and `app.pane_layout(layout.main)`, assert that the cell at each pane's top-left corner (`buffer.cell((x, y))`) has `fg ==` the accent (`app.settings.accent` with milestone 6, `theme::ACCENT` without) for the focused pane and `fg == theme::DIM` for the other, and that `LEFT-TEXT` starts at a column inside the left pane's inner rect.
- `the_title_shows_model_and_zoom`: a window with `model: Some("opus")` shows `· claude opus` in its title; after `C-b z` the title contains `[zoom]`.
- `too_small_terminal_renders_only_the_focused_pane`: two Row panes, render at 40 by 12 with the sidebar hidden (each pane would be 20 columns wide): only the focused title appears, followed by `[zoom]`.
- `only_the_focused_pane_gets_the_cursor`: after rendering two panes, `terminal.get_cursor_position()` is inside the focused pane's inner rect.
- `the_picker_lists_hidden_windows`: windows `a`, `b`, `c` with `a` shown; open the picker; the output contains `show in new pane`, `1`, `b`, `2`, `c`, and `Enter show · j/k move · Esc cancel`; it does not list `a`.
- `help_lists_the_pane_keys`: the help modal contains `split side by side / stacked` and `close pane (window keeps running)`.
- `visible_windows_are_marked_in_the_sidebar`: two panes showing windows 1 and 2, window 3 hidden: the rows for 1 and 2 start with `▎`, the row for 3 does not; the bar for the focused window has the accent as `fg`, the other `theme::DIM`.

**Change**
- Decisions 30, 31 and 35, the picker drawing and the help rows from Interfaces. The picker is sized to its longest line plus 4 and centred like the other modals.

**Acceptance**
- The tests pass. A manual look at a 120 by 40 terminal matches the sketch in Interfaces.

### M7.9 Smoke stage: two windows side by side

**Files**
- Modify `scripts/pty-smoke.py`.

**Steps of the new stage**, called `stage 8d: split panes`, placed after milestone 4's stage 8c and before stage 9. Windows `shell-1` to `shell-4` already exist; the stages of milestones 3 and 4 remove the windows they create.

1. Start `proc4 = PtyProc([BIN])` and wait for `agents`, milestone 4's sidebar title.
2. Send `\x021` (focus `shell-1`), then `\x02|`. Wait for `show in new pane`.
3. Send `\r` to pick the first entry, `shell-2`. Wait until the screen contains both `shell-1 ·` and `shell-2 ·` titles.
4. Type `echo pane-$((6*7))-right\r` (the new pane is focused). Wait for `pane-42-right`.
5. Send `\x02\x1b[D` (prefix, then Left). Type `echo pane-$((6*7))-left\r`. Wait for `pane-42-left`.
6. On one reconstructed screen, find the row and column of both markers. Assert the column range of `pane-42-left` ends before the column where `pane-42-right` starts. This proves the two windows are side by side.
7. Send `\x02\x1b[1;2C` (prefix, then Shift+Right). Wait 0.3 s while draining output, then assert both markers are still on screen.
8. Send `\x02z`. Wait until `pane-42-right` is gone and `[zoom]` is on screen. Send `\x02z` again and wait until `pane-42-right` is back.
9. Send `\x02d`, wait for exit status 0, close the PTY.

Print one `ok:` line per assertion group, as the other stages do.

**Acceptance**
- `python3 scripts/pty-smoke.py` passes three runs in a row on macOS.

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

1. `grep -rn "PROTO_VERSION: u32 = 4" crates/proto/src/lib.rs` matches.
2. `cargo test -p anthrex-daemon --test server` runs the six subscription tests and the two jiggle tests.
3. `cargo test -p anthrex-tui panes` and `cargo test -p anthrex-tui view` run the layout and wheel tests.
4. The smoke output contains the stage 8d `ok:` lines.
5. `pgrep -fl "anthrex daemon"` shows nothing of yours afterwards.

## Manual check

Use an isolated daemon for every step:

```bash
export ANTHREX_SOCKET=/tmp/anthrex-m7.sock ANTHREX_DATA_DIR=/tmp/anthrex-m7-data
cargo build && ./target/debug/anthrex
```

1. Create a Claude window and a Codex window (`anthrex new --runtime claude` and `--runtime codex` from a second terminal with the same variables, or milestone 5's dialog). In the client, focus the Claude window and press `C-b |`. The picker lists the Codex window. Pick it. Both agents are visible, each with its own title, glyph and border; the focused border is blue.
2. Type a prompt into the Codex pane. Press `C-b` Left and type into the Claude pane. Keystrokes reach only the focused pane.
3. Watch both agents work at once. Status glyphs in the titles and in the sidebar update in both panes. The sidebar marks both windows as visible.
4. Press `C-b -` in the Claude pane and pick a shell. Three panes. Walk them with `C-b o` and with `C-b` plus each arrow; focus follows the geometry.
5. Press `C-b` Shift+Left and Shift+Right several times. The divider moves by about 5 percent each time and stops before a pane gets narrower than 20 columns. Each agent redraws at its new width.
6. Press `C-b z`. The focused pane fills the main area and its title shows `[zoom]`. Press `C-b o`: the next pane is shown zoomed. Press `C-b z` to return.
7. Shrink the terminal window until the panes cannot fit. Only the focused pane is shown, with `[zoom]`. Grow it back; the layout returns.
8. In the shell pane, run `less /etc/services`. Scroll with the wheel: `less` scrolls (alternate screen without mouse mode). Quit. Run `vim -c 'set mouse=a ttymouse=xterm' /etc/services` and scroll: vim scrolls (default X10 encoding). Repeat with `ttymouse=sgr`.
9. Wheel over a pane that is not focused. It becomes focused and scrolls.
10. Press `C-b w` on the shell pane. The pane closes, its neighbour takes the space, and `anthrex ls` still lists the shell as running.
11. Detach with `C-b d` and attach again. One pane shows. Focus the Codex window: it repaints fully (the jiggle).
12. From the second terminal, `anthrex rm` the window shown in a non-focused pane. That pane closes and the rest take its space.
13. Finish with `anthrex daemon stop` using the same variables, and check `pgrep -fl "anthrex daemon"` is empty.

## Risks and gotchas

1. **Milestone-4 names.** This brief uses the milestone 4 brief's names ("Starting point"). Before M7.1, run `grep -rn "fn focus\|focused\|parser\|scroll_offset\|FocusIndex\|Command::" crates/tui/src` and map every use. If a tree action reaches a window without going through `App::focus`, route it through `focus` (decision 23) and record that in "Implementation notes".
2. **Effect order in tests.** `reconcile` emits unsubscribes before subscribes, and debounced resizes in `views` key order. Tests assert exact vectors; if one fails on order only, fix the code to match the decisions, not the test.
3. **`|` arrives with SHIFT.** Most terminals report `|` as `Char('|')` with `KeyModifiers::SHIFT` in crossterm 0.29. Match `Char('|')` regardless of modifiers.
4. **Shift+arrow may not reach the client.** Some terminal emulators bind Shift+arrow themselves (for example to scroll or switch tabs). If the manual check shows nothing happens, confirm with `cat -v` outside anthrex; if the terminal eats the key, note it in "Implementation notes". Do not change the binding.
5. **The jiggle and the snapshot race.** The repaint the jiggle triggers must arrive as `Output` after the `Snapshot`. That holds because the jiggle runs before `attach`, and the forwarder is spawned after the snapshot is queued. Do not move `resize_for_attach` after `attach`.
6. **A program that reads its size during the jiggle.** It may draw once at `rows - 1`. The second resize sends a second SIGWINCH, so it redraws at the right size. If a real app stays one row short in the manual check, record it and stop on decision 7.
7. **Finished forwarders count toward the limit.** Without the prune in decision 3, a client that shows 16 windows over time and removes them would hit the limit. The `a_removed_window_frees_its_subscription_slot` test guards this.
8. **Hidden panes still stream.** Windows in panes hidden by zoom stay subscribed so their parsers stay current. That costs bandwidth, bounded by the pane limit (at most 16). Do not unsubscribe on zoom.
9. **Overflow in ratio math.** `u16 * u8` overflows for wide terminals. Compute in `u32`.
10. **Snapshot size differs from pane size.** With two clients at different sizes, a snapshot can arrive at the other client's size. The view adopts the snapshot's size, as milestone 1 does. The next layout change resizes it back. This is the documented last-resize-wins limitation.
11. **Tests that spawn 17 shells.** They take a few seconds and 17 PTYs. Remove every window at the end of the test so the shells do not outlive it.
12. **`app.rs` size.** Milestones 3 to 6 have already grown `app.rs`. Split as M7.6 and M7.7 describe rather than letting it pass about 600 lines.

## Follow-ups handled

From `docs/superpowers/plans/2026-09-17-anthrex-foundation-followups.md`, "Assignment to milestones", row M7:

| Item | Where |
|------|-------|
| SIGWINCH jiggle on attach so full-screen apps repaint | Decision 7, task M7.2 |
| Wheel scrolling for alternate-screen apps without mouse mode | Decision 28 (b), tasks M7.5 and M7.6 |
| Check `mouse_protocol_encoding()` instead of assuming SGR | Decision 28 (a), tasks M7.5 and M7.6 |

## Implementation notes

The implementer fills this section in during implementation: every deviation from the brief, every surprise, and every decision taken, with evidence.
