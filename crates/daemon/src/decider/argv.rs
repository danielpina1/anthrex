//! What the installed CLIs offer a decider call (`DECIDER_CAPS`, set from M8b.1's
//! findings, the "M8b.1 external facts" entry of the milestone's implementation notes).
//! Pure.

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
