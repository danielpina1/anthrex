//! The numbers glossary (milestone 9.19 decision 15): one line per term, in the order of
//! `docs/numbers.md`. The TUI help draws its `numbers` group from it, and a test holds the
//! two in step. Pure data: no clock, no environment, no file.

/// One glossary term.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Term {
    /// The term, as the doc's first column.
    pub term: &'static str,
    /// How it reads on screen (the doc's second column, without its backticks).
    pub shown: &'static str,
    /// The one-line definition (the doc's last column, without its `(not yet unified)` mark).
    pub definition: &'static str,
}

/// The terms, in the doc's order.
pub const GLOSSARY: &[Term] = &[
    Term {
        term: "tokens",
        shown: "tokens 1.8M",
        definition: "Billed tokens: input + cache writes + output. Cache reads excluded. A run's total covers every role, design agents included.",
    },
    Term {
        term: "cache hits",
        shown: "cache hits 71% of input",
        definition: "Cache reads ÷ (input + cache reads + cache writes). Not a share of tokens.",
    },
    Term {
        term: "context",
        shown: "context 91% full (183k / 200k)",
        definition: "The last main-chain turn's tokens, cache reads and output included (Codex: last input + output), over the session's window. Not the same number as tokens.",
    },
    Term {
        term: "window",
        shown: "200k window",
        definition: "Config above 0 wins; else Codex's rollout window, Claude `[1m]` → 1M, the learned window, else 200k.",
    },
    Term {
        term: "quota",
        shown: "5h 42% · 7d 63%",
        definition: "The runtime's own reported use of its rolling window; `stale` when older than `quota_fresh_secs`. A stale reading can keep braking: a trip made while the reading was fresh holds until its `until`, and then reads `braked`, not `stale`.",
    },
    Term {
        term: "duration",
        shown: "42s, 41m 12s, 1h 05m, 3d 04h",
        definition: "A span: run time, check time, tier time, how long an agent ran. The fixed-width cells (the sidebar row, the graph boxes, the alerts age column) and the overview's run title show the largest unit only (`41m`).",
    },
    Term {
        term: "age",
        shown: "3m 10s ago, just now",
        definition: "Now minus when it happened, on the daemon's clock. The alerts column prints `now` under 10 s and the largest unit only after that. Frozen while the daemon link is lost.",
    },
    Term {
        term: "clock time",
        shown: "14:30 +02:00, Thu 14:30 +02:00, 2026-10-12 14:30 UTC",
        definition: "Always labelled with its zone. The TUI and CLI show your local time with its offset (an offset of 0 prints `+00:00`); text the daemon writes (attention lines, prompts) is labelled `UTC`, and `REPORT.md` writes `YYYY-MM-DD HH:MM:SSZ`. A weekday within six days, a date beyond. Past times across a DST change use today's offset.",
    },
    Term {
        term: "merged",
        shown: "3/5 merged, 3/5 ✓",
        definition: "Merged tasks over tasks that can still merge: cancelled, replaced and reported tasks are in neither number. All rounds. `✓` means merged (`+` in ASCII mode).",
    },
    Term {
        term: "stage",
        shown: "stage 2 of 3",
        definition: "The task's stage over the larger of the planned stages and the recorded ones.",
    },
    Term {
        term: "round",
        shown: "round 2",
        definition: "One `run iterate` request and the tasks planned for it.",
    },
    Term {
        term: "session",
        shown: "worker #2",
        definition: "One launch of an agent on a task.",
    },
    Term {
        term: "review",
        shown: "review 2",
        definition: "The nth review verdict on a task.",
    },
    Term {
        term: "bounces",
        shown: "review 1/3",
        definition: "A gate failure sent back to the same session, over the run's `max_bounces`.",
    },
    Term {
        term: "escalation step",
        shown: "step 2",
        definition: "1 send back, 2 fresh session, 3 size raised, 4 block.",
    },
    Term {
        term: "budget",
        shown: "40 calls · 20 min · 200k tok",
        definition: "A ceiling per worker session; soft at 1×, hard at 1.5× on any axis. Spend is kept in seconds.",
    },
    Term {
        term: "est. left",
        shown: "est. left ~40m 00s",
        definition: "Time left in the current round, after approval, from history's weights. The one value rounded up.",
    },
    Term {
        term: "bound ratio",
        shown: "1.2× the bound",
        definition: "(active elapsed + est. left) ÷ the round's fastest possible time from history's weights.",
    },
    Term {
        term: "writers, readers",
        shown: "writers 2/3, readers 4/3 (over)",
        definition: "Busy over allowed; `(over)` after a limit was lowered mid-run.",
    },
    Term {
        term: "slots, load",
        shown: "slots 4 · load 0.5",
        definition: "Test slots held ÷ all slots, daemon-wide. Not the OS load average.",
    },
    Term {
        term: "lesson credit",
        shown: "credit 3/4, unmeasured",
        definition: "Of finished tasks shown the lesson, those merged and not reverted.",
    },
    Term {
        term: "alerts",
        shown: "⚑ Alerts 3",
        definition: "Every entry in the alerts box.",
    },
    Term {
        term: "window id",
        shown: "id 7",
        definition: "The id `ls`, `kill` and `attach` take. `#1` is a position in the visible list.",
    },
    Term {
        term: "run id",
        shown: "ab12",
        definition: "The last four characters of the run id; the full id only in `--json`.",
    },
    Term {
        term: "unknown",
        shown: "unknown, -",
        definition: "A value anthrex does not have. Never shown as `0`.",
    },
];
