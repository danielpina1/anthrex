# The README screenshots

Every image in `docs/images/` is rendered from the real `anthrex` client. Nothing is drawn by hand. To regenerate them:

```bash
cargo build --workspace --examples
python3 scripts/screenshots.py            # both scenes, about five minutes
python3 scripts/screenshots.py agents     # or one scene: agents, design
```

## How it works

1. **`scripts/screenshots.py`** starts a daemon of its own. Its socket, data directory, config and `HOME` all live in one fresh `/tmp/anthrex-shots-*` directory, which the script removes at the end (set `ANTHREX_SHOTS_KEEP=1` to keep it for debugging). `ANTHREX_CLAUDE_BIN`, `ANTHREX_CODEX_BIN` and `ANTHREX_DECIDER_BIN` point at `target/debug/fake-agent`, and `ANTHREX_GH_BIN` at a path that does not exist, so no real agent or `gh` ever runs. Each scene ends with `anthrex daemon stop`.
2. It builds two made-up repositories, `acme-api` and `acme-web`, then drives the client in a 140 × 34 pseudo-terminal the way `scripts/pty-smoke.py` does: it sends keys and waits for text on the screen.
   - **agents**: four agents and a shell. Their statuses come from real hook events sent with `anthrex hook`, and the `api` agent's conversation comes from those hooks and a short transcript. Shots: `agents`, `new-agent`, `overview`, `conversation`.
   - **design**: a goal through the design flow and into its run. It reuses the smoke test's scripted agents: the orchestrator, the two brainstormers, the document reviewers, and workers and reviewers that commit and approve, or stay busy. Their scripts are read from `.git/fake-agent/` in the demo repository. Shots: `brainstorm-gate`, `spec-gate`, `plan-review`, `run-view`, `task-inspector`, `action-menu`.
3. At each shot, the script saves every byte the client has written and runs **`crates/tui/examples/render_svg.rs`**. The example replays the bytes through the `vt100` crate and writes the final screen as an SVG: one cell per glyph, with colours, bold, dim, italic, underline and inverse. Box-drawing and block characters are drawn as shapes, so frames join up whatever monospace font the viewer has. The screen sits in a window frame with a title bar.

`ANTHREX_SHOTS_VERBOSE=1` prints each screen's text as it is captured. To render a capture of your own:

```bash
target/debug/examples/render_svg --rows 34 --cols 140 --title "anthrex" capture.bin out.svg --text
```

## Checking the result

On macOS, `qlmanage -t -s 1400 -o /tmp/preview docs/images/run-view.svg` renders an SVG to a PNG. The thumbnail is square, so a wide image is cropped. Before committing new images, look at each one. Check that the glyphs are clean, that the content makes sense, and that no path or prompt shows a real user, host or home directory.
