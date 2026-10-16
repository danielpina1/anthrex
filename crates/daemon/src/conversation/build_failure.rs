//! Milestone 9.18 decision 13: a failed tool call closes on `PostToolUseFailure`.
//! Claude Code fires it instead of `PostToolUse` (milestone 9.14 fact F4), with the
//! same `tool_use_id`, an `error` and `is_interrupt`, and no `tool_response`. Pure, as
//! `build.rs` is.

use super::{Caps, Draft, build};
use crate::hooks::{HookKind, ParsedHook};
use serde_json::json;
use std::time::Instant;

/// The summary of an interrupted call whose `error` is empty.
const INTERRUPTED: &str = "interrupted";

/// Closes the matching open call as `build::post_tool_use` closes one whose
/// `tool_response` is `{"error": <error>}`: `Failed`, its summary the error's first
/// line (`interrupted` when `is_interrupt` is true and `error` is empty), its duration
/// measured. `false`, changing nothing, when no open call matches. The transcript
/// reader's later result for the call replaces the detail exactly as it does after a
/// `PostToolUse`.
pub(super) fn post_tool_use_failure(
    draft: &mut Draft,
    hook: &ParsedHook,
    now: Instant,
    caps: Caps,
) -> bool {
    let error = hook.error.as_deref().unwrap_or_default();
    let text = if error.is_empty() && hook.is_interrupt == Some(true) {
        INTERRUPTED
    } else {
        error
    };
    let as_post = ParsedHook {
        kind: HookKind::PostToolUse,
        tool_response: Some(json!({ "error": text })),
        ..hook.clone()
    };
    build::post_tool_use(draft, &as_post, now, caps)
}

#[cfg(test)]
#[path = "build_failure_tests.rs"]
mod tests;
