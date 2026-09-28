//! Milestone 9's report sections (decisions 35 and 36): `## Research`, every research
//! task's report, and `## Review findings`, every review task's verdict with its
//! findings of every severity; and `research.md`, the research alone, which the driver
//! writes beside the report. Pure — no `std::fs`, `std::process`, `std::thread`,
//! `tokio` or `std::time::SystemTime` (design decision 2). Every text here is an
//! agent's, so each goes through `report_escape`.

use proto::{Finding, Severity, TaskKind, Verdict};

use super::model::{Run, Task};
use super::report_escape::{escape_heading, list_item_text, plain_text_line};

/// `research.md`, in the run's data directory (decision 35).
pub const RESEARCH_FILE: &str = "research.md";

/// `## Research` and `## Review findings`, each only when a task has one.
pub(super) fn sections(run: &Run, out: &mut String) {
    if let Some(research) = research_entries(run) {
        out.push_str("\n## Research\n\n");
        out.push_str(&research);
    }
    let reviews: Vec<&Task> = run
        .tasks
        .iter()
        .filter(|t| t.spec.kind == TaskKind::Review)
        .filter(|t| t.reviews.iter().any(|r| r.verdict.is_some()))
        .collect();
    if reviews.is_empty() {
        return;
    }
    out.push_str("\n## Review findings\n\n");
    for task in reviews {
        review_entry(task, out);
    }
}

/// Decision 35: `research.md`'s text, or `None` when no research task reported.
pub fn research_text(run: &Run) -> Option<String> {
    let entries = research_entries(run)?;
    Some(format!("# Research of run {}\n\n{entries}", run.id))
}

fn research_entries(run: &Run) -> Option<String> {
    let mut out = String::new();
    for task in &run.tasks {
        let Some(report) = &task.orch.research else {
            continue;
        };
        out.push_str(&format!(
            "### {}: {}\n\n",
            task.id(),
            escape_heading(&task.spec.title)
        ));
        out.push_str(&plain_text_line(&report.summary));
        out.push_str("\n\n");
        if !report.files.is_empty() {
            out.push_str("Files:\n");
            for file in &report.files {
                let path = file.path.replace(['\n', '\r'], " ");
                let prefix = format!("{path}: ");
                out.push_str(&format!("- {}\n", list_item_text(&prefix, &file.why)));
            }
            out.push('\n');
        }
        for (label, items) in [
            ("Modules", &report.modules),
            ("Interfaces", &report.interfaces),
            ("Risks", &report.risks),
        ] {
            if items.is_empty() {
                continue;
            }
            out.push_str(&format!("{label}:\n"));
            for item in items {
                out.push_str(&format!("- {}\n", list_item_text("", item)));
            }
            out.push('\n');
        }
    }
    (!out.is_empty()).then_some(out)
}

fn review_entry(task: &Task, out: &mut String) {
    let target = task
        .spec
        .review_target
        .as_deref()
        .unwrap_or("")
        .replace(['\n', '\r'], " ");
    out.push_str(&format!(
        "### {}: {} ({target})\n\n",
        task.id(),
        escape_heading(&task.spec.title)
    ));
    for review in task.reviews.iter().filter(|r| r.verdict.is_some()) {
        let verdict = match review.verdict {
            Some(Verdict::Approve) => "approve",
            _ => "changes",
        };
        out.push_str(&format!(
            "Verdict: {verdict}. {}\n",
            plain_text_line(&review.summary)
        ));
        if review.findings.is_empty() {
            out.push('\n');
            continue;
        }
        out.push('\n');
        for finding in &review.findings {
            out.push_str(&format!("- {}\n", finding_line(finding)));
        }
        out.push('\n');
    }
}

/// `critical src/a.rs:3: text`, `important: text`, or with `input <input>`.
fn finding_line(finding: &Finding) -> String {
    let severity = match finding.severity {
        Severity::Critical => "critical",
        Severity::Important => "important",
        Severity::Minor => "minor",
    };
    let one = |s: &str| s.replace(['\n', '\r'], " ");
    let at = match (&finding.file, finding.line, &finding.input) {
        (Some(file), Some(line), _) => format!(" {}:{line}", one(file)),
        (Some(file), None, _) => format!(" {}", one(file)),
        (None, _, Some(input)) => format!(" input {}", one(input)),
        _ => String::new(),
    };
    list_item_text(&format!("{severity}{at}: "), &finding.text)
}
