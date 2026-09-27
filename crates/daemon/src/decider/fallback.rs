//! Decision 17's deterministic fallbacks: the answer every decider gives when it is off,
//! fails, or cannot get a slot. A fallback is never an error and never blocks a run.
//! Pure.

use super::{BlockKind, DeciderAnswer, DeciderRequest, Decision, SizeVerdict, TriageAnswer};
use proto::{DeciderSource, Scale, TaskKind};

/// Triage's fallback reason (spec §5.1: "Without a decider, the path is plan").
pub const TRIAGE_FALLBACK_REASON: &str = "without a decider, the path is plan";
/// A size-check fallback verdict's reason: the task keeps the engine's size.
pub const SIZE_FALLBACK_REASON: &str = "the engine's size is kept";
/// The blocked-reason fallback's reason (M8a decision 32's default kind).
pub const BLOCKED_FALLBACK_REASON: &str = "an unclassified block is a question";

/// The deterministic answer to `request`:
/// - triage: kinds `[code]`, scale `plan`, no task;
/// - size check: every task keeps its size;
/// - check summary: M8a's last 40 lines (`run::exec::summary`);
/// - blocked reason: `question`.
pub fn fallback(request: &DeciderRequest) -> DeciderAnswer {
    match request {
        DeciderRequest::Triage(_) => DeciderAnswer::Triage(TriageAnswer {
            kinds: vec![TaskKind::Code],
            scale: Scale::Plan,
            reason: TRIAGE_FALLBACK_REASON.into(),
            task: None,
        }),
        DeciderRequest::SizeCheck(input) => DeciderAnswer::SizeCheck(
            input
                .tasks
                .iter()
                .map(|t| SizeVerdict {
                    id: t.id.clone(),
                    size: t.size,
                    reason: SIZE_FALLBACK_REASON.into(),
                })
                .collect(),
        ),
        DeciderRequest::CheckSummary(input) => DeciderAnswer::CheckSummary {
            lines: crate::run::exec::summary(&input.tail)
                .split('\n')
                .map(String::from)
                .collect(),
        },
        DeciderRequest::BlockedReason(_) => DeciderAnswer::BlockedReason {
            kind: BlockKind::Question,
            reason: BLOCKED_FALLBACK_REASON.into(),
        },
    }
}

/// Decision 16's fallback reason with the deciders off (exact).
pub const OFF_REASON: &str = "deciders are off";
/// Decision 16's reason for the fast path's task, which is never cross-checked
/// (decision 19): triage sized it.
pub const SIZED_BY_TRIAGE: &str = "sized by triage";

/// The fallback [`Decision`] for `request`, with one of decision 16's reasons.
pub fn fallback_decision(request: &DeciderRequest, reason: String) -> Decision {
    Decision {
        kind: request.kind(),
        answer: fallback(request),
        source: DeciderSource::Fallback,
        fallback_reason: Some(reason),
        usage: None,
        secs: 0,
    }
}
