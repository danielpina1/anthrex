# Reading the numbers

Every number anthrex shows has one meaning, one label and one format, in the TUI, the CLI and the text the daemon writes. This page is the list; `?` help shows the same terms, and a number that disagrees with it is a bug.

## Formats

- **Tokens** are billed tokens, floored: `999`, `4.5k`, `45k`, `1.2M`.
- **Durations** have two units, the second zero-padded, floored: `42s`, `41m 12s`, `1h 05m`, `3d 04h`. Fixed-width cells (the sidebar row, the graph boxes, the alerts age column) and the overview's run title show the largest unit only: `42s`, `41m`, `1h`, `3d`.
- **Ages** read `just now` under 10 s, else `<duration> ago`: `read 3m 10s ago`, `verified 2h 05m ago`.
- **Clock times** carry their zone. Clients show your local time with its offset: `14:30 +02:00` today, `Thu 14:30 +02:00` within six days, `2026-10-12 14:30 +02:00` beyond. Text the daemon writes says `UTC`: `14:30 UTC`.
- **Percentages** are floored and never pass 100 silently: `context 91% full (183k / 200k)`, `context over 100% (450k / 200k window)`, `context 183k (window unknown)`.
- **A missing value** reads `unknown`, or `-` in a table cell. It is never shown as `0`.

## Glossary

| Term | Shown as | Unit, format, rounding | Definition |
|---|---|---|---|
| tokens | `tokens 1.8M` | `999`, `4.5k`, `45k`, `1.2M`; floor | Billed tokens: input + cache writes + output. Cache reads excluded. A run's total covers every role, design agents included. |
| cache hits | `cache hits 71% of input` | %; floor | Cache reads ÷ (input + cache reads + cache writes). Not a share of tokens. |
| context | `context 91% full (183k / 200k)` | % of the window; floor; `over 100%` past it | The last main-chain turn's tokens, cache reads and output included (Codex: last input + output), over the session's window. Not the same number as tokens. |
| window | `200k window` | tokens | Config above 0 wins; else Codex's rollout window, Claude `[1m]` → 1M, the learned window, else 200k. |
| quota | `5h 42% · 7d 63%` | %; floor | The runtime's own reported use of its rolling window; `stale` when older than `quota_fresh_secs`. A stale reading can keep braking: a trip made while the reading was fresh holds until its `until`, and then reads `braked`, not `stale`. |
| duration | `42s`, `41m 12s`, `1h 05m`, `3d 04h` | two units; floor | A span: run time, check time, tier time, how long an agent ran. The fixed-width cells (the sidebar row, the graph boxes, the alerts age column) and the overview's run title show the largest unit only (`41m`). |
| age | `3m 10s ago`, `just now` | a duration; floor | Now minus when it happened, on the daemon's clock. The alerts column prints `now` under 10 s and the largest unit only after that. Frozen while the daemon link is lost. |
| clock time | `14:30 +02:00`, `Thu 14:30 +02:00`, `2026-10-12 14:30 UTC` | local offset, or UTC | Always labelled with its zone. The TUI and CLI show your local time with its offset (an offset of 0 prints `+00:00`); text the daemon writes (attention lines, prompts) is labelled `UTC`, and `REPORT.md` writes `YYYY-MM-DD HH:MM:SSZ`. A weekday within six days, a date beyond; never a date alone (a lesson's `last seen` is a clock time). Past times across a DST change use today's offset. |
| merged | `3/5 merged`, `3/5 ✓` | tasks | Merged tasks over tasks that can still merge: cancelled, replaced and reported tasks are in neither number. All rounds. `✓` means merged (`+` in ASCII mode). |
| stage | `stage 2 of 3` | stages | The task's stage over the larger of the planned stages and the recorded ones. |
| round | `round 2` | goal rounds | One `run iterate` request and the tasks planned for it. |
| session | `worker #2` | count | One launch of an agent on a task. (not yet unified) |
| review | `review 2` | count | The nth review verdict on a task. (not yet unified) |
| bounces | `review 1/3` | per gate | A gate failure sent back to the same session, over the run's `max_bounces`. |
| escalation step | `step 2` | 1–4 | 1 send back, 2 fresh session, 3 size raised, 4 block. |
| budget | `40 calls · 20 min · 200k tok` | per session | A ceiling per worker session; soft at 1×, hard at 1.5× on any axis. Spend is kept in seconds. |
| est. left | `est. left ~40m 00s` | duration; ceil | Time left in the current round, after approval, from history's weights. The one value rounded up. |
| bound ratio | `1.2× the bound` | ×; floor | (active elapsed + est. left) ÷ the round's fastest possible time from history's weights. |
| writers, readers | `writers 2/3`, `readers 4/3 (over)` | slots | Busy over allowed; `(over)` after a limit was lowered mid-run. |
| slots, load | `slots 4 · load 0.5` | slots; one decimal, half up | Test slots held ÷ all slots, daemon-wide. Not the OS load average. |
| lesson credit | `credit 3/4`, `unmeasured` | tasks | Of finished tasks shown the lesson, those merged and not reverted. (not yet unified) |
| alerts | `⚑ Alerts 3` | entries | Every entry in the alerts box. (not yet unified) |
| window id | `id 7` | — | The id `ls`, `kill` and `attach` take. `#1` is a position in the visible list. |
| run id | `ab12` | — | The last four characters of the run id; the full id only in `--json`. (not yet unified) |
| unknown | `unknown`, `-` | — | A value anthrex does not have. Never shown as `0`. |
