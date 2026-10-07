//! What the installed CLIs offer a decider call (`DECIDER_CAPS`, set from M8b.1's
//! findings, the "M8b.1 external facts" entry of the milestone's implementation notes),
//! and a decider's argv for each runtime.
//! Pure.

use super::{DeciderContext, DeciderKind};
use crate::headless::codex_sandbox::{Mode, SandboxPlan};
use crate::launch::codex::toml_string;
use proto::Effort;
use std::path::{Path, PathBuf};

/// The decider-specific capabilities of the installed `claude` and `codex`. The argv
/// builders take one, so tests can exercise each branch whatever the installed CLI does;
/// a flag whose capability is `false` is omitted from the argv.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeciderCaps {
    /// `claude --json-schema <schema>` exists.
    pub claude_json_schema: bool,
    /// `claude --max-turns <n>` exists (a hidden flag in 2.1.280).
    pub claude_max_turns: bool,
    /// `claude --no-session-persistence` exists.
    pub claude_no_session_persistence: bool,
    /// Where a Claude decider's JSON answer is taken from first (decision 16's order).
    pub answer_source: AnswerSource,
    /// Whether a structured-output schema must list every property in `required`. The
    /// decider schemas already do, so the value changes nothing in them.
    pub strict_schemas: bool,
    /// Whether `codex exec --output-schema <file>` is passed.
    pub codex_output_schema: bool,
    /// Whether `codex exec --ephemeral` is passed.
    pub codex_ephemeral: bool,
}

/// Where the answer to a Claude decider call appears in its stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnswerSource {
    /// `result.structured_output` on the `result` line.
    ResultField,
    /// The `input` of a top-level `tool_use` named `StructuredOutput`.
    StructuredOutputTool,
    /// The last top-level assistant text, parsed as JSON.
    Text,
}

/// M8b.1's findings against `claude` 2.1.280 (observed calls, fixtures
/// `tests/fixtures/deciders/claude-2.1.280-decider*.jsonl`) and `codex-cli` 0.156.1
/// (`codex exec --help` only; no Codex call was run).
pub const DECIDER_CAPS: DeciderCaps = DeciderCaps {
    // Accepted, and the answer arrived as structured output.
    claude_json_schema: true,
    // Hidden from `--help`, accepted. A call uses 2 turns (the StructuredOutput call and
    // its result), or 3 when the model first answers in text and the CLI makes it call
    // StructuredOutput (observed on triage), which is exactly `--max-turns 3`.
    claude_max_turns: true,
    claude_no_session_persistence: true,
    // The answer was in `result.structured_output`, in a top-level `StructuredOutput`
    // tool use, and as JSON text in `result.result`; there was no assistant text block.
    answer_source: AnswerSource::ResultField,
    // A schema leaving `reason` out of `required` was accepted and answered; the
    // triage schema's `anyOf` with null and `["string","null"]` types were accepted too.
    strict_schemas: false,
    // `--output-schema` exists in `codex exec --help` (controller ruling: a flag that
    // exists is passed). No Codex call was run, so its runtime behaviour is unverified
    // (outstanding for the user); `parse` validates every answer either way.
    codex_output_schema: true,
    // `--ephemeral` ("Run without persisting session files to disk") exists in
    // `codex exec --help`; passing it only reduces what a call writes.
    codex_ephemeral: true,
};

/// The caps the engine picks design agents by (milestone 9.6 ruling WB-A-W2):
/// [`DECIDER_CAPS`]. A test replaces them on its own thread only ([`with_caps`]).
pub fn caps() -> DeciderCaps {
    #[cfg(test)]
    if let Some(caps) = TEST_CAPS.with(std::cell::Cell::get) {
        return caps;
    }
    DECIDER_CAPS
}

#[cfg(test)]
thread_local! {
    static TEST_CAPS: std::cell::Cell<Option<DeciderCaps>> = const { std::cell::Cell::new(None) };
}

/// Runs `f` with [`caps`] answering `caps` on this thread.
#[cfg(test)]
pub fn with_caps<T>(caps: DeciderCaps, f: impl FnOnce() -> T) -> T {
    let before = TEST_CAPS.with(|c| c.replace(Some(caps)));
    let out = f();
    TEST_CAPS.with(|c| c.set(before));
    out
}

/// Every built-in tool that could read or change anything; `--tools ""` already leaves
/// only `StructuredOutput`, and this list keeps them out if a CLI ignores it.
pub const CLAUDE_DECIDER_DISALLOWED: &str =
    "Bash,Edit,Write,NotebookEdit,Agent,WebFetch,WebSearch,Read,Glob,Grep";

/// A Claude decider's argv (decision 16). No `--settings` (a decider has no window and
/// no hooks), no `--mcp-config` (with the caps' `--strict-mcp-config` it has no MCP
/// server), no `--bare` (the user's login is used). `--tools ""` (ruling R-T1-6) leaves
/// only `StructuredOutput`; the caps' user-settings flags keep `--setting-sources user`.
/// The prompt goes on stdin as one stream-json user message.
pub fn claude_decider_args(
    ctx: &DeciderContext,
    dcaps: &DeciderCaps,
    schema: &serde_json::Value,
) -> Vec<String> {
    let caps = &ctx.caps;
    let mut args: Vec<String> = [
        "-p",
        "--input-format",
        "stream-json",
        "--output-format",
        "stream-json",
    ]
    .map(String::from)
    .to_vec();
    if caps.claude_verbose {
        args.push("--verbose".into());
    }
    if caps.claude_permission_prompts {
        args.extend(["--permission-prompts".into(), "none".into()]);
    }
    if let Some(flags) = caps.claude_user_settings_only {
        args.extend(flags.iter().map(|f| f.to_string()));
    }
    args.extend(["--tools".into(), String::new()]);
    args.extend(["--permission-mode".into(), "dontAsk".into()]);
    args.extend(["--disallowedTools".into(), CLAUDE_DECIDER_DISALLOWED.into()]);
    if dcaps.claude_max_turns {
        args.extend(["--max-turns".into(), "3".into()]);
    }
    if dcaps.claude_json_schema {
        args.extend(["--json-schema".into(), schema.to_string()]);
    }
    if dcaps.claude_no_session_persistence {
        args.push("--no-session-persistence".into());
    }
    if !ctx.route.model.is_empty() {
        args.extend(["--model".into(), ctx.route.model.clone()]);
    }
    if caps.claude_effort_flag {
        args.extend(["--effort".into(), effort(ctx.route.effort).into()]);
    }
    args
}

/// A Codex decider's argv (decision 16): read-only sandbox in the caps' dialect
/// (`headless::codex_sandbox`; the legacy one carries the sandbox pins), no approvals, the schema file when the CLI takes one,
/// then `--` and the prompt as the last argument.
pub fn codex_decider_args(
    ctx: &DeciderContext,
    dcaps: &DeciderCaps,
    schema_file: &Path,
    prompt: &str,
) -> Vec<String> {
    let mut args: Vec<String> = ["exec", "--json", "--skip-git-repo-check"]
        .map(String::from)
        .to_vec();
    if dcaps.codex_ephemeral {
        args.push("--ephemeral".into());
    }
    if let Some(flags) = ctx.caps.codex_user_config_only {
        args.extend(flags.iter().map(|f| f.to_string()));
    }
    let plan = SandboxPlan::new(Mode::ReadOnly, PathBuf::new(), vec![], vec![]);
    args.extend(
        ctx.caps
            .codex_dialect()
            .render(&plan, false, true)
            .expect("every dialect expresses read-only"),
    );
    args.extend([
        "-c".into(),
        format!("approval_policy={}", toml_string("never")),
        "-c".into(),
        format!(
            "model_reasoning_effort={}",
            toml_string(effort(ctx.route.effort))
        ),
    ]);
    if dcaps.codex_output_schema {
        args.extend(["--output-schema".into(), schema_file.display().to_string()]);
    }
    if !ctx.route.model.is_empty() {
        args.extend(["-m".into(), ctx.route.model.clone()]);
    }
    args.extend(["--".into(), prompt.to_owned()]);
    args
}

/// `<kind>-<16 hex>.json`: the schema file's name under `<data_dir>/deciders/schemas/`,
/// keyed by the plain 64-bit FNV-1a of the schema's compact JSON, so a changed schema
/// gets a new file and an unchanged one is written once.
pub fn schema_file_name(kind: DeciderKind, schema: &serde_json::Value) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in schema.to_string().bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{}-{hash:016x}.json", kind.label())
}

fn effort(effort: Effort) -> &'static str {
    match effort {
        Effort::Low => "low",
        Effort::Medium => "medium",
        Effort::High => "high",
    }
}
