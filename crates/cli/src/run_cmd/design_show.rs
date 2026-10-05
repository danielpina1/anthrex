//! `run show`'s text (task M9.6.16, DF §7): the version on stdout, its header and its
//! gate's review on stderr. Part of `run_cmd::design`. Pure.

use proto::{DocGateKind, DocKind, DocView, RunInfo};

use super::GateArg;
use crate::run_cmd::status::printable;

/// `run show`'s two texts. stdout: the version's text exactly as stored (so it can be
/// edited and sent back with `edit-doc`), then with `--diff` the line diff against the
/// previous version and with `--findings` its review's findings, each with the
/// orchestrator's answer (a review draft's: that review's, or `no findings yet`);
/// through `printable` only when stdout is a `terminal` (ruling T16-3: piped or
/// redirected, the bytes as stored). stderr, always through
/// `printable`: the header (`<Kind> · run <id> · v<n> of <m> · <reason>`, or a review
/// draft's `<Kind> · run <id> · draft r<k>`), and when the version is the one waiting
/// at its gate, the gate's change summary, why it went unreviewed, its disputed
/// findings and, at the brainstorm gate, the merged report's counts and tags.
pub(in crate::run_cmd) fn show_text(
    info: &RunInfo,
    doc: &DocView,
    (diff, findings): (bool, bool),
    terminal: bool,
) -> (String, String) {
    // Ruling T16-2 (m4): the stored text exactly, so a show and edit-doc round trip
    // sends the same bytes back; a section asked for after it starts on its own line.
    let mut out = doc.text.clone();
    if (diff || findings) && !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    if diff {
        match &doc.diff {
            Some(text) => {
                // WB-C M-4: a review draft's diff is against the draft before it.
                let against = match doc.draft_review {
                    Some(_) => "the previous review draft".to_string(),
                    None => format!("v{}", doc.version.saturating_sub(1)),
                };
                out.push_str(&format!("=== diff against {against} ===\n{text}"));
                if !text.ends_with('\n') {
                    out.push('\n');
                }
            }
            None => out.push_str("=== no earlier version to diff against ===\n"),
        }
    }
    if findings {
        // The W3 carry: a review draft's are its review's, none until it gives some.
        match doc.draft_review.is_some() && doc.findings.is_empty() {
            true => out.push_str("=== no findings yet ===\n"),
            false => out.push_str(&findings_text(&doc.findings)),
        }
    }
    let out = if terminal { printable(&out) } else { out };
    (out, printable(&header(info, doc)))
}

fn findings_text(findings: &[(proto::DocFinding, Option<String>)]) -> String {
    if findings.is_empty() {
        return "=== findings: none ===\n".to_string();
    }
    let mut out = format!("=== findings: {} ===\n", findings.len());
    for (f, answer) in findings {
        let severity = match f.severity {
            proto::DocSeverity::Blocking => "blocking",
            proto::DocSeverity::Minor => "minor",
        };
        let place = match f.place.is_empty() {
            true => String::new(),
            false => format!(" at {}", f.place),
        };
        out.push_str(&format!("{} {severity}{place}: {}\n", f.id, f.text));
        match answer {
            Some(answer) => out.push_str(&format!("  answer: {answer}\n")),
            None => out.push_str("  no answer\n"),
        }
    }
    out
}

/// `run show`'s stderr ([`show_text`]).
fn header(info: &RunInfo, doc: &DocView) -> String {
    let kind = match doc.kind {
        DocKind::BrainstormDraft => "Brainstorm draft",
        DocKind::Brainstorm => "Brainstorm",
        DocKind::Spec => "Spec",
        DocKind::Plan => "Plan",
    };
    if let Some(k) = doc.draft_review {
        return format!("{kind} · run {} · draft r{k}\n", info.run_id);
    }
    let of = (info.docs.iter())
        .filter(|d| d.kind == doc.kind)
        .map(|d| d.version)
        .fold(doc.version, u32::max);
    let mut out = format!("{kind} · run {} · v{} of {of}", info.run_id, doc.version);
    let stored = (info.docs.iter()).find(|d| d.kind == doc.kind && d.version == doc.version);
    if let Some(reason) = stored.map(|d| d.reason.as_str()).filter(|r| !r.is_empty()) {
        out.push_str(&format!(" · {reason}"));
    }
    out.push('\n');
    let gate = info
        .doc_gate
        .as_ref()
        .filter(|g| DocKind::from(gate_arg(g.kind)) == doc.kind && g.version == doc.version);
    let Some(gate) = gate else {
        return out;
    };
    if !gate.changes_summary.is_empty() {
        out.push_str(&format!("changes: {}\n", gate.changes_summary.join("; ")));
    }
    if let Some(why) = &gate.not_reviewed {
        out.push_str(&format!("not reviewed: {why}\n"));
    }
    if !gate.disputed.is_empty() {
        let ids: Vec<&str> = gate.disputed.iter().map(|f| f.id.as_str()).collect();
        out.push_str(&format!("disputed: {}\n", ids.join(", ")));
    }
    if let Some(report) = &gate.report {
        let tags: Vec<String> = (report.approaches.iter())
            .map(|a| format!("{} [{}]", a.name, a.tag))
            .collect();
        out.push_str(&format!(
            "report: {} agree, {} disagree; approaches: {}\n",
            report.agree,
            report.disagree,
            tags.join(", ")
        ));
    }
    out
}

fn gate_arg(kind: DocGateKind) -> GateArg {
    match kind {
        DocGateKind::Brainstorm => GateArg::Brainstorm,
        DocGateKind::Spec => GateArg::Spec,
        DocGateKind::Plan => GateArg::Plan,
    }
}
