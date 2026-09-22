//! Transcript parsers: one per runtime, version-tagged (spec decision 3).
//!
//! A parser turns one line of a runtime's own transcript file into the few [`Record`]s
//! the conversation enricher may apply. Everything here is pure: no I/O, no clock, and
//! no line can make a parser panic or error. A line a version does not model yields no
//! records (spec decision 2: a conversation degrades, it does not fail).

mod claude;
mod codex;

/// A transcript format version, as recognised by a parser's `detect`. It names the
/// shape of the file, not the CLI release that wrote it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Version(pub u32);

/// What a parser can hand back. Deliberately small: decision 1 lets a transcript add
/// prose and tool detail and nothing else, so there is no variant that could create,
/// reorder or re-state a turn.
#[derive(Debug, Clone, PartialEq)]
pub enum Record {
    /// The text of the transcript's `ordinal`-th real user prompt (0-based).
    UserText {
        session_id: Option<String>,
        ordinal: u32,
        text: String,
    },
    /// One piece of assistant prose that follows the `ordinal`-th prompt. Several may
    /// share an ordinal: a runtime can write one line per content block.
    AssistantText {
        session_id: Option<String>,
        ordinal: u32,
        text: String,
    },
    /// Detail for one tool call, keyed by the runtime's tool-use id. A call line
    /// carries `input` only; a result line carries `detail` and `ok` only. The enricher
    /// merges them by whichever fields are `Some`.
    ToolDetail {
        tool_use_id: String,
        input: Option<serde_json::Value>,
        detail: Option<String>,
        ok: Option<bool>,
    },
}

/// The state a parser carries from one line to the next of the same file: which turn
/// the next line belongs to, and anything a runtime states once and not per line. Start
/// each file with `Cursor::default()` and pass the same cursor to every line in order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Cursor {
    /// How many real user prompts this file has shown so far.
    prompts: u32,
    /// A session id stated once for the whole file (Codex's `session_meta`).
    session_id: Option<String>,
}

impl Cursor {
    /// Counts a real prompt and returns its 0-based ordinal.
    fn next_prompt(&mut self) -> u32 {
        let ordinal = self.prompts;
        self.prompts = self.prompts.saturating_add(1);
        ordinal
    }

    /// The ordinal of the turn the current line follows, or `None` before any prompt:
    /// prose with no prompt ahead of it has no turn to land on.
    fn current_turn(&self) -> Option<u32> {
        self.prompts.checked_sub(1)
    }
}

pub trait TranscriptParser: Send + Sync {
    fn runtime(&self) -> proto::Runtime;
    /// Recognises a transcript format from one line. `None` means this line does not
    /// show the format; the reader asks [`detect_head`], which tries the first
    /// [`DETECT_LINES`] lines, and degrades rather than guessing when none is recognised.
    fn detect(&self, first_line: &str) -> Option<Version>;
    /// Pure, total, and never an error: a line this version does not model yields an
    /// empty vec. `cursor` carries turn position across the lines of one file.
    fn record(&self, version: Version, line: &str, cursor: &mut Cursor) -> Vec<Record>;
}

static CLAUDE: claude::ClaudeParser = claude::ClaudeParser;
static CODEX: codex::CodexParser = codex::CodexParser;

/// The parser for a runtime's transcripts, or `None` for a runtime that writes none.
pub fn parser_for(runtime: proto::Runtime) -> Option<&'static dyn TranscriptParser> {
    match runtime {
        proto::Runtime::Claude => Some(&CLAUDE),
        proto::Runtime::Codex => Some(&CODEX),
        proto::Runtime::Shell => None,
    }
}

/// How many lines at the head of a file `detect_head` tries.
pub const DETECT_LINES: usize = 16;

/// Recognises a transcript by the first line among its first [`DETECT_LINES`] that the
/// parser recognises. The reader calls this, never `detect` directly.
///
/// A Claude file's first line is a bookkeeping record, and one without a `sessionId`
/// (an old `summary`, say) says nothing about the format, while a message record follows
/// within a few lines (review F1). Codex's first line is always `session_meta`, so the
/// lookahead is harmless there.
pub fn detect_head<'a>(
    parser: &dyn TranscriptParser,
    lines: impl IntoIterator<Item = &'a str>,
) -> Option<Version> {
    lines
        .into_iter()
        .take(DETECT_LINES)
        .find_map(|line| parser.detect(line))
}

/// Parses a line as a JSON object, or `None` for anything else.
fn object(line: &str) -> Option<serde_json::Map<String, serde_json::Value>> {
    match serde_json::from_str(line) {
        Ok(serde_json::Value::Object(map)) => Some(map),
        _ => None,
    }
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
