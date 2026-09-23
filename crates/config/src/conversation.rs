//! The `[conversation]` and `[conversation.badges]` tables (milestone 6.5): their
//! types, their bounds, and their parsing. Split out of `lib.rs` (the final review's
//! M5) to keep every file under the 600-line rule; a pure move.

use super::*;
use unicode_width::UnicodeWidthStr;

/// The `[conversation]` and `[conversation.badges]` tables that conversation-view spec
/// decision 7 (the caps) and decision 10, amended by this milestone's decision A5 (the
/// three runtime badges), assign to milestone 6.5.
///
/// `max_turns` and `max_bytes` are `daemon::conversation::Caps`'s inputs (decision 7);
/// `linger_secs` is `WindowManager::unsubscribe_conversation`'s grace period (decision 9).
/// As with `[git]`, every default is the spec's or this brief's own number, and the
/// *ranges* are this milestone's, since neither gives one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conversation {
    pub max_turns: u64,
    pub max_bytes: u64,
    pub max_result_bytes: u64,
    pub linger_secs: u64,
    pub badges: Badges,
}

/// Inclusive bounds for `conversation.max_turns`. Below the lower bound the view would
/// show less history than a single turn of back-and-forth; above the upper one the cap
/// stops capping anything a real overnight run produces (decision 7's own worry).
pub const CONVERSATION_MAX_TURNS_RANGE: std::ops::RangeInclusive<u64> = 1..=10_000;
/// What `conversation.max_bytes`'s ceiling leaves free below `proto::MAX_FRAME`: a whole
/// conversation travels in one `ConversationSnapshot` frame, and this covers the
/// snapshot's envelope and the turn that decision A9 keeps whole even past the cap.
pub const CONVERSATION_MAX_BYTES_HEADROOM: u64 = 1_048_576;
/// Inclusive bounds for `conversation.max_bytes`. Below the lower bound a single large
/// tool result would blow the cap on its own. The ceiling is derived from the frame the
/// conversation has to fit in (the amendment under task M6.5.1), not typed: 15 MiB.
pub const CONVERSATION_MAX_BYTES_RANGE: std::ops::RangeInclusive<u64> =
    65_536..=(proto::MAX_FRAME as u64 - CONVERSATION_MAX_BYTES_HEADROOM);
const _: () = assert!(
    *CONVERSATION_MAX_BYTES_RANGE.end() + CONVERSATION_MAX_BYTES_HEADROOM
        <= proto::MAX_FRAME as u64,
    "a conversation at conversation.max_bytes must fit in one frame"
);
/// The floor of `conversation.max_result_bytes`. `crates/cli/src/hook.rs`'s
/// `TOOL_RESULT_SUMMARY_MAX` (4 KiB) is a hook-delivered tool result's own hard cap, and its
/// comment claims a hook result can never trip this config cap. That claim only holds if
/// this floor is at least `TOOL_RESULT_SUMMARY_MAX`; `hook.rs` asserts it at compile time
/// (`const _: () = assert!(...)`) directly below `TOOL_RESULT_SUMMARY_MAX`'s own definition,
/// so the two can never drift apart silently again.
pub const CONVERSATION_MAX_RESULT_BYTES_MIN: u64 = 4096;
/// Inclusive bounds for `conversation.max_result_bytes`. The floor is
/// [`CONVERSATION_MAX_RESULT_BYTES_MIN`]; the ceiling keeps one truncated tool result from
/// being most of `conversation.max_bytes`'s own default budget.
pub const CONVERSATION_MAX_RESULT_BYTES_RANGE: std::ops::RangeInclusive<u64> =
    CONVERSATION_MAX_RESULT_BYTES_MIN..=1_048_576;
/// Inclusive bounds for `conversation.linger_secs`. Zero means "unsubscribe re-parses
/// next time"; the upper bound keeps a lingering subscription from outliving a person who
/// switched views and forgot about this window for a while.
pub const CONVERSATION_LINGER_SECS_RANGE: std::ops::RangeInclusive<u64> = 0..=300;

impl Default for Conversation {
    fn default() -> Self {
        Conversation {
            max_turns: 500,
            max_bytes: 2_097_152,
            max_result_bytes: 16384,
            linger_secs: 30,
            badges: Badges::default(),
        }
    }
}

/// The three runtime badges (decision A5: exactly Claude, Codex and Shell -- fake-agent has
/// no `[conversation.badges]` entry of its own). `force_ascii` and the locale check are
/// `tui::ui::badge::prefers_ascii`'s inputs; this crate only parses the keys.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Badges {
    pub force_ascii: bool,
    pub claude: Badge,
    pub codex: Badge,
    pub shell: Badge,
}

impl Badges {
    /// The badge for `runtime`. Total: every `proto::Runtime` variant has a badge.
    pub fn for_runtime(&self, runtime: proto::Runtime) -> &Badge {
        match runtime {
            proto::Runtime::Claude => &self.claude,
            proto::Runtime::Codex => &self.codex,
            proto::Runtime::Shell => &self.shell,
        }
    }
}

impl Default for Badges {
    fn default() -> Self {
        Badges {
            force_ascii: false,
            claude: Badge {
                glyph: "\u{25c6}".to_string(),
                ascii: "[C]".to_string(),
                color: Rgb(0xd7, 0x9b, 0x61),
            },
            codex: Badge {
                glyph: "\u{25c7}".to_string(),
                ascii: "[X]".to_string(),
                color: Rgb(0x7f, 0xc8, 0xb4),
            },
            shell: Badge {
                glyph: "$".to_string(),
                ascii: "[$]".to_string(),
                color: Rgb(0x9a, 0xa0, 0xb5),
            },
        }
    }
}

/// One runtime's badge: a glyph for a unicode terminal, an ASCII fallback, and a colour.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Badge {
    pub glyph: String,
    pub ascii: String,
    pub color: Rgb,
}

const KNOWN_CONVERSATION_KEYS: &[&str] = &[
    "max_turns",
    "max_bytes",
    "max_result_bytes",
    "linger_secs",
    "badges",
];
const KNOWN_BADGES_KEYS: &[&str] = &["force_ascii", "claude", "codex", "shell"];
const KNOWN_BADGE_KEYS: &[&str] = &["glyph", "ascii", "color"];

/// A string's display width in terminal cells, via `unicode-width`. Used for both
/// `badges.*.glyph` (1 or 2 columns) and `badges.*.ascii` (1 to 3, and ASCII-only).
fn display_width(s: &str) -> usize {
    s.width()
}

fn read_badge(table: &toml::Table, prefix: &str, badge: &mut Badge, problems: &mut Vec<Problem>) {
    if let Some(value) = table.get("glyph") {
        match value.as_str().filter(|s| matches!(display_width(s), 1 | 2)) {
            Some(s) => badge.glyph = s.to_string(),
            None => problems.push(Problem {
                key: format!("{prefix}.glyph"),
                message: "expected 1 or 2 display columns".to_string(),
                default: badge.glyph.clone(),
            }),
        }
    }

    if let Some(value) = table.get("ascii") {
        match value
            .as_str()
            .filter(|s| s.is_ascii() && (1..=3).contains(&s.chars().count()))
        {
            Some(s) => badge.ascii = s.to_string(),
            None => problems.push(Problem {
                key: format!("{prefix}.ascii"),
                message: "expected 1 to 3 ASCII characters".to_string(),
                default: badge.ascii.clone(),
            }),
        }
    }

    if let Some(value) = table.get("color") {
        match value.as_str().and_then(parse_rgb) {
            Some(rgb) => badge.color = rgb,
            None => problems.push(Problem {
                key: format!("{prefix}.color"),
                message: "expected # followed by six hex digits".to_string(),
                default: badge.color.to_hex(),
            }),
        }
    }
}

fn read_badges(table: &toml::Table, config: &mut Config, problems: &mut Vec<Problem>) {
    let Some(value) = table.get("badges") else {
        return;
    };
    let Some(badges) = value.as_table() else {
        problems.push(not_a_table_problem("conversation.badges"));
        return;
    };

    read_bool_key(
        badges,
        "force_ascii",
        "conversation.badges.force_ascii",
        &mut config.conversation.badges.force_ascii,
        problems,
    );

    if let Some(value) = badges.get("claude") {
        match value.as_table() {
            Some(claude) => read_badge(
                claude,
                "conversation.badges.claude",
                &mut config.conversation.badges.claude,
                problems,
            ),
            None => problems.push(not_a_table_problem("conversation.badges.claude")),
        }
    }

    if let Some(value) = badges.get("codex") {
        match value.as_table() {
            Some(codex) => read_badge(
                codex,
                "conversation.badges.codex",
                &mut config.conversation.badges.codex,
                problems,
            ),
            None => problems.push(not_a_table_problem("conversation.badges.codex")),
        }
    }

    if let Some(value) = badges.get("shell") {
        match value.as_table() {
            Some(shell) => read_badge(
                shell,
                "conversation.badges.shell",
                &mut config.conversation.badges.shell,
                problems,
            ),
            None => problems.push(not_a_table_problem("conversation.badges.shell")),
        }
    }
}

pub(super) fn read_conversation(
    table: &toml::Table,
    config: &mut Config,
    problems: &mut Vec<Problem>,
) {
    let Some(value) = table.get("conversation") else {
        return;
    };
    let Some(conversation) = value.as_table() else {
        problems.push(not_a_table_problem("conversation"));
        return;
    };

    read_u64_in_range(
        conversation,
        "max_turns",
        "conversation.max_turns",
        &CONVERSATION_MAX_TURNS_RANGE,
        &mut config.conversation.max_turns,
        problems,
    );
    read_u64_in_range(
        conversation,
        "max_bytes",
        "conversation.max_bytes",
        &CONVERSATION_MAX_BYTES_RANGE,
        &mut config.conversation.max_bytes,
        problems,
    );
    read_u64_in_range(
        conversation,
        "max_result_bytes",
        "conversation.max_result_bytes",
        &CONVERSATION_MAX_RESULT_BYTES_RANGE,
        &mut config.conversation.max_result_bytes,
        problems,
    );
    read_u64_in_range(
        conversation,
        "linger_secs",
        "conversation.linger_secs",
        &CONVERSATION_LINGER_SECS_RANGE,
        &mut config.conversation.linger_secs,
        problems,
    );
    read_badges(conversation, config, problems);
}

/// Mirrors `report_unknown_runtimes`'s shape, but one level deeper: `[conversation]` has
/// its own unknown keys, plus `badges`, which has its own three runtime tables.
pub(super) fn report_unknown_conversation(value: &toml::Value, problems: &mut Vec<Problem>) {
    let Some(table) = value.as_table() else {
        return;
    };
    for (key, sub) in table {
        if key == "badges" {
            report_unknown_badges(sub, problems);
        } else if !KNOWN_CONVERSATION_KEYS.contains(&key.as_str()) {
            problems.push(unknown_key_problem(&format!("conversation.{key}")));
        }
    }
}

fn report_unknown_badges(value: &toml::Value, problems: &mut Vec<Problem>) {
    let Some(table) = value.as_table() else {
        return;
    };
    for (key, sub) in table {
        match key.as_str() {
            "claude" => report_unknown_nested(
                sub,
                "conversation.badges.claude",
                KNOWN_BADGE_KEYS,
                problems,
            ),
            "codex" => {
                report_unknown_nested(sub, "conversation.badges.codex", KNOWN_BADGE_KEYS, problems)
            }
            "shell" => {
                report_unknown_nested(sub, "conversation.badges.shell", KNOWN_BADGE_KEYS, problems)
            }
            other if KNOWN_BADGES_KEYS.contains(&other) => {}
            other => problems.push(unknown_key_problem(&format!("conversation.badges.{other}"))),
        }
    }
}
