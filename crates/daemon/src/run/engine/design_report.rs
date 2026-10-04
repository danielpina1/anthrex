//! Milestone 9.6 task M9.6.9, part of `design.rs`: the merged brainstorm report (DF
//! §3.4, §3.5). It is taken only once the brainstorm's drafts are in ([`drafts_not_in`],
//! ruling T8-5's `drafts_settled`, and each draft's text loaded or found unreadable,
//! ruling T9-1a); the orchestrator is told its template with the
//! drafts-in wake ([`template_note`]); and the version stored is the report with the
//! drafts attached as its appendix (decision 13, [`report_doc`]), whose length and
//! SHA-256 are the written file's. Pure (design decision 2).

use proto::{DocAuthor, DocKind};

use super::super::requests::log;
use super::checked_text;
use crate::run::design::report::{self, attach};
use crate::run::design::state::{DesignAgentState, NewDoc};
use crate::run::design::template::tags;
use crate::run::model::Run;

/// The refusal of a report submitted before the drafts are in.
pub const NOT_IN: &str = "the brainstorm drafts are not in yet";

/// A raw report this long or shorter may carry the engine's appendix back (the
/// report's cap, both drafts' caps, and room for the labels and the demoted headings);
/// a longer one is left whole for the cap to refuse it unread.
const WITH_APPENDIX_MAX: usize = 32 * 1024 + 2 * 12 * 1024 + 8 * 1024;

/// The carry from task 7's review and ruling T9-1a: the merged report waits for the
/// drafts, both in, or one in and the other brainstormer's failure recorded
/// (`drafts_settled`), and for each settled draft's text, loaded or marked unreadable
/// after a restore ([`drafts`]'s lookup).
fn drafts_not_in(run: &Run) -> Option<String> {
    let settled = run.orch.design.as_ref().is_some_and(|d| d.drafts_settled);
    (!settled || drafts(run).is_none()).then(|| NOT_IN.to_string())
}

/// The carry from task 4's review: the report's template, sent with the drafts-in wake
/// (rule 49's text is M9.6.14's): its sections, each approach a `### <name> [<tag>]`
/// heading under `## Approaches` with the run's tags, and, with one brainstormer failed,
/// the line the report must begin with (DF §3.5).
pub(in crate::run::engine) fn template_note(run: &Run, failed: Option<(&str, &str)>) -> String {
    let labels: Vec<String> = (run.orch.design.iter())
        .flat_map(|d| &d.brainstormers)
        .map(|a| a.label.clone())
        .collect();
    let tags = tags(&labels);
    let first = labels.first().map_or("both", String::as_str);
    let named = match tags.split_last() {
        Some((last, rest)) if !rest.is_empty() => format!("{} or {last}", rest.join(", ")),
        Some((last, _)) => last.clone(),
        None => String::new(),
    };
    let mut note = format!(
        "the merged report's template: ## Where they agree; ## Where they disagree (each \
         side, then your judgment); ## Approaches, each approach a \"### <name> [{first}]\" \
         heading tagged {named}; ## Recommendation, naming one listed approach; ## \
         Questions for you"
    );
    if let Some((label, reason)) = failed {
        let line = format!("single brainstorm: {label} failed: {reason}");
        note.push_str(&format!("; begin with the line \"{line}\""));
    }
    note
}

/// The orchestrator's report, or the user's edit of it at the gate, as the version to
/// store; the one place both are refused until the drafts are in (ruling T9-1a). The
/// engine's appendix is cut from `raw` if it carries one back (a report read with
/// `get_doc` is the whole file), the rest checked against the template, then the drafts
/// attached anew (decision 13), with the summary the gate shows (DF §6.1). Also
/// returns whether the cut part was not the appendix the engine would attach: an
/// appendix heading of the report's own ([`warn_cut`]).
pub(in crate::run::engine) fn report_doc(
    run: &Run,
    raw: &str,
    author: DocAuthor,
    reason: &str,
) -> Result<(NewDoc, bool), String> {
    if let Some(text) = drafts_not_in(run) {
        return Err(text);
    }
    let drafts = drafts(run).unwrap_or_default();
    let (raw, cut) = match raw.len() <= WITH_APPENDIX_MAX {
        true => report::split(raw),
        false => (raw, None),
    };
    let text = checked_text(run, DocKind::Brainstorm, raw, false, true)?;
    let labels: Vec<String> = (run.orch.design.iter())
        .flat_map(|d| &d.brainstormers)
        .map(|a| a.label.clone())
        .collect();
    let summary = report::summary(&text, &labels);
    let file = attach(&text, &drafts);
    let own =
        cut.is_some_and(|cut| Some(cut.trim_end()) != report::split(&file).1.map(str::trim_end));
    let mut doc = NewDoc::new(DocKind::Brainstorm, author, reason, &file);
    doc.report = Some(summary);
    Ok((doc, own))
}

/// Review minor 5: a report whose own `## Appendix: the drafts` heading cut it is said
/// so in the run's log, since what followed the heading is not stored.
pub(in crate::run::engine) fn warn_cut(run: &mut Run, own: bool, now: u64) {
    if own {
        log(run, now, CUT_WARNING);
    }
}

/// The run's log line when a report's own appendix heading cut it.
pub const CUT_WARNING: &str = "design flow: warning: the brainstorm report had its own \"## Appendix: the drafts\" section; it was cut, and anthrex attached the drafts";

/// The appendix's drafts, in the brainstormers' order: a failed one with its reason;
/// otherwise its latest stored draft (this round's, since the brainstorm settled), from
/// the texts the engine keeps, or, when a restore could not read it back, unread with
/// why (ruling T9-1a). `None` while a draft is neither loaded nor marked unreadable (a
/// restore's read-back still under way).
fn drafts(run: &Run) -> Option<Vec<(String, Result<String, String>)>> {
    let design = run.orch.design.as_ref()?;
    (design.brainstormers.iter())
        .map(|a| {
            let draft = match &a.state {
                DesignAgentState::Failed(reason) => Err(reason.clone()),
                _ => {
                    let n = design.draft_from(&a.label)?.n;
                    match (design.draft_text(n), design.unread_reason(n)) {
                        (Some(text), _) => Ok(text.to_string()),
                        (None, Some(why)) => Err(format!("{UNREAD}: {why}")),
                        (None, None) => return None,
                    }
                }
            };
            Some((a.label.clone(), draft))
        })
        .collect()
}

/// How the appendix names a draft a restore could not read back.
pub const UNREAD: &str = "its draft could not be read back";
