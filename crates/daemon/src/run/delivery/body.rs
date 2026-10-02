//! Decisions 20 and 21: a stage PR's title and body, exactly as Interfaces "The PR
//! body" gives them. Pure.
//!
//! Every agent-written string (titles, reasons, the goal, test names, paths) is
//! untrusted here: it is put on one line, then either backslash-escaped where GFM
//! would read markup (so `|` cannot add a table column, `<!--` cannot hide the rest of
//! the body, `*`, `_`, `[`, `` ` `` and `<` cannot format or link) or set in a code
//! span longer than any backtick run in it; and `@` becomes `＠`, so no text can
//! mention, and notify, anyone. The one live `#<n>` is the stack's own link.

use proto::TaskState;
use proto::safe_text::one_line;
use unicode_segmentation::UnicodeSegmentation;

use super::snapshot::stage_count;
use super::{BODY_MAX_CHARS, PrRecord};
use crate::run::contract::{mode_label, size_label};
use crate::run::globs::{ProtectedMatcher, names_literally};
use crate::run::model::{Run, Task, TierRecord};
use crate::run::report::verdict_label;

/// GitHub's own cap on a pull request's title, which `allow::check` also enforces.
pub const TITLE_MAX_CHARS: usize = 256;

const NONE_LINE: &str = "(none: nothing here needs a closer look than the rest)";
const TABLE_HEAD: &str = "| Task | Title | Size | Test mode | Named test | Review | Rung |\n|------|-------|------|-----------|------------|--------|------|\n";

/// Decision 20: `[anthrex r<h4> <n>/<N>] <stage title>`, the stage title being its
/// first task's title, with ` (+<k> more)` when it has more tasks; on one line with no
/// control or bidi character (task 4's ruling: `allow::check` refuses a title with a
/// line break as `Forbidden`), cut to [`TITLE_MAX_CHARS`].
pub fn pr_title(run: &Run, stage: u16) -> String {
    let title = format!(
        "[anthrex r{} {stage}/{}] {}",
        run.short(),
        stage_count(run),
        stage_title(&stage_tasks(run, stage), str::to_string)
    );
    cut_title(&one_line(&title))
}

/// At most [`TITLE_MAX_CHARS`] characters (as `allow::check` counts them), cut between
/// grapheme clusters (fix round 1, m2), so a flag, a skin-toned emoji or a letter with
/// its combining mark is kept whole or dropped whole.
fn cut_title(title: &str) -> String {
    let mut out = String::new();
    let mut count = 0;
    for cluster in title.graphemes(true) {
        count += cluster.chars().count();
        if count > TITLE_MAX_CHARS {
            break;
        }
        out.push_str(cluster);
    }
    out
}

/// The stage's nearest lower stage whose PR is open, which stage `stage`'s PR is
/// based on (decision 20); `None`: the base branch.
pub fn stacked_on(run: &Run, stage: u16) -> Option<(u16, &PrRecord)> {
    (1..stage).rev().find_map(|m| {
        run.delivery
            .pr(m)
            .filter(|p| p.state == proto::PrState::Open)
            .map(|p| (m, p))
    })
}

/// Decision 21: the PR body, at most [`BODY_MAX_CHARS`] characters.
pub fn pr_body(run: &Run, stage: u16) -> String {
    let tasks = stage_tasks(run, stage);
    let stack = match stacked_on(run, stage) {
        Some((m, pr)) => format!("based on stage {m}, #{}", pr.number),
        None => format!("based on the base branch {}", md(&run.base_branch)),
    };
    let goal: String = one_line(&run.goal).chars().take(500).collect();
    let head = format!(
        "<!-- anthrex:pr {} stage {stage} -->\n**Goal:** {}\n\n**Stage {stage} of {}:** {}\n**Stack:** {stack}\n\n### Look here first\n{}\n### Tasks\n{TABLE_HEAD}",
        run.id,
        md(&goal),
        stage_count(run),
        stage_title(&tasks, md),
        look_here(run, &tasks),
    );
    let rows: Vec<String> = tasks.iter().copied().map(row).collect();
    let tail = format!("\n{}\n{}", evidence(run, stage, &tasks), footer(run));
    fit(&head, &rows, &tail)
}

/// The tasks of stage `stage` its PR carries: every one not cancelled, in plan order.
fn stage_tasks(run: &Run, stage: u16) -> Vec<&Task> {
    run.tasks
        .iter()
        .filter(|t| t.stage() == stage && t.state != TaskState::Cancelled)
        .collect()
}

fn stage_title(tasks: &[&Task], text: impl Fn(&str) -> String) -> String {
    let first = tasks.first().map_or(String::new(), |t| text(&t.spec.title));
    match tasks.len() {
        0 | 1 => first,
        k => format!("{first} (+{} more)", k - 1),
    }
}

/// Decision 21's "Look here first", one line per reason, ranked by reason (hub,
/// atomic, merged without approval, rung, signals), each in plan order; then the
/// protected files.
fn look_here(run: &Run, tasks: &[&Task]) -> String {
    let mut out = String::new();
    for rank in 0..REASONS {
        for t in tasks {
            if let Some(text) = reason(rank, t) {
                out.push_str(&format!(
                    "- {} ({}): {text}\n",
                    md(t.id()),
                    md(&t.spec.title)
                ));
            }
        }
    }
    let (paths, invalid) = protected(run, tasks);
    for path in paths {
        out.push_str(&format!("- {}: a protected file\n", code(&path, false)));
    }
    if invalid {
        out.push_str("- protected files: invalid pattern\n");
    }
    if out.is_empty() {
        out = format!("{NONE_LINE}\n");
    }
    out
}

/// The task reasons of "Look here first", in rank order.
const REASONS: usize = 5;

fn reason(rank: usize, t: &Task) -> Option<String> {
    match rank {
        0 => t.hub.then(|| "hub task, runs alone".to_string()),
        1 => t.spec.atomic.then(|| match &t.spec.atomic_reason {
            Some(reason) => format!("atomic task ({})", md(reason)),
            None => "atomic task".to_string(),
        }),
        2 => t
            .merged_without_approval
            .as_ref()
            .map(|reason| format!("merged without approval: {}", md(reason))),
        3 => (t.max_rung >= 2).then(|| format!("escalated to rung {}", t.max_rung)),
        _ => {
            let k = u32::try_from(t.signals.len())
                .unwrap_or(u32::MAX)
                .saturating_add(t.signals_more);
            (k > 0).then(|| format!("test changes to justify: {}", plural(k, "signal")))
        }
    }
}

/// The protected paths the stage's tasks may have changed. A task changes a protected
/// file only when its `owns` names it exactly (M8a decision 56), so these are the
/// paths a task of the stage names exactly that `profile.protected` matches or that
/// were protected at the run's base. (The stage's changed paths are not in the model,
/// which is pure; this is what it can know.) The `bool` is true when
/// `profile.protected` does not compile (fix round 1, m5): the body then says so
/// rather than silently listing less.
fn protected(run: &Run, tasks: &[&Task]) -> (Vec<String>, bool) {
    let matcher = ProtectedMatcher::new(&run.profile.protected).ok();
    let mut paths: Vec<String> = Vec::new();
    for t in tasks {
        let literal = t
            .spec
            .owns
            .iter()
            .filter(|e| !e.contains(['*', '?', '[', ']', '{', '}']));
        for path in literal.chain(&run.protected_files) {
            let guarded = run.protected_files.contains(path)
                || matcher.as_ref().is_some_and(|m| m.matches(path));
            if guarded && names_literally(&t.spec.owns, path) && !paths.contains(path) {
                paths.push(path.clone());
            }
        }
    }
    paths.sort();
    (paths, matcher.is_none())
}

/// One table row: id, title, size, test mode, named test, review verdict and rounds,
/// highest rung.
fn row(t: &Task) -> String {
    let test = t
        .proofs
        .last()
        .map(|p| p.test.as_str())
        .or(t.spec.test_to_write.as_deref())
        .filter(|s| !s.is_empty())
        .map_or("–".to_string(), |s| code(s, true));
    let verdict = if t.merged_without_approval.is_some() {
        "override"
    } else {
        verdict_label(t.reviews.last().and_then(|r| r.verdict))
    };
    let rounds = u32::try_from(t.reviews.len()).unwrap_or(u32::MAX);
    format!(
        "| {} | {} | {} | {} | {test} | {verdict}, {} | {} |\n",
        md(t.id()),
        md(&t.spec.title),
        size_label(t.size),
        mode_label(t.test_mode),
        plural(rounds, "round"),
        t.max_rung
    )
}

fn more_row(k: usize) -> String {
    format!("| … and {k} more tasks | | | | | | |\n")
}

/// Decision 21: the whole body when it fits; else the table's rows are dropped from
/// the end, with one `… and <k> more tasks` row, until it does. Only when no row is
/// left and it still does not fit (titles of thousands of characters each) is the
/// text before the footer cut, so the footer always stays.
fn fit(head: &str, rows: &[String], tail: &str) -> String {
    let len = |s: &str| s.chars().count();
    let fixed = len(head) + len(tail);
    let sizes: Vec<usize> = rows.iter().map(|r| len(r)).collect();
    let all: usize = sizes.iter().sum();
    let kept = if fixed + all <= BODY_MAX_CHARS {
        rows.len()
    } else {
        let mut used = 0;
        let mut kept = 0;
        for (i, size) in sizes.iter().enumerate() {
            let more = len(&more_row(rows.len() - i - 1));
            if fixed + used + size + more > BODY_MAX_CHARS {
                break;
            }
            used += size;
            kept = i + 1;
        }
        kept
    };
    let mut body = head.to_string();
    for r in &rows[..kept] {
        body.push_str(r);
    }
    if kept < rows.len() {
        body.push_str(&more_row(rows.len() - kept));
    }
    body.push_str(tail);
    if len(&body) <= BODY_MAX_CHARS {
        return body;
    }
    let footer = footer_of(tail);
    let room = BODY_MAX_CHARS - len(footer) - 2;
    let before: String = body.chars().take(room).collect();
    format!("{before}\n\n{footer}")
}

fn footer_of(tail: &str) -> &str {
    tail.rfind("\n---\n").map_or(tail, |at| &tail[at + 1..])
}

/// Decision 21's test evidence, from 9.1's records: the tier jobs the stage's tasks
/// ran (tiers 1 and 2) and the stage's counted tier-3 runs (`StageFull.runs`, fix
/// round 1), tier 3 on the stage's head, and every flaky test seen.
fn evidence(run: &Run, stage: u16, tasks: &[&Task]) -> String {
    let records = || {
        tasks
            .iter()
            .flat_map(|t| t.checks.iter().filter_map(|c| c.tier.as_ref()))
    };
    let count = |tier: u8| records().filter(|r| r.tier == tier).count();
    let full = run.stage(stage).map(|s| &s.full);
    let last = full.and_then(|f| f.last.as_ref());
    let head = run.stage_head(stage);
    let on_head = last.filter(|l| Some(l.commit.as_str()) == head && !l.commit.is_empty());
    let tier3 = match on_head {
        Some(l) => {
            let shards = run.profile.tiers.full_shards;
            format!(
                "{} in {} min {} s{}",
                if l.ok { "green" } else { "red" },
                l.secs / 60,
                l.secs % 60,
                if shards > 1 {
                    format!(", {shards} shards")
                } else {
                    String::new()
                }
            )
        }
        None if run.profile.check.is_none() => {
            "not run (the profile has no check, so the run is unverified)".to_string()
        }
        None => "none".to_string(),
    };
    let mut flaky: Vec<&str> = Vec::new();
    for name in records()
        .chain(last)
        .flat_map(|r| r.flaky.iter().map(String::as_str))
    {
        if !flaky.contains(&name) {
            flaky.push(name);
        }
    }
    let flaky = if flaky.is_empty() {
        "none".to_string()
    } else {
        flaky
            .iter()
            .map(|n| code(n, false))
            .collect::<Vec<_>>()
            .join(", ")
    };
    format!(
        "### Test evidence\n- Tiers run: tier 1 ×{}, tier 2 ×{}, tier 3 ×{}\n- Tier 3 on this head: {tier3}\n- Flaky tests seen: {flaky}\n",
        count(1),
        count(2),
        tier3_runs(full, last),
    )
}

/// Fix round 1: the stage's counted tier-3 runs; a stage from 9.1, which counted none
/// but kept its last record, shows that one.
fn tier3_runs(full: Option<&crate::run::model::StageFull>, last: Option<&TierRecord>) -> u32 {
    full.map_or(0, |f| f.runs).max(u32::from(last.is_some()))
}

fn footer(run: &Run) -> String {
    format!(
        "---\nOpened by anthrex for run {}. anthrex pushes fixes to this branch when CI fails or a reviewer with write access comments, and replies on the threads it addressed. **anthrex never merges this pull request**, never approves it and never resolves a thread: that is yours. For a stack, merge with a merge commit; a squash or rebase merge also works, and anthrex then merges the new base into the next stage before retargeting it.\n",
        md(&run.id)
    )
}

fn plural(n: u32, word: &str) -> String {
    if n == 1 {
        format!("1 {word}")
    } else {
        format!("{n} {word}s")
    }
}

/// Untrusted text as inline Markdown: one line, every character GFM reads as markup
/// mid-line backslash-escaped (CommonMark renders an escaped ASCII punctuation
/// character as itself; `$` too, fix round 1's m3, so no `$…$` renders as math), and
/// `@` made `＠` so it mentions no one. Bare URLs and `owner/repo#n` cross-references
/// still autolink (accepted, fix round 1).
pub(crate) fn md(text: &str) -> String {
    let mut out = String::new();
    for c in one_line(text).chars() {
        match c {
            '\\' | '`' | '*' | '_' | '[' | ']' | '<' | '>' | '&' | '~' | '|' | '#' | '$' => {
                out.push('\\');
                out.push(c);
            }
            '@' => out.push('＠'),
            c => out.push(c),
        }
    }
    out
}

/// Untrusted text as an inline code span: one line, between backtick runs one longer
/// than any in it (padded with a space where it starts or ends with a backtick or a
/// space). In a table cell `|` is escaped too, which GFM requires even in code.
pub(crate) fn code(text: &str, in_table: bool) -> String {
    let text = one_line(text);
    let mut longest = 0;
    let mut run = 0;
    for c in text.chars() {
        run = if c == '`' { run + 1 } else { 0 };
        longest = longest.max(run);
    }
    let fence = "`".repeat(longest + 1);
    let edge = |c: Option<char>| matches!(c, Some('`' | ' '));
    let inner = if edge(text.chars().next()) || edge(text.chars().last()) {
        format!(" {text} ")
    } else {
        text
    };
    let inner = if in_table {
        inner.replace('|', "\\|")
    } else {
        inner
    };
    format!("{fence}{inner}{fence}")
}
