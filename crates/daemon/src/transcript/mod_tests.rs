use super::*;

pub(super) const CLAUDE_FIXTURE: &str =
    include_str!("../../tests/fixtures/transcripts/claude-2.1.278.jsonl");
pub(super) const CODEX_FIXTURE: &str =
    include_str!("../../tests/fixtures/transcripts/codex-0.155.0.jsonl");

/// The prompt both captures were given, verbatim.
pub(super) const PROMPT: &str = "First say exactly: Starting the check. Then read the file \
    Cargo.toml. Then read the file .gitignore. Then reply with exactly one word: ready";

/// The workspace `Cargo.toml` as both captures read it. Written out rather than
/// `include_str!`ed from the live file, so a later edit to the manifest cannot move the
/// expected value away from what the frozen fixture holds.
pub(super) const CARGO_TOML: &str = r#"[workspace]
resolver = "3"
members = ["crates/proto", "crates/config", "crates/daemon", "crates/tui", "crates/cli", "crates/fake-agent"]

[workspace.package]
version = "0.1.0"
edition = "2024"
rust-version = "1.92"
license = "MIT"

[workspace.dependencies]
proto = { path = "crates/proto", package = "anthrex-proto" }
config = { path = "crates/config", package = "anthrex-config" }
daemon = { path = "crates/daemon", package = "anthrex-daemon" }
tui = { path = "crates/tui", package = "anthrex-tui" }
anyhow = "1"
thiserror = "2"
serde = { version = "1", features = ["derive"] }
serde_bytes = "0.11"
serde_json = "1"
sha2 = "0.10"
rmp-serde = "1"
tokio = { version = "1", features = ["rt-multi-thread", "macros", "net", "io-util", "sync", "time", "signal"] }
tokio-util = "0.7"
toml = "1"
bytes = "1"
dirs = "6"
libc = "0.2"
notify = "8"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
tracing-appender = "0.2"
portable-pty = "0.9"
vt100 = "0.16"
ratatui = "0.30"
crossterm = { version = "0.29", features = ["event-stream"] }
tui-term = "0.3"
clap = { version = "4", features = ["derive"] }
futures = "0.3"
tempfile = "3"
unicode-width = "0.2"
unicode-segmentation = "1"

[profile.release]
lto = "thin"
codegen-units = 1
strip = true"#;

/// The first line of a fixture.
pub(super) fn first_line(fixture: &str) -> &str {
    fixture.lines().next().expect("a fixture has a first line")
}

/// Reads a whole file the way the reader will: detect on the first line, then feed
/// every line, the first included, through one cursor, in order. Goes through
/// `parser_for` and the trait so nothing here bypasses what the reader calls.
pub(super) fn parse_all(runtime: proto::Runtime, text: &str) -> Vec<Record> {
    let parser = parser_for(runtime).expect("this runtime has a parser");
    let version = parser
        .detect(first_line(text))
        .expect("the fixture is detected");
    let mut cursor = Cursor::default();
    text.lines()
        .flat_map(|line| parser.record(version, line, &mut cursor))
        .collect()
}

/// Feeds the given lines through one cursor at `Version(1)`.
pub(super) fn parse_lines(runtime: proto::Runtime, lines: &[&str]) -> Vec<Record> {
    let parser = parser_for(runtime).expect("this runtime has a parser");
    let mut cursor = Cursor::default();
    lines
        .iter()
        .flat_map(|line| parser.record(Version(1), line, &mut cursor))
        .collect()
}

/// Decision 1: a transcript may add prose and tool detail, and may never create,
/// reorder or remove a turn. Every field of every variant is named here with no `..`,
/// so adding a variant or a field fails to compile until this match is revisited. None
/// of the fields may be a role change, a position in the timeline, or a tool state:
/// `ordinal` only says which hook-built turn to look up, and `ok` is detail the
/// enricher never turns into a `ToolState`.
pub(super) fn assert_carries_no_turn_boundary(record: &Record) {
    match record {
        Record::UserText {
            session_id: _,
            ordinal: _,
            text: _,
        } => {}
        Record::AssistantText {
            session_id: _,
            ordinal: _,
            text: _,
        } => {}
        Record::ToolDetail {
            tool_use_id: _,
            input: _,
            detail: _,
            ok: _,
        } => {}
    }
}

#[test]
fn parser_for_maps_each_runtime() {
    let claude = parser_for(proto::Runtime::Claude).expect("claude has a parser");
    assert_eq!(claude.runtime(), proto::Runtime::Claude);
    let codex = parser_for(proto::Runtime::Codex).expect("codex has a parser");
    assert_eq!(codex.runtime(), proto::Runtime::Codex);
    assert!(parser_for(proto::Runtime::Shell).is_none());
}

#[test]
fn each_parser_rejects_the_others_fixture() {
    let claude = parser_for(proto::Runtime::Claude).unwrap();
    let codex = parser_for(proto::Runtime::Codex).unwrap();
    // Each accepts its own, so a `None` below is a rejection and not a parser that
    // accepts nothing.
    assert_eq!(claude.detect(first_line(CLAUDE_FIXTURE)), Some(Version(1)));
    assert_eq!(codex.detect(first_line(CODEX_FIXTURE)), Some(Version(1)));
    assert_eq!(claude.detect(first_line(CODEX_FIXTURE)), None);
    assert_eq!(codex.detect(first_line(CLAUDE_FIXTURE)), None);
}

#[test]
fn each_parser_rejects_every_line_of_the_others_fixture() {
    // Stronger than the first line alone: whichever line a file happens to start with,
    // the wrong parser must not claim it.
    let claude = parser_for(proto::Runtime::Claude).unwrap();
    let codex = parser_for(proto::Runtime::Codex).unwrap();
    for line in CODEX_FIXTURE.lines() {
        assert_eq!(claude.detect(line), None, "claude accepted {line}");
    }
    for line in CLAUDE_FIXTURE.lines() {
        assert_eq!(codex.detect(line), None, "codex accepted {line}");
    }
}

#[test]
fn an_unknown_version_yields_nothing() {
    for runtime in [proto::Runtime::Claude, proto::Runtime::Codex] {
        let parser = parser_for(runtime).unwrap();
        let fixture = match runtime {
            proto::Runtime::Claude => CLAUDE_FIXTURE,
            _ => CODEX_FIXTURE,
        };
        let mut cursor = Cursor::default();
        for line in fixture.lines() {
            assert!(parser.record(Version(2), line, &mut cursor).is_empty());
        }
        assert_eq!(cursor, Cursor::default());
    }
}

/// Review F1: the first lines of the user's 94 real Claude transcripts, measured by
/// key presence: `queue-operation` 52, `bridge-session` 28, `last-prompt` 10,
/// `custom-title` 4, every one with a string `sessionId`. Each shape is detected alone.
#[test]
fn every_measured_claude_first_line_shape_is_detected() {
    let claude = parser_for(proto::Runtime::Claude).unwrap();
    for line in [
        r#"{"type":"queue-operation","operation":"enqueue","timestamp":"t","sessionId":"s","content":"x"}"#,
        r#"{"type":"bridge-session","sessionId":"s"}"#,
        r#"{"type":"last-prompt","leafUuid":"u","sessionId":"s"}"#,
        r#"{"type":"custom-title","customTitle":"x","sessionId":"s"}"#,
    ] {
        assert_eq!(claude.detect(line), Some(Version(1)), "{line}");
    }
}

/// Review F1: a head whose first lines carry no `sessionId` is still detected by the
/// first message record behind them.
#[test]
fn detect_head_looks_past_a_first_line_without_a_session() {
    let claude = parser_for(proto::Runtime::Claude).unwrap();
    let prompt = CLAUDE_FIXTURE.lines().nth(7).unwrap();
    let head = [
        r#"{"type":"summary","summary":"x","leafUuid":"u"}"#,
        r#"{"type":"queue-operation","operation":"enqueue","timestamp":"t","content":"x"}"#,
        prompt,
    ];
    assert_eq!(claude.detect(head[0]), None);
    assert_eq!(claude.detect(head[1]), None);
    assert_eq!(detect_head(claude, head), Some(Version(1)));
}

#[test]
fn detect_head_stops_after_its_bound() {
    let claude = parser_for(proto::Runtime::Claude).unwrap();
    let prompt = CLAUDE_FIXTURE.lines().nth(7).unwrap();
    let filler = r#"{"type":"summary","summary":"x"}"#;
    let mut head = vec![filler; DETECT_LINES - 1];
    head.push(prompt);
    assert_eq!(detect_head(claude, head.iter().copied()), Some(Version(1)));
    head.insert(0, filler);
    assert_eq!(detect_head(claude, head.iter().copied()), None);
}

#[test]
fn detect_head_keeps_each_parser_to_its_own_fixture() {
    let claude = parser_for(proto::Runtime::Claude).unwrap();
    let codex = parser_for(proto::Runtime::Codex).unwrap();
    assert_eq!(
        detect_head(claude, CLAUDE_FIXTURE.lines()),
        Some(Version(1))
    );
    assert_eq!(detect_head(codex, CODEX_FIXTURE.lines()), Some(Version(1)));
    assert_eq!(detect_head(claude, CODEX_FIXTURE.lines()), None);
    assert_eq!(detect_head(codex, CLAUDE_FIXTURE.lines()), None);
}
