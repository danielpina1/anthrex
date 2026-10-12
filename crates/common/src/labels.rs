//! The labels a run's triage prints, in the daemon's reports and the client's inspector
//! (decision 10). Pure.

use proto::{DeciderSource, Scale, TaskKind};

/// `code`, `docs`, `research` or `review`.
pub fn kind_label(kind: TaskKind) -> &'static str {
    match kind {
        TaskKind::Code => "code",
        TaskKind::Docs => "docs",
        TaskKind::Research => "research",
        TaskKind::Review => "review",
    }
}

pub fn scale_label(scale: Scale) -> &'static str {
    match scale {
        Scale::Single => "single",
        Scale::Plan => "plan",
        Scale::Large => "large",
    }
}

pub fn source_label(source: DeciderSource) -> &'static str {
    match source {
        DeciderSource::Decider => "decider",
        DeciderSource::Fallback => "fallback",
    }
}

/// `<kinds joined with ,>/<scale>`, as `code/single`.
pub fn kinds_scale(kinds: &[TaskKind], scale: Scale) -> String {
    let kinds: Vec<&str> = kinds.iter().map(|k| kind_label(*k)).collect();
    format!("{}/{}", kinds.join(","), scale_label(scale))
}
