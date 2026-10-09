# Model roles: design

Date: 2026-10-07. Status: approved in conversation (sections 1–3), awaiting spec review.

## 1. Goal

The user can choose any available model for any job anthrex runs, simply and clearly, and anthrex
uses exactly that choice. The list of models and their effort levels comes from the user's
installed Claude Code and Codex, not from a list shipped with anthrex.

Success:
- Every model anthrex launches is read from one role table; nothing else picks a model.
- `C-b S` shows the table on one screen; choosing a model is one picker that lists what the user's
  CLIs report, with their own names, descriptions and effort levels.
- A new model appears in the picker the day the user's CLI offers it, without an anthrex release.
- A config written for anthrex ≤ 0.1.2 keeps choosing the same models after the upgrade.

## 2. What the user said (decisions)

| # | Decision |
|---|---|
| D1 | Models are chosen **per role**: one table of rows. |
| D2 | Each row has a model, an effort, and an optional **"if it struggles"** fallback model. Escalation raises effort, then switches to the fallback; anthrex never switches to a model the user did not choose. |
| D3 | The reviewer may be any model, but never the exact model of the work's author; if they would match, the reviewer row's fallback reviews (if none is set, the reviewer runs with a warning in the run log). Same runtime is allowed. |
| D4 | Implementers have one row per size: `small`, `medium`, `hub`. The orchestrator sizes tasks; it never picks models. |
| D5 | One `helpers` row covers the six one-shot calls, with optional per-kind overrides. |
| D6 | The table is global, with per-repository overrides of individual rows. |
| D7 | `C-b S` shows one table; `⏎` opens a picker. |
| D8 | The picker lists what the installed CLIs report (Claude Code `initialize`, Codex `model/list`), with each model's own effort levels; `custom…` types any name. Strength tiers and the enabled-models checklist are removed. |
| D9 | Old configuration keeps working: it is migrated on load and rewritten only when the user saves. |

Out of scope: runtimes other than Claude and Codex; per-run overrides beyond today's goal-form
orchestrator choice; per-task model choice by the orchestrator.

## 3. The role table

### 3.1 Rows

| Row key | Who uses it | Today |
|---|---|---|
| `orchestrator` | the run's orchestrator agent | `[orchestrator.agent]`, `routes.orchestrator`; the goal form `C-b g` still overrides per run |
| `planner` | sub-planners | `[orchestrator.planners]`, `routes.planner` |
| `implementer.small` / `implementer.medium` / `implementer.hub` | workers by task size | strength by class, `routes.s/m/hub`, plan `route` |
| `racer` | *(no row)* the second lane uses its implementer row's fallback if set, else the same model | `peer_route` |
| `test_writer` | paired test writer | `peer_route` (other runtime) |
| `reviewer` | code reviews and design-doc reviews | `routes.review`, `pick_reviewer`, doc reviewer's peer route |
| `research` | run scouts and research tasks | `[orchestrator.scouts]`, `routes.scout` |
| `brainstorm` | two models for the dual-model brainstorm | `routes.brainstorm`, strongest per runtime |
| `helpers` | the six deciders, with overrides `run_name`, `triage`, `size_check`, `check_summary`, `blocked_reason`, `ci_summary` | `[orchestrator.deciders]`, `routes.decider` |

### 3.2 A row

```rust
pub struct RoleChoice {
    pub model: ModelRef,              // "claude:<id>", "codex:<id>", "codex:default"
    pub effort: Option<String>,       // None: the model's default effort
    pub fallback: Option<ModelRef>,   // "if it struggles"
}
pub struct ModelRef { pub runtime: Runtime, pub id: Option<String> } // id None: the CLI's default
```

`brainstorm` holds two `ModelRef`s (`first`, `second`) and one effort. Effort is a string because
each model offers its own set (`low` … `xhigh`, `max`, or none); it is validated against discovery
(§4) at launch, not at load.

### 3.3 Configuration

Global, in `~/.config/anthrex/config.toml`:

```toml
[models.implementer.medium]
model = "codex:gpt-6-sol"
effort = "medium"
fallback = "claude:claude-opus-5-5"

[models.helpers]
model = "claude:claude-haiku-4-5"

[models.helpers.run_name]
model = "claude:claude-sonnet-5"
```

Per repository, the same shape in `<data_dir>/repos/<basename>-<hash8>/models.toml` (beside the
profile, never in the repository). A repository file lists only the rows it overrides; a row is
overridden whole (model, effort and fallback together).

### 3.4 Resolution

`config::models::resolve(role, repo_overrides, global) -> RoleChoice` is the only function a
launcher calls: repository row, else global row, else the built-in default (§6.1). For a helper
kind: repository kind override, global kind override, repository `helpers`, global `helpers`,
built-in.

### 3.5 Escalation

A struggling role (today's ladder rungs that change route) first raises effort one step along the
model's discovered effort list; at the top, it switches to the row's fallback at the fallback
model's default effort; with no fallback it stays and the ladder's later rungs (bounce cap,
human block) apply as today. `roster::escalate`, the peer-runtime step and strength steps are
removed.

### 3.6 The orchestrator's task routes

`plan_task.route` keeps only `size`. A `runtime`, `model`, `strength` or `effort` the orchestrator
sends is ignored with a run-log note (`route model ignored: models come from the role table`).
Contract rule 21 is rewritten to say so. Plan files with `route = { runtime = … }` are accepted
the same way (ignored, noted), so old plans still load.

## 4. Discovery

### 4.1 Probes

New module `crates/daemon/src/models/`:

- `claude_probe.rs`: spawn the configured `claude` binary with `-p --input-format stream-json
  --output-format stream-json --verbose` and the same scrubbed environment and process group as
  headless sessions; write one `control_request` with `subtype: "initialize"`; read lines until
  the matching `control_response`; take `response.models[]` (`value`, `displayName`,
  `description`, `supportsEffort`, `supportedEffortLevels`); close stdin and reap. No prompt is
  ever written.
- `codex_probe.rs`: spawn `codex app-server` (stdio JSON-RPC); send `initialize`, then
  `model/list` with `includeHidden: false`, following `nextCursor` until empty; take `id`/`model`,
  `displayName`, `description`, `supportedReasoningEfforts`, `defaultReasoningEffort`,
  `isDefault`; close and reap.
- Both run on `spawn_blocking` with a 10 s deadline, kill their own process group on deadline
  (through the existing `ProbeChild`-style guard pattern, not new kill code), and never run under a
  lock (AGENTS.md rules 2 and 3).
- Field names above are taken from the public SDK/app-server docs; the implementation records the
  real CLIs' replies as fixtures during the manual check (§8) and parses only those fields.

### 4.2 Catalog and cache

```rust
pub struct ModelCatalog {
    pub runtime: Runtime,
    pub cli_version: String,
    pub fetched_at: u64,
    pub source: CatalogSource, // Live | Cached | Builtin
    pub models: Vec<CatalogModel>,
}
pub struct CatalogModel {
    pub id: String, pub label: String, pub description: String,
    pub efforts: Vec<String>, pub default_effort: Option<String>, pub is_default: bool,
}
```

Cached at `<data_dir>/models/<runtime>-<cli_version>.json`. The daemon reuses a cache whose CLI
version matches the probed CLI version (the startup version probe for Codex; `claude --version`
through the same bounded probe for Claude). A failed probe serves the newest cache for that
runtime marked `Cached`, else a minimal built-in list (the three Claude models and "Codex
default") marked `Builtin` with a warning. `r` in the picker forces a live probe.

### 4.3 Protocol

`ClientMsg::ListModels { runtime: Option<Runtime>, refresh: bool }` →
`DaemonMsg::Models { catalogs: Vec<ModelCatalog> }`. `PROTO_VERSION` 18 → 19 (AGENTS.md rule 4):
the TUI and CLI clients are updated in the same change, with a round-trip test.

### 4.4 Launch-time validation

At launch, a role's effort not offered by its model (per the current catalog) is replaced by the
model's default effort and logged once per run (`effort 'xhigh' not offered by gpt-6-luna; using
medium`). A model missing from the catalog still launches (custom models are allowed); the TUI
marks it `⚠ not reported by codex 0.160.1`.

## 5. The `C-b S` models screen

### 5.1 Table

```
 models · scope ‹ everywhere › / this repo (anthrex)            tab: limits

 ROLE                  MODEL                   EFFORT   IF IT STRUGGLES
 orchestrator          Claude · Opus 5.5       high     —
 planner               Claude · Opus 5.5       high     —
 implementer · small   Codex  · gpt-6 luna     low      Codex · gpt-6 sol
▸implementer · medium  Codex  · gpt-6 sol      medium   Claude · Opus 5.5
 implementer · hub     Claude · Opus 5.5       high     Codex · gpt-6.1 sol
 test writer           Codex  · gpt-6 sol      medium   —
 reviewer              Codex  · gpt-6.1 sol    high     Claude · Opus 5.5
 research              Claude · Sonnet 5       medium   —
 brainstorm            Claude · Opus 5.5  +  Codex · gpt-6.1 sol
 helpers ▸             Claude · Haiku 4.5      —        —

 ⏎ choose model   e effort   f if-it-struggles   x reset   w save   esc back
```

- Labels are the CLI's `displayName`, prefixed by runtime.
- `helpers ▸` expands its six kinds inline; each shows "same as helpers" until set.
- Scope `this repo`: overridden rows marked `●`; inherited rows dimmed with `(everywhere)`;
  `x` removes the override. Scope `everywhere`: `x` resets the row to the built-in default.
- `e` cycles the row model's discovered efforts (and "default"); `—` when the model has none.
- A row whose saved effort is not offered shows `⚠ effort 'xhigh' not offered`.
- `w` saves the scope's file; the status line says `saved · new runs use these models · runs in
  progress keep theirs`.
- The previous claude/codex checklist sections are removed; the limits section remains.

### 5.2 Picker

```
 choose model · implementer · medium              r refresh · updated 2m ago

  CLAUDE  (claude 2.1.290)
    Haiku 4.5        Fastest for quick tasks              effort —
    Sonnet 5         Best for everyday tasks              low … max
    Opus 5.5         Most capable for complex work        low … max
  CODEX  (codex 0.160.1)
    gpt-6 luna       Fast, low cost                        low … xhigh
  ● gpt-6 sol        Balanced (Codex default)              low … max
    gpt-6.1 sol      Frontier                              low … max
  ─────
    custom…          type any model name

 j/k move  ⏎ select  r refresh  esc cancel
```

- `(cached)` or `(built-in list)` replaces the version when the catalog is not live.
- A runtime whose CLI is missing is listed greyed with `codex not found`.
- `custom…` asks for the runtime, then the name.
- `f` opens the same picker with a `none` entry first.
- The goal form's orchestrator picker and the task edit form use this picker (their effort cycles
  come from the catalog).

### 5.3 Purity

All of the above is state in `crates/tui/src/app/` and rendering in `crates/tui/src/ui/`; catalogs
arrive through `Effect`s and messages (AGENTS.md rule 5).

## 6. Migration

### 6.1 Built-in defaults

The built-in table reproduces today's default behaviour:

| Row | Built-in |
|---|---|
| orchestrator, planner | `claude:claude-opus-5-5`, high |
| implementer.small | `claude:claude-sonnet-5`, low |
| implementer.medium | `claude:claude-sonnet-5`, medium |
| implementer.hub | `claude:claude-opus-5-5`, high |
| test_writer | `codex:default`, medium |
| reviewer | `codex:default`, high, fallback `claude:claude-opus-5-5` |
| research | `claude:claude-sonnet-5`, medium |
| brainstorm | `claude:claude-opus-5-5` + `codex:default`, high |
| helpers | `claude:claude-haiku-4-5` |

The implementation verifies each row against today's resolution code with the default config
(the golden test in §8) and corrects this table where it differs; the table above is the intent.

### 6.2 Old keys

On load, each old key is translated into the role table in memory, after the built-ins and before
`[models]` (an explicit `[models]` row wins):

- `[[orchestrator.models]]` + `builtin_models`: the roster used to resolve strength tiers below.
- `[orchestrator.routes.<name>]`: the first candidate becomes the row's model, the second its
  fallback (`s`/`m`/`hub` → implementer rows, `review` → reviewer, `scout` → research,
  `decider` → helpers, `planner`, `orchestrator`, `brainstorm` → its first two).
- `[orchestrator.agent] runtime/model/effort` → orchestrator.
- `[orchestrator.planners|scouts|deciders] runtime/strength/effort` → planner / research /
  helpers, the strength resolved to the first roster model of that runtime at that strength.
- `default_runtime` → the runtime of implementer rows that the above left unset.

Each old key logs one warning at daemon start (`config: [orchestrator.routes.review] is replaced by
[models.reviewer]; it is migrated until you save in C-b S`). Saving in `C-b S` writes `[models]`,
removes the old keys, and keeps the previous file as `config.toml.bak`.

### 6.3 Removed

Strength (`Strength`, `strength` keys and fields), `builtin_models`, the shipped catalog
(`settings/shipped.rs`), `routes.rs`, the roster pickers (`first_at`, `lowest_at_or_above`,
`strongest_of`, `pick_reviewer`, `peer_route`, `escalate`) and their duplicates in
`scout/spec.rs`, `decider/call.rs`, `orch/launch.rs`, and `tuning.toml`'s route refit
(`escalate_above_percent`). The old keys stay parseable for migration only.

## 7. Error handling

- A probe that fails or times out never blocks a launch: launches read the role table, not the
  catalog; the catalog only validates effort and feeds the picker.
- A role whose runtime's CLI is missing fails its launch with today's error, naming the role
  (`reviewer: codex not found; choose another model in C-b S`).
- An unparseable `models.toml` in a repository is ignored with a warning; the global table
  applies.

## 8. Testing

- The fake agent answers both handshakes (`initialize` with `models[]`; `codex app-server`
  `initialize` + paged `model/list`) from fixtures; tests never reach real agents.
- Discovery: live probe parse, timeout, missing CLI, cache hit on the same version, refresh on a
  new version, `Cached` and `Builtin` fallbacks, paging.
- Resolution: repository → global → built-in for rows and helper kinds.
- Migration: a golden test runs today's resolution code and the new `resolve` over a matrix of old
  configs (default, each route list, agent/planners/scouts/deciders tables, custom roster rows,
  `builtin_models = false`) and asserts the same model for every role; it is written first, against
  the current code.
- Escalation: effort climbs the discovered list, then the fallback; no fallback stays.
- Reviewer: equal to author → fallback; no fallback → same model with a run-log warning.
- Orchestrator route: a model in `plan_task.route` is ignored and noted.
- Protocol: round-trip of `ListModels`/`Models`.
- TUI: render tests for the table (both scopes, expanded helpers, warnings) and the picker
  (live/cached/built-in, missing CLI, custom).
- End to end: a run whose rows name distinct fake models launches each role with its configured
  model (argv checked).
- Manual (with the user's approval; no prompt is sent): one real discovery per CLI on macOS and
  nexus1, recording the replies as the fixtures' source.

## 9. Delivery

One milestone brief (`docs/milestones/`), about twelve tasks, built after the Codex sandbox PR
(#49) merges. Execution: subagent-driven with TDD.
