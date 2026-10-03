//! Milestone 9.5 decision 11: the tuning block `anthrex run stats` prints after M8b's
//! text (`stats::render`), in the brief's exact layout (Interfaces, CLI). The TUI's
//! stats screen draws the same lines (decision 48). Pure.

use proto::{ClassTuning, RefitState, TuningReport};

use super::refit::budget_text;
use super::report::format_utc;

/// The block for `report`, every line ending in a newline.
pub fn render(report: &TuningReport) -> String {
    let mut out = String::new();
    if let Some(moved) = &report.moved_bad_file {
        // The parse error is not in `TuningReport` (implementation notes, M9.5.8).
        out.push_str(&format!(
            "tuning: tuning.toml did not parse; it was moved to {} and tuning starts again from history\n",
            moved.display()
        ));
    }
    let header = if report.refit_budgets {
        format!("refit after {} samples per class", report.min_samples)
    } else {
        "budget refit off: [orchestrator.tuning] refit_budgets = false".to_string()
    };
    out.push_str(&format!("tuning: {}  ({header})\n", report.path.display()));
    out.push_str(&row("CLASS", "SAMPLES", "BUDGET", "WEIGHT", "REFIT"));
    for c in &report.classes {
        let samples = if c.samples >= report.min_samples {
            c.samples.to_string()
        } else {
            format!("{}/{}", c.samples, report.min_samples)
        };
        let budget = budget_text(&c.budget);
        out.push_str(&row(
            &c.class,
            &samples,
            &budget,
            &weight(c),
            &refit(c.refit),
        ));
    }
    if report.classes.iter().any(|c| c.weight_derived) {
        out.push_str("  * derived from another class's median\n");
    }
    for c in report.classes.iter().filter(|c| c.configured) {
        if let Some(refit) = &c.refit_budget {
            out.push_str(&format!(
                "  budget {}: configured {} (refit would be {})\n",
                c.class,
                budget_text(&c.budget),
                budget_text(refit)
            ));
        }
    }
    for c in &report.classes {
        if c.route.starts_with("list (config): ") {
            out.push_str(&format!("  route {}: {}\n", c.class, c.route));
        }
    }
    if report.proposals.is_empty() {
        out.push_str("tuning proposals: none\n");
        return out;
    }
    out.push_str("tuning proposals:\n");
    for p in &report.proposals {
        out.push_str(&format!("  {:<12}  {}\n", p.id, p.text));
    }
    out.push_str(
        "apply with anthrex run stats --apply <id>; dismiss with anthrex run stats --dismiss <id>\n",
    );
    out
}

/// One row in fixed widths, unlike M8b's table.
fn row(class: &str, samples: &str, budget: &str, weight: &str, refit: &str) -> String {
    format!("  {class:<5}  {samples:<7}  {budget:<14}  {weight:<6}  {refit}\n")
}

/// `<m>m`, or `<s>s` under a minute, `*` when derived; `-` without weights.
fn weight(c: &ClassTuning) -> String {
    let Some(secs) = c.weight_secs else {
        return "-".to_string();
    };
    let star = if c.weight_derived { "*" } else { "" };
    if secs < 60 {
        format!("{secs}s{star}")
    } else {
        format!("{}m{star}", secs / 60)
    }
}

fn refit(state: RefitState) -> String {
    match state {
        // `yyyy-mm-dd hh:mm` UTC.
        RefitState::Written { at } => format_utc(at).chars().take(16).collect(),
        RefitState::NotYet => "not yet".to_string(),
        RefitState::Kept => "kept".to_string(),
        RefitState::Configured => "configured".to_string(),
        RefitState::Off => "off".to_string(),
    }
}
