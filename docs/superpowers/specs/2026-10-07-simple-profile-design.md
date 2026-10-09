# A simple repository profile: design

Date: 2026-10-07. Status: approved in conversation.

## 1. Goal

Getting and keeping a repository profile takes no manual steps beyond one review, and the profile
screen explains itself in plain words. Success:
- Starting the first goal in a repository with no profile never refuses: it sets the profile up,
  asks for one review, then starts the goal.
- The profile screen shows one plain status line, plain section and field labels, and no hidden
  toggles.
- Editing a command verifies it and saves it without a separate confirm step.

## 2. Decisions (from the user)

| # | Decision |
|---|---|
| D1 | Automatic set-up with one review card: **Use this** / **Edit**. |
| D2 | Plain sections with an **Advanced** section collapsed; plain labels, the config key dimmed beside each. |
| D3 | An edited command is verified once: ✓ saves at once; ✗ shows the output with **Save anyway** / **Revert**. Edits work on a proposal as well as on the stored profile. |

## 3. Today (summary)

A goal run refuses without a stored profile, pointing at `anthrex profile detect|show|confirm`
(`run/driver/adapt_goal.rs`). The Profile screen (`C-b P`; `tui/src/app/profile_screen.rs`,
`profile_pages.rs`, `profile_views.rs`, `ui/profile.rs`, `ui/profile_pages.rs`, `profile_view.rs`)
has two tabs (status, profile), a status vocabulary of stored / confirmed / proposal / ready /
failed / stale / unparseable / dropped, a hidden `s` "store once verification passes" toggle that
only affects edits, a confirm page showing raw TOML, raw key names, and edits that only start from
the stored profile. The daemon side (`daemon/src/profile/`) detects with an onboarding scout,
verifies each command in a confined checkout, keeps one proposal, and stores on confirm.

## 4. Design

### 4.1 Goal start without a profile

`adapt_goal` no longer refuses when the repository has no stored profile:
- No proposal and no detection running: it starts detection (as `profile detect` would, with the
  run's `trust_project` / `unconfined_checks` choices) and **queues** the goal.
- Detection running or verifying: it queues the goal.
- A proposal ready: it queues the goal and raises the review (4.2).
- When the profile is stored (by **Use this** or `profile confirm`), every queued goal for that
  repository starts. When detection fails, queued goals get one alert: `setting up <repo>
  failed: <reason>` with **Retry** and **Open profile**.
- `--yes` on `run start --goal` with a ready proposal stores it without review (today's `confirm
  --yes` semantics); with detection still running it waits for it, then stores only if nothing was
  dropped, else raises the review.
- The goal view shows `Setting up anthrex for this repo… reading the repo` / `… checking commands
  (2/4)` while queued.

Queued goals are kept in the profile service (one list per repository), persisted with the
proposal record, and survive a daemon restart.

### 4.2 The review card

When a proposal becomes ready, the TUI raises one alert, `<repo>: review how anthrex will work
here`; Enter opens the Profile screen on the proposal, which renders as the card:

```
 anthrex learned how to work in this repo

   How anthrex checks your work
     setup        cargo fetch                      ✓ 12s
     check        cargo test --workspace           ✓ 3m10s
     single test  cargo test -p {crate} {test}     ✓ 4s
   Your repo
     source       crates/          tests  crates/*/tests
     protected    .github/, Cargo.lock
   Delivery       pull request to main
   couldn't verify: lint (cargo clippy) — timed out after 600s

   ⏎ use this    e edit    a advanced    esc later
```

- **Use this** stores the proposal (today's confirm) and starts queued goals.
- **Later** (`esc`) keeps the proposal and the alert; queued goals keep waiting with `profile
  needs review` in their alert.
- A re-detection after the profile went out of date shows **only the changed rows**, each as
  `old → new`, with the same keys; the stored profile stays in use until **Use this**.

### 4.3 Editing

- `e` on a row edits it, on the stored profile or on the proposal (today edits only start from the
  stored profile).
- A command row (`setup`, `check`, `single_test`, tier commands) is verified once in the confined
  verify checkout after the edit (today's `REVERIFY_KEYS` path):
  - ✓: the edit is saved at once (to the stored profile, or into the proposal).
  - ✗: the row shows ✗, the output is one key away (`o`), and the user chooses **Save anyway**
    (`s`) or **Revert** (`r`).
- Non-command rows (paths, delivery, env, timeouts) are saved at once.
- The `s` "store once verification passes" toggle and its line are removed; the CLI's `profile
  edit --yes` keeps its meaning (store if verification passes) and gains `--anyway`.

### 4.4 The screen

One page, no tabs:

```
 profile · anthrex                                  Ready · verified 2 days ago

 How anthrex checks your work
   setup        cargo fetch                         ✓        setup
   check        cargo test --workspace              ✓        check
   single test  cargo test -p {crate} {test}        ✓        single_test
 Your repo
   source       crates/                                      source
   tests        crates/*/tests                               test_paths
   generated    —                                            generated
   protected    .github/, Cargo.lock                         protected
 Delivery       pull request to main                         delivery.mode
 Advanced ▸     testing tiers, output filter, environment, timeouts, shared code area

 ⏎ open   e edit   u unset   a advanced   d detect again   o output   esc back
```

- **Status line** (exactly one of):
  `Not set up — press d to set up` · `Setting up… reading the repo` · `Setting up… checking
  commands (2/4)` · `Needs review — anthrex has a proposal` · `Ready · verified <age> ago` ·
  `Out of date — <file> changed · re-checking` · `Out of date — <file> changed · review the
  changes` · `Can't read the profile file — ⏎ shows it`.
- The right column shows the config key, dimmed. Selecting a row shows a one-line hint under the
  list (e.g. `single test — how anthrex runs one test; {crate} and {test} are filled in`).
- **Advanced** (collapsed; `a` toggles): testing tiers (`build_check`, `module_test(s)`,
  `module_graph`, `module_names`, `full_triggers`, `slow_tests`, `timing_tests`, `skip_markers`,
  `full_shards`, `toolchain_id`), output filter (`output_filter`, `filter_prefixes`), environment
  (`env`), timeouts (`check_timeout_secs`), test result pattern (`test_passed`, `sample_test`),
  shared code area (`hub`), languages, modules, manifests, conventions.
- `x discard proposal` appears only when a proposal exists; its confirm text says what stops.
- The detect page explains its two options in plain words: **Trust this repo's agent settings**
  ("let agents use the settings files this repo tracks, like .claude/ and .mcp.json") and **Run
  checks outside the sandbox** ("only needed on systems where anthrex can't confine commands").
- The unreadable-profile message's action opens the raw file text (today's hint pointed at a key
  that does nothing on that view).

### 4.5 CLI

Commands stay; wording follows the screen. `profile status` prints the same status line.
`profile confirm` is kept as the CLI's **Use this**. Goal-start refusals that named profile
commands are replaced by the queued behaviour (4.1).

### 4.6 Protocol

Queued goals in the profile snapshot, the review alert payload, per-row verify results for edits,
and the `--anyway` edit flag are wire changes: one `PROTO_VERSION` bump, shared with milestones
9.8 / orchestrator-first alerts if they ship together (AGENTS.md rule 4), with round-trip tests.

## 5. Error handling

- Detection failure: one alert per repository with queued goals (Retry, Open profile); the queue
  is kept.
- A queued goal whose repository is removed or whose base branch is gone is dropped with a note.
- Verification of an edit that times out counts as ✗ (Save anyway / Revert).
- Edits during a live run in that repository stay refused, as today, with the plain message
  `finish or cancel the run in this repo to change its profile`.

## 6. Testing

- TUI render tests: every status line; the card (fresh and diff); Advanced collapsed and open;
  hints; key rows per state (no `x` without a proposal).
- Goal queue (daemon + fake scout end to end): no profile → detection starts → card → Use this →
  goal starts; detection failure → one alert → Retry; daemon restart keeps the queue; `--yes`.
- Edit flow: command ✓ saves; ✗ then Save anyway; ✗ then Revert; edit on a proposal;
  non-command saves at once.
- Out-of-date diff card shows only changed rows; stored profile used until Use this.
- CLI: status line text; `confirm` still stores; `edit --anyway`.
- Protocol round trips.

## 7. Delivery

One milestone brief after 9.8 (model roles) and the orchestrator-first alerts milestone.
Execution: subagent-driven with TDD.
