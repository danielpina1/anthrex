//! Milestone 9.1's worker-facing texts (task M9.1.13): the tier-0 block of a tiered
//! profile's worker prompt (decision 13, Interfaces "Prompts (exact)") and the tier
//! line of a tier job's bounce. Pure, like `contract.rs`, which calls them. An untiered
//! profile gets neither, so M8a's texts are byte-identical (decision 6).

use proto::ModuleNames;

use crate::run::globs::path_module;
use crate::run::model::{CheckRecord, Run, Task};
use crate::run::tiers::Scope;
use crate::run::tiers::command::{Placeholders, filter_expr, substitute};

/// The modules task `task`'s `owns` fall in (decision 10 on each glob's literal
/// prefix), by name, sorted. Names are known here only with `module_names = "dir"`;
/// `cargo` names come from the module graph, which the engine does not hold.
fn task_modules(run: &Run, task: &Task) -> Vec<String> {
    if run.profile.tiers.module_names != ModuleNames::Dir {
        return Vec::new();
    }
    let mut names: Vec<String> = task
        .spec
        .owns
        .iter()
        .filter_map(|glob| path_module(glob, &run.profile.modules))
        .filter_map(|dir| dir.rsplit('/').next().map(str::to_string))
        .collect();
    names.sort();
    names.dedup();
    names
}

/// Decision 13's lines after `Check command:`: none for an untiered profile or one
/// with no module command.
pub(super) fn tier0_lines(run: &Run, task: &Task) -> Vec<String> {
    let tiers = &run.profile.tiers;
    if !tiers.is_tiered() {
        return Vec::new();
    }
    let names = task_modules(run, task);
    let filter = filter_expr(tiers, Scope::Gate, false);
    let command = match (&tiers.module_test, &tiers.module_tests) {
        (Some(one), _) => substitute(
            one,
            &Placeholders {
                filter,
                ..Placeholders::default()
            },
        ),
        (None, Some(many)) => substitute(
            many,
            &Placeholders {
                modules: (!names.is_empty()).then(|| names.clone()),
                filter,
                ..Placeholders::default()
            },
        ),
        (None, None) => return Vec::new(),
    };
    let mut lines = vec![format!("Module test command: {command}")];
    if !names.is_empty() {
        lines.push(format!(
            "Your modules: {} (put one in place of {{module}})",
            names.join(", ")
        ));
    }
    lines.push(
        "While you work, run your own test and these module tests only; after task_done the engine runs the wider suites."
            .to_string(),
    );
    lines
}

/// The tier line of a tier job's bounce (task M9.1.13): `\ntier <n>: <affected>`, put
/// after the bounce's first line; empty for M8a's check.
pub(super) fn tier_line(c: &CheckRecord) -> String {
    c.tier
        .as_ref()
        .map(|t| format!("\ntier {}: {}", t.tier, t.affected))
        .unwrap_or_default()
}

/// A bisect fix task's title may be at most this long (decision 37).
const FIX_TITLE_MAX: usize = 120;
/// With more failing tests than this, the acceptance names none of them (decision 37).
const FIX_ACCEPTANCE_TESTS: usize = 10;

/// What a bisect fix task's texts name (decision 37, Interfaces "Prompts (exact)").
pub(crate) struct BisectFix<'a> {
    pub id: &'a str,
    pub stage: u16,
    pub culprit: &'a str,
    pub culprit_title: &'a str,
    pub culprit_brief: &'a str,
    pub tests: &'a [String],
    pub summary: &'a str,
    pub show: &'a str,
}

/// `Fix <first failing test> after <culprit>`, with ` and <k> more` after the test when
/// there are more, cut to 120 characters.
pub(crate) fn bisect_fix_title(tests: &[String], culprit: &str) -> String {
    let first = tests.first().map_or("the failing tests", String::as_str);
    let more = match tests.len() {
        0 | 1 => String::new(),
        n => format!(" and {} more", n - 1),
    };
    format!("Fix {first}{more} after {culprit}")
        .chars()
        .take(FIX_TITLE_MAX)
        .collect()
}

/// One `<test> passes` per failing test (at most 10; with more, one item for all of
/// them), then `no test is weakened, skipped or deleted`.
pub(crate) fn bisect_fix_acceptance(tests: &[String]) -> Vec<String> {
    let mut items: Vec<String> = if tests.len() > FIX_ACCEPTANCE_TESTS {
        vec!["every failing test listed in the brief passes".to_string()]
    } else {
        tests.iter().map(|t| format!("{t} passes")).collect()
    };
    items.push("no test is weakened, skipped or deleted".to_string());
    items
}

/// A bisect fix task's brief (exact).
pub(crate) fn bisect_fix_brief(f: &BisectFix<'_>) -> String {
    format!(
        "[anthrex] Fix task {id}: the full test suite of stage {n} fails, and bisecting the stage's merges found that it started failing with the merge of task {c} (\"{title}\").\n\
         Failing tests: {tests}\n\
         Make these tests pass without weakening them. {c}'s work stays merged; fix it here.\n\
         \n\
         Summary of the failure:\n\
         {summary}\n\
         \n\
         The merge that introduced it:\n\
         {show}\n\
         \n\
         The brief of {c}:\n\
         {brief}",
        id = f.id,
        n = f.stage,
        c = f.culprit,
        title = f.culprit_title,
        tests = f.tests.join(", "),
        summary = f.summary,
        show = f.show,
        brief = f.culprit_brief,
    )
}

/// A sync fix task's title (decision 51, exact): stage `k` merged into stage `n`.
pub(crate) fn sync_fix_title(k: u16, n: u16) -> String {
    format!("Resolve the merge of stage {k} into stage {n}")
}

/// A sync fix task's acceptance (decision 51, exact).
pub(crate) fn sync_fix_acceptance() -> Vec<String> {
    vec![
        "no conflict markers remain".to_string(),
        "both stages' changes are kept".to_string(),
    ]
}

/// A sync fix task's brief (decision 51, exact), one `- <file>` line per conflicted
/// file, each made safe to show ([`shown`]): the names come from the repository.
pub(crate) fn sync_fix_brief(id: &str, k: u16, n: u16, files: &[String]) -> String {
    let files: Vec<String> = files.iter().map(|f| format!("- {}", shown(f))).collect();
    format!(
        "[anthrex] Fix task {id}: merging stage {k} into stage {n} conflicted. Your worktree already holds that merge, with conflict markers in:\n\
         {files}\n\
         Resolve every conflict so that both stages' work is kept, commit, and call task_done. Change nothing else.",
        files = files.join("\n"),
    )
}

/// At most this many characters of a path or marker taken from a worker's diff are
/// shown (task M9.1.16).
const SHOWN_CHARS_MAX: usize = 200;

/// Whether `c` could hide or reorder what is shown (task M9.1.16, ruling C-20):
/// control characters, the line and paragraph separators, the bidi overrides,
/// isolates and marks, and the zero-width characters.
fn unsafe_char(c: char) -> bool {
    c.is_control()
        || matches!(
            c,
            '\u{2028}'
                | '\u{2029}'
                | '\u{202A}'..='\u{202E}'
                | '\u{2066}'..='\u{2069}'
                | '\u{200E}'
                | '\u{200F}'
                | '\u{061C}'
                | '\u{200B}'..='\u{200D}'
                | '\u{2060}'
                | '\u{FEFF}'
        )
}

/// FNV-1a, 32-bit, over `text`'s bytes.
fn fnv1a32(text: &str) -> u32 {
    text.bytes().fold(0x811c_9dc5, |h: u32, b| {
        (h ^ u32::from(b)).wrapping_mul(0x0100_0193)
    })
}

/// A path or marker from a worker's diff, made safe to show a reviewer or the user
/// (task M9.1.16, ruling C-20): every [`unsafe_char`] becomes `?`, so the text can
/// neither start a line of its own (a forged `W<n>` line) nor hide or reorder what is
/// shown; and a text past [`SHOWN_CHARS_MAX`] is cut, then ends `…#<8 hex>`, the FNV-1a
/// of the whole text, so two long paths never look the same.
pub(crate) fn shown(text: &str) -> String {
    let safe = |c: char| if unsafe_char(c) { '?' } else { c };
    let mut out: String = text.chars().take(SHOWN_CHARS_MAX).map(safe).collect();
    if text.chars().count() > SHOWN_CHARS_MAX {
        out.push_str(&format!("…#{:08x}", fnv1a32(text)));
    }
    out
}

/// A restore command longer than this is left out (ruling C-20).
const RESTORE_COMMAND_MAX: usize = 1000;

/// `text` single-quoted for a POSIX shell.
fn sh_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

/// Decision 41's rung-1 message (exact): one `deleted test file` line per path, then
/// the restore command from `base` (the commit the diff was read from), each raw path
/// single-quoted. With a path holding a control character, or a command longer than
/// [`RESTORE_COMMAND_MAX`], the command is left out (ruling C-20).
pub(crate) fn deleted_test_file_message(paths: &[String], base: &str) -> String {
    let mut lines = vec!["[anthrex] task_done rejected:".to_string()];
    lines.extend(paths.iter().map(|p| {
        format!(
            "deleted test file {}; restore it or own it exactly",
            shown(p)
        )
    }));
    let quoted: Vec<String> = paths.iter().map(|p| sh_quote(p)).collect();
    let command = format!("git checkout {} -- {}", super::sha7(base), quoted.join(" "));
    let plain = !paths.iter().any(|p| p.chars().any(char::is_control));
    let how = if plain && command.chars().count() <= RESTORE_COMMAND_MAX {
        format!("Restore it ({command}, then commit)")
    } else {
        format!(
            "Restore the deleted test files from {}, then commit,",
            super::sha7(base)
        )
    };
    lines.push(format!(
        "{how} and call task_done again. If this task must delete it, call task_blocked with kind question and ask for the plan to be amended."
    ));
    lines.join("\n")
}

/// Decision 42's reviewer block (exact): `lines` are the `- W<n> …` lines, `more` the
/// signals past the cap. Empty with no signal.
pub(crate) fn signals_block(lines: &[String], more: u32) -> String {
    if lines.is_empty() {
        return String::new();
    }
    let mut out = vec!["Test changes to justify:".to_string()];
    out.extend(lines.iter().cloned());
    if more > 0 {
        out.push(format!("- … and {more} more (see git diff)"));
    }
    out.push(
        "Answer each with a finding whose text starts with its id: minor, as \"W1 accepted: <why the change is right>\", or critical when it weakens a test."
            .to_string(),
    );
    out.join("\n")
}

/// Decision 42: the refusal of a `submit_review` that leaves signal ids out (exact).
pub(crate) fn signals_unanswered(ids: &[String]) -> String {
    format!(
        "the review must address {}: add a finding whose text starts with each id",
        ids.join(", ")
    )
}

/// Decision 42: the engine's finding for a signal a second review still left out
/// (exact); `(<path>)` alone for a signal with no line, `(the diff)` for one with no
/// path (ruling C-20's `DiffTooLarge`).
pub(crate) fn signal_unjustified(id: &str, path: &str, line: Option<u32>) -> String {
    if path.is_empty() {
        return format!("{id} (the diff) was not justified by the review");
    }
    match line {
        Some(line) => format!(
            "{id} ({}:{line}) was not justified by the review",
            shown(path)
        ),
        None => format!("{id} ({}) was not justified by the review", shown(path)),
    }
}
