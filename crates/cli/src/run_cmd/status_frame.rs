//! Milestone 9.17 decision 19: `anthrex run status`'s plan frame lines, printed after
//! the goal while the snapshot carries a frame (at the plan gate, or while a hold
//! awaits). Pure: text from the snapshot.

use proto::{PlanFrameInfo, RunInfo};

use super::status::one_line;

/// The frame's lines, or nothing: every `decided for you` item, the run's then each
/// epic's (`[<epic>] ` before an epic's), then every `out of scope` item in that order,
/// then a design run's approved Non-goals a line each; `decided for you: none stated`
/// when the frames hold nothing. Every model-written text goes through `one_line`.
pub(super) fn lines(run: &RunInfo) -> String {
    let epics = (run.planners.iter())
        .filter_map(|p| Some((format!("[{}] ", one_line(&p.epic)), p.frame.as_ref()?)));
    let frames: Vec<(String, &PlanFrameInfo)> = (run.plan_frame.iter().map(|f| (String::new(), f)))
        .chain(epics)
        .collect();
    if frames.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    for (tag, frame) in &frames {
        for d in &frame.decided {
            let (q, a) = (one_line(&d.question), one_line(&d.answer));
            out.push_str(&format!("  decided for you: {tag}{q} → {a}\n"));
        }
    }
    for (tag, frame) in &frames {
        for item in &frame.out_of_scope {
            out.push_str(&format!("  out of scope: {tag}{}\n", one_line(item)));
        }
    }
    let goals = (frames.iter()).filter_map(|(_, f)| f.spec_non_goals.as_deref());
    for line in goals.flat_map(str::lines) {
        let line = one_line(line);
        if !line.trim().is_empty() {
            out.push_str(&format!("  spec non-goals: {}\n", line.trim_end()));
        }
    }
    if out.is_empty() {
        out.push_str("  decided for you: none stated\n");
    }
    out
}

#[cfg(test)]
#[path = "status_frame_tests.rs"]
mod tests;
