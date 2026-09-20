# anthrex design: the graph overview, and sub-agent labels worth reading

Date: 2026-09-20. Status: binding. Implemented by milestone 4.6.

Amends `docs/superpowers/specs/2026-09-18-anthrex-product-design.md` §5.1 (what the tree shows) and §10.3 (keys). Section 6 below lists exactly what it supersedes.

## 1. What changes

1. **Sub-agent labels become informative.** Today every Claude sub-agent row reads `Explore: Repository: /Users/…` — the first line of the agent's prompt, which is repository boilerplate. The label now comes from the Agent tool's `description` field, which exists precisely to be a short summary.
2. **The full-screen overview becomes a drawn tree.** `C-b T` stops showing indented rows in aligned columns and shows a left-to-right node-and-edge diagram on a pannable canvas.

Neither change touches the protocol. `SubagentInfo.label` already exists and the graph is client-only, so this milestone can land beside milestone 5 without colliding with its protocol bump.

## 2. Terms

- **node** — one drawn box: a project, an agent window, or a sub-agent.
- **tier** — the column of nodes at one depth. Tier 0 is projects.
- **canvas** — the full laid-out drawing, usually larger than the terminal.
- **viewport** — the window onto the canvas that is actually drawn.

## 3. Sub-agent labels

`daemon::subagents::spawn_request` declares, for Claude, the tool-input keys it reads. Today they are `("name", "prompt")`. Claude Code's Agent tool sends `subagent_type`, `description` and `prompt`; it does not send `name`, so every label falls through to the first line of `prompt`.

The order becomes **`description`, then `name`, then the first line of `prompt`**, each skipped when absent or empty. `name` is kept because it costs nothing and other spawners may send it. `LABEL_MAX_CHARS` stays at 60; `description` is short by design, so the cap will rarely bite.

Codex's keys are unchanged.

This is a two-line change with four tests, and it is a precondition for the graph: a node box has room for about twenty characters, and `Repository: /Users/…` uses all of them saying nothing.

## 4. The graph overview

### 4.1 Orientation and why

Left to right: the root tier on the left, each tier of children to its right, siblings stacked vertically.

A tree's extent along the sibling axis grows with its **leaf count**; along the tier axis it grows with **depth**. anthrex's depth is small and bounded in practice — project, agent, sub-agent, sub-sub-agent — while its leaf count is not: seven sessions with four sub-agents each is thirty leaves, and orchestration will multiply that. Drawn downward, thirty leaves at 24 columns each is roughly 700 columns of horizontal panning. Rotated, the same tree is about 110 columns wide and scrolls vertically, which is the axis terminals and mice already scroll.

### 4.2 Layout

Pure, deterministic, one pass, no floating point. Input is the same visible row list the sidebar already builds, so collapse, the filter and selection keep working unchanged.

```rust
pub const MIN_NODE_WIDTH: u16 = 12;  // including both borders
pub const MAX_NODE_WIDTH: u16 = 30;
pub const NODE_HEIGHT: u16 = 3;      // top border, content, bottom border
pub const TIER_GAP: u16 = 3;         // columns between a tier's right edge and the next tier
pub const ROW_GAP: u16 = 1;          // blank rows between sibling boxes

pub struct PlacedNode { pub key: NodeKey, pub rect: Rect, pub depth: u16 }
pub struct Layout { pub nodes: Vec<PlacedNode>, pub edges: Vec<Edge>, pub size: (u16, u16) }
pub fn layout(rows: &[Row<'_>]) -> Layout;
```

- **Every node in a tier shares one width**, so the tier's boxes and the edges leaving them line up. That width is the widest content in the tier plus the two borders and one space of padding each side, clamped to `MIN_NODE_WIDTH..=MAX_NODE_WIDTH`. Tiers differ: a tier of project names is narrow, a tier of sub-agent labels is wide, and the drawing is tighter than one global width would make it.
- `x` is the sum of the widths of the tiers to the left, plus `TIER_GAP` for each.
- `y` is assigned by a post-order walk. A leaf takes the next free row and advances it by `NODE_HEIGHT + ROW_GAP`. A parent's `y` centres it on the span from its first child's top to its last child's bottom, rounded down.
- `size` is the bounding box of every placed node.

Widths are measured with `unicode_width`, and content is truncated to the tier's width before painting, so a single very long label widens its tier only to the cap.

A node with no visible children is a leaf for layout, whether it truly has none or is collapsed.

### 4.3 Drawing

Nodes are rounded boxes in the style the app already uses:

```
╭──────────╮   ╭──────────────────╮   ╭────────────────────────╮
│ anthrex  ├───┤ ◆ 2 api          ├─┬─┤ ✓ Explore: call sites  │
╰──────────╯   ╰──────────────────╯ │ ╰────────────────────────╯
               ╭──────────────────╮ │ ╭────────────────────────╮
               │ ○ 1 shell-1      │ └─┤ ⣾ Plan: migration      │
               ╰──────────────────╯   ╰────────────────────────╯
```

Content is one row: the status glyph in its status colour, then the window's tree position for window nodes, then the name or label, truncated to the remaining display width with `…`. The focused window's box uses the focused border style; the selected node's box is highlighted the way the selected row is today.

Edges occupy the `TIER_GAP` columns. A parent connects from the middle of its right border. With one visible child the edge is a straight horizontal run. With several, a vertical bus sits in the middle gap column, spanning the first child's middle row to the last child's; each child is joined by a horizontal run into the middle of its left border. Junction glyphs follow from which arms meet at a cell: `┌` at the bus's top and `└` at its bottom, `├` at a middle child, `┤` at the parent's row when no child sits there, and `┬`, `┴` or `┼` where the parent's row coincides with the top, the bottom or a middle child respectively. Corners are sharp, not rounded — `└` already means "last child" in the sidebar's guides. Borders that an edge meets become `├` on the parent and `┤` on the child.

**Painting is a blit.** `paint(&Layout, viewport, &App) -> Vec<Line<'static>>` fills a character grid the size of the viewport and converts it to ratatui lines. Layout and painting are separately testable: layout by asserting node rectangles, painting by asserting exact strings for small trees.

### 4.4 The viewport

`Pan { x: u16, y: u16 }` is the canvas coordinate drawn at the viewport's top-left corner, clamped so the viewport never extends past the canvas.

After any change of selection, rows or focus, the anchor is revealed: if the selected node's rectangle is not wholly inside the viewport, the pan moves by the smallest amount that puts it inside. This is the two-dimensional form of the rule the list already uses.

There are no panning keys. The selection reveals itself, the mouse wheel scrolls vertically by three rows, and dragging with the left button pans both axes. Anything more is unnecessary while the selection can reach every node.

### 4.5 Interaction

The keys are the tree-mode keys that already exist, so nothing has to be relearned:

| Key | Action |
|---|---|
| `j` / `k`, `Down` / `Up` | previous and next node in visible tree order |
| `h` / `l`, `Left` / `Right` | the parent; the first visible child |
| `Enter` | focus this window, or the window owning this sub-agent |
| `Space` | fold or unfold |
| `/` | filter |
| `Esc` | leave the overview |

`h` and `l` are new and exist because a drawn tree makes the parent relationship visible; in the sidebar they had nothing to mean.

The footer shows the selected node in full: its label untruncated, its model, its state and its elapsed time. That is where the aligned columns go.

### 4.6 Geometry and hit-testing

`GraphGeometry { area: Rect, pan: Pan }` with `node_at(column, row) -> Option<NodeKey>`, which maps a screen cell to a canvas cell and returns the node whose rectangle contains it, or `None` for a cell on an edge, in a gap, or outside the canvas. A linear scan over placed nodes is fine: a click is not a hot path, and the node count is in the hundreds at worst.

Clicking a node selects it. Double-clicking focuses it.

### 4.7 What is deleted

`WideColumns` and `wide_line` in `crates/tui/src/ui/tree_view.rs` exist only for the overview. When the overview stops rendering rows they become dead and are removed, along with their tests. `narrow_line` and the sidebar are untouched, as is `anthrex tree` on the command line, which has its own formatting.

## 5. Testing

- Layout: exact rectangles for a known tree; a parent centred on two children; a parent with one child; a collapsed node treated as a leaf; a six-level chain; a tree whose first tier has several roots; a tier whose widest label exceeds the cap, and one whose widest is below the floor.
- Painting: exact strings for a three-node tree, for a one-child straight edge, for a three-child bus, and for a node whose label needs truncating. A label containing wide characters must not push the border out of alignment.
- Viewport: reveal moves by the smallest amount on each axis; the pan clamps at both ends; a selection change that is already visible does not move the pan.
- Hit-testing: every cell of a node's rectangle returns that node; an edge cell, a gap cell and a cell past the canvas return `None`.
- Keys: `h` at a root and `l` at a leaf are no-ops; `j`/`k` order matches the sidebar's order exactly, which is asserted against the existing row builder rather than restated.
- The PTY smoke script gains a stage that opens the overview and asserts a box border and an edge glyph are on screen.

## 6. What this supersedes

| In `2026-09-18-anthrex-product-design.md` | Status |
|---|---|
| §5.1's description of the wide overview as aligned columns | replaced by §4 here |
| §10.3's entry for `C-b T` | still `C-b T`; its contents are §4.5 here |
| everything else | unchanged and still binding |

In `2026-09-20-git-surface-and-simple-orchestration-design.md`, §4.4's requirement that the sidebar and the overview share guide strings no longer applies to the overview, which no longer draws guides. The sidebar's guides are unchanged.

## 7. Risks

- **The canvas can be large.** Layout is linear and painting is bounded by the viewport, so size costs memory for the node list and nothing else. A pathological tree — hundreds of leaves — is slow to *read*, not slow to draw.
- **Wide characters.** Box content must be measured with `unicode_width`. A CJK label that overflows by one cell breaks every border to its right, and it will not be obvious which node caused it.
- **Edge junctions are fiddly.** The combinations of first-child, last-child, only-child and parent-row-coincides are where this will be wrong. Each is a named painting test.
- **Losing the columns is a real loss** for one job: sorting thirty agents by elapsed time. Nothing in this design replaces that. If it turns out to matter, `anthrex ls` already does it and does it better.
