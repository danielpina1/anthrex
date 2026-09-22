//! Config parsing for anthrex: `config.toml`, read by the daemon, the client
//! and the CLI. Parsing never fails: each key is validated on its own, and an
//! invalid or unknown key produces a [`Problem`] while its value keeps the
//! built-in default. See milestone 6 decisions 1-7.

use std::path::Path;
use std::str::FromStr;

use unicode_width::UnicodeWidthStr;

/// The parsed, validated configuration. Always usable: any invalid or
/// unknown key in the source file is reported as a [`Problem`] and the
/// affected field keeps its default instead of failing the whole parse.
#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub prefix: Prefix,
    pub accent: Rgb,
    pub bell: Bell,
    pub default_runtime: proto::Runtime,
    pub scrollback_lines: usize,
    pub ui: Ui,
    pub panes: Panes,
    pub runtimes: Runtimes,
    pub git: Git,
    pub conversation: Conversation,
}

/// `Ctrl` plus this lowercase letter. Never `h`, `i`, `j` or `m`: terminals
/// send those as Backspace, Tab and Enter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Prefix(pub char);

impl Prefix {
    /// The display form, e.g. `"C-b"`.
    pub fn label(&self) -> String {
        format!("C-{}", self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    fn to_hex(self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.0, self.1, self.2)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bell {
    pub attention: bool,
    pub done: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ui {
    pub sidebar_width: Option<u16>,
    pub tree_keep_finished_secs: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Panes {
    pub max: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Runtimes {
    pub claude: RuntimeCommand,
    pub codex: RuntimeCommand,
    pub codex_bypass_hook_trust: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeCommand {
    pub command: String,
}

/// The `[git]` table that git-surface spec 3.7 assigns to this milestone: "Milestone 6
/// adds a `[git]` table with `enabled`, `poll_secs`, `debounce_ms` and an `ignore` list
/// appended to the built-in filter."
///
/// Every default here is that spec's own number, not a new choice: section 3.3 sets the
/// 30-second safety poll and the 300 ms debounce, and section 3.7 leaves the subsystem on
/// unless `ANTHREX_GIT` turns it off. The *ranges* below are this milestone's, since no
/// spec gives one.
///
/// `enabled` cannot turn git back on: the daemon computes
/// `git::enabled_from_env() && config.git.enabled`, so `ANTHREX_GIT=off` always wins.
/// An escape hatch a config file could override would not be one, and the smoke script
/// and CI depend on the variable winning.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Git {
    pub enabled: bool,
    pub poll_secs: u64,
    pub debounce_ms: u64,
    /// Extra path components appended to the daemon's built-in deny list
    /// (`daemon::git::watch::DENY_COMPONENTS`). Each is a single component, matched
    /// against a path's components exactly as the built-in names are.
    pub ignore: Vec<String>,
}

/// Inclusive bounds for `git.poll_secs`. Below the lower bound a repository is probed
/// often enough to be its own event storm; above the upper one the "safety" poll is no
/// longer a safety net within any session a person would notice.
pub const GIT_POLL_SECS_RANGE: std::ops::RangeInclusive<u64> = 5..=3600;
/// Inclusive bounds for `git.debounce_ms`. Below the lower bound the debounce stops
/// coalescing a burst of writes at all; above the upper one the bottom bar lags an edit
/// by longer than a person waits before believing it is broken.
pub const GIT_DEBOUNCE_MS_RANGE: std::ops::RangeInclusive<u64> = 50..=5000;
/// How many entries `git.ignore` may carry, and how long each may be. The filter runs
/// on every accepted filesystem event, so the list is bounded rather than free.
pub const GIT_IGNORE_MAX_ENTRIES: usize = 32;
pub const GIT_IGNORE_MAX_CHARS: usize = 64;

impl Default for Git {
    fn default() -> Self {
        Git {
            enabled: true,
            poll_secs: 30,
            debounce_ms: 300,
            ignore: Vec::new(),
        }
    }
}

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

/// One key that could not be used as written: the value was the wrong type,
/// out of range, or the key itself is unknown. The affected field kept its
/// default.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    pub key: String,
    pub message: String,
    pub default: String,
}

impl std::fmt::Display for Problem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {} (using {})", self.key, self.message, self.default)
    }
}

impl Default for Config {
    fn default() -> Self {
        Config {
            prefix: Prefix('b'),
            accent: Rgb(0x89, 0xb4, 0xfa),
            bell: Bell {
                attention: true,
                done: false,
            },
            default_runtime: proto::Runtime::Shell,
            scrollback_lines: 5000,
            ui: Ui {
                sidebar_width: None,
                tree_keep_finished_secs: 300,
            },
            panes: Panes { max: 6 },
            runtimes: Runtimes {
                claude: RuntimeCommand {
                    command: "claude".to_string(),
                },
                codex: RuntimeCommand {
                    command: "codex".to_string(),
                },
                codex_bypass_hook_trust: false,
            },
            git: Git::default(),
            conversation: Conversation::default(),
        }
    }
}

/// Known top-level and nested key names, used to detect and report unknown
/// keys (decision 5). `orchestrator` is deliberately absent: milestone 8
/// parses it, so it is skipped everywhere, silently.
const KNOWN_BELL_KEYS: &[&str] = &["attention", "done"];
const KNOWN_UI_KEYS: &[&str] = &["sidebar_width", "tree_keep_finished_secs"];
const KNOWN_PANES_KEYS: &[&str] = &["max"];
const KNOWN_RUNTIME_COMMAND_KEYS: &[&str] = &["command"];
const KNOWN_RUNTIME_CODEX_KEYS: &[&str] = &["command", "bypass_hook_trust"];
const KNOWN_GIT_KEYS: &[&str] = &["enabled", "poll_secs", "debounce_ms", "ignore"];
const KNOWN_CONVERSATION_KEYS: &[&str] = &[
    "max_turns",
    "max_bytes",
    "max_result_bytes",
    "linger_secs",
    "badges",
];
const KNOWN_BADGES_KEYS: &[&str] = &["force_ascii", "claude", "codex", "shell"];
const KNOWN_BADGE_KEYS: &[&str] = &["glyph", "ascii", "color"];

/// Parse `text` as `config.toml`. Never fails: a TOML syntax error, an
/// out-of-range or wrong-typed value, or an unknown key each produce one
/// [`Problem`] and the affected field (or the whole config, for a syntax
/// error) keeps its default.
pub fn parse(text: &str) -> (Config, Vec<Problem>) {
    let mut config = Config::default();
    let mut problems = Vec::new();

    let table: toml::Table = match text.parse() {
        Ok(t) => t,
        Err(e) => {
            problems.push(Problem {
                key: "<config>".to_string(),
                message: format!("invalid TOML: {e}"),
                default: "all defaults".to_string(),
            });
            return (config, problems);
        }
    };

    read_prefix(&table, &mut config, &mut problems);
    read_accent(&table, &mut config, &mut problems);
    read_default_runtime(&table, &mut config, &mut problems);
    read_scrollback_lines(&table, &mut config, &mut problems);
    // The legacy alias runs before the `[ui]` table so that, when both are
    // present, `ui.sidebar_width` (read next) wins (decision 5).
    read_legacy_sidebar_width(&table, &mut config, &mut problems);
    read_bell(&table, &mut config, &mut problems);
    read_ui(&table, &mut config, &mut problems);
    read_panes(&table, &mut config, &mut problems);
    read_runtimes(&table, &mut config, &mut problems);
    read_git(&table, &mut config, &mut problems);
    read_conversation(&table, &mut config, &mut problems);

    report_unknown_keys(&table, &mut problems);

    (config, problems)
}

/// Load and parse the config file at `path`. A missing file is not a
/// problem: it yields defaults, silently. Any other read failure (the path
/// is a directory, permissions deny reading it, or its bytes are not UTF-8)
/// is reported as a [`Problem`] naming the path and the reason, so it is
/// never silently indistinguishable from "no config file" -- but it must
/// still never stop the daemon, client or CLI from starting, so defaults are
/// returned either way.
pub fn load(path: &Path) -> (Config, Vec<Problem>) {
    match std::fs::read_to_string(path) {
        Ok(text) => parse(&text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (Config::default(), Vec::new()),
        Err(e) => {
            let problems = vec![Problem {
                key: "<config>".to_string(),
                message: format!("could not read {}: {e}", path.display()),
                default: "all defaults".to_string(),
            }];
            (Config::default(), problems)
        }
    }
}

fn unknown_key_problem(key: &str) -> Problem {
    Problem {
        key: key.to_string(),
        message: "unknown key, ignored".to_string(),
        default: "nothing".to_string(),
    }
}

fn not_a_table_problem(key: &str) -> Problem {
    Problem {
        key: key.to_string(),
        message: "expected a table".to_string(),
        default: "table of defaults".to_string(),
    }
}

fn read_prefix(table: &toml::Table, config: &mut Config, problems: &mut Vec<Problem>) {
    let Some(value) = table.get("prefix") else {
        return;
    };
    match value.as_str().and_then(parse_prefix_char) {
        Some(c) => config.prefix = Prefix(c),
        None => problems.push(Problem {
            key: "prefix".to_string(),
            message: "expected C- followed by a letter".to_string(),
            default: config.prefix.label(),
        }),
    }
}

fn parse_prefix_char(s: &str) -> Option<char> {
    let rest = s.strip_prefix("C-")?;
    let mut chars = rest.chars();
    let c = chars.next()?;
    if chars.next().is_some() {
        return None;
    }
    if !c.is_ascii_lowercase() || matches!(c, 'h' | 'i' | 'j' | 'm') {
        return None;
    }
    Some(c)
}

fn read_accent(table: &toml::Table, config: &mut Config, problems: &mut Vec<Problem>) {
    let Some(value) = table.get("accent") else {
        return;
    };
    match value.as_str().and_then(parse_rgb) {
        Some(rgb) => config.accent = rgb,
        None => problems.push(Problem {
            key: "accent".to_string(),
            message: "expected # followed by six hex digits".to_string(),
            default: config.accent.to_hex(),
        }),
    }
}

fn parse_rgb(s: &str) -> Option<Rgb> {
    let hex = s.strip_prefix('#')?;
    if hex.len() != 6 || !hex.is_ascii() {
        return None;
    }
    let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
    Some(Rgb(r, g, b))
}

fn read_default_runtime(table: &toml::Table, config: &mut Config, problems: &mut Vec<Problem>) {
    let Some(value) = table.get("default_runtime") else {
        return;
    };
    match value
        .as_str()
        .and_then(|s| proto::Runtime::from_str(s).ok())
    {
        Some(runtime) => config.default_runtime = runtime,
        None => problems.push(Problem {
            key: "default_runtime".to_string(),
            message: "expected claude, codex or shell".to_string(),
            default: config.default_runtime.to_string(),
        }),
    }
}

fn read_scrollback_lines(table: &toml::Table, config: &mut Config, problems: &mut Vec<Problem>) {
    let Some(value) = table.get("scrollback_lines") else {
        return;
    };
    match value.as_integer().filter(|n| (0..=100_000).contains(n)) {
        Some(n) => config.scrollback_lines = n as usize,
        None => problems.push(Problem {
            key: "scrollback_lines".to_string(),
            message: "must be between 0 and 100000".to_string(),
            default: config.scrollback_lines.to_string(),
        }),
    }
}

fn read_legacy_sidebar_width(
    table: &toml::Table,
    config: &mut Config,
    problems: &mut Vec<Problem>,
) {
    let Some(value) = table.get("sidebar_width") else {
        return;
    };
    match parse_sidebar_width(value) {
        Some(n) => {
            config.ui.sidebar_width = Some(n);
            problems.push(Problem {
                key: "sidebar_width".to_string(),
                message: "renamed to ui.sidebar_width".to_string(),
                default: n.to_string(),
            });
        }
        None => problems.push(Problem {
            key: "sidebar_width".to_string(),
            message: "must be between 24 and 60".to_string(),
            default: sidebar_width_default_text(config.ui.sidebar_width),
        }),
    }
}

fn parse_sidebar_width(value: &toml::Value) -> Option<u16> {
    value
        .as_integer()
        .and_then(|n| u16::try_from(n).ok())
        .filter(|n| (24..=60).contains(n))
}

fn sidebar_width_default_text(width: Option<u16>) -> String {
    match width {
        Some(n) => n.to_string(),
        None => "unset".to_string(),
    }
}

fn read_bool_key(
    table: &toml::Table,
    local_key: &str,
    full_key: &str,
    field: &mut bool,
    problems: &mut Vec<Problem>,
) {
    let Some(value) = table.get(local_key) else {
        return;
    };
    match value.as_bool() {
        Some(b) => *field = b,
        None => problems.push(Problem {
            key: full_key.to_string(),
            message: "expected a boolean".to_string(),
            default: field.to_string(),
        }),
    }
}

fn read_bell(table: &toml::Table, config: &mut Config, problems: &mut Vec<Problem>) {
    let Some(value) = table.get("bell") else {
        return;
    };
    let Some(bell) = value.as_table() else {
        problems.push(not_a_table_problem("bell"));
        return;
    };
    read_bool_key(
        bell,
        "attention",
        "bell.attention",
        &mut config.bell.attention,
        problems,
    );
    read_bool_key(bell, "done", "bell.done", &mut config.bell.done, problems);
}

fn read_ui(table: &toml::Table, config: &mut Config, problems: &mut Vec<Problem>) {
    let Some(value) = table.get("ui") else {
        return;
    };
    let Some(ui) = value.as_table() else {
        problems.push(not_a_table_problem("ui"));
        return;
    };

    if let Some(value) = ui.get("sidebar_width") {
        match parse_sidebar_width(value) {
            Some(n) => config.ui.sidebar_width = Some(n),
            None => problems.push(Problem {
                key: "ui.sidebar_width".to_string(),
                message: "must be between 24 and 60".to_string(),
                default: sidebar_width_default_text(config.ui.sidebar_width),
            }),
        }
    }

    if let Some(value) = ui.get("tree_keep_finished_secs") {
        match value
            .as_integer()
            .and_then(|n| u64::try_from(n).ok())
            .filter(|n| *n <= 300)
        {
            Some(n) => config.ui.tree_keep_finished_secs = n,
            None => problems.push(Problem {
                key: "ui.tree_keep_finished_secs".to_string(),
                message: "must be between 0 and 300".to_string(),
                default: config.ui.tree_keep_finished_secs.to_string(),
            }),
        }
    }
}

fn read_panes(table: &toml::Table, config: &mut Config, problems: &mut Vec<Problem>) {
    let Some(value) = table.get("panes") else {
        return;
    };
    let Some(panes) = value.as_table() else {
        problems.push(not_a_table_problem("panes"));
        return;
    };

    if let Some(value) = panes.get("max") {
        match value
            .as_integer()
            .and_then(|n| u8::try_from(n).ok())
            .filter(|n| (1..=16).contains(n))
        {
            Some(n) => config.panes.max = n,
            None => problems.push(Problem {
                key: "panes.max".to_string(),
                message: "must be between 1 and 16".to_string(),
                default: config.panes.max.to_string(),
            }),
        }
    }
}

/// A leading `~/` is expanded against `$HOME` when the config is read
/// (decision 6). A bare name, or a value that is not `~/...`, is left as-is.
fn expand_home(command: &str) -> String {
    if let Some(rest) = command.strip_prefix("~/")
        && let Ok(home) = std::env::var("HOME")
    {
        return format!("{home}/{rest}");
    }
    command.to_string()
}

fn read_runtime_command(
    table: &toml::Table,
    full_key: &str,
    field: &mut String,
    problems: &mut Vec<Problem>,
) {
    let Some(value) = table.get("command") else {
        return;
    };
    match value.as_str() {
        None => problems.push(Problem {
            key: full_key.to_string(),
            message: "expected a string".to_string(),
            default: field.clone(),
        }),
        Some("") => problems.push(Problem {
            key: full_key.to_string(),
            message: "must not be empty".to_string(),
            default: field.clone(),
        }),
        Some(s) => *field = expand_home(s),
    }
}

fn read_runtimes(table: &toml::Table, config: &mut Config, problems: &mut Vec<Problem>) {
    let Some(value) = table.get("runtimes") else {
        return;
    };
    let Some(runtimes) = value.as_table() else {
        problems.push(not_a_table_problem("runtimes"));
        return;
    };

    if let Some(value) = runtimes.get("claude") {
        match value.as_table() {
            Some(claude) => read_runtime_command(
                claude,
                "runtimes.claude.command",
                &mut config.runtimes.claude.command,
                problems,
            ),
            None => problems.push(not_a_table_problem("runtimes.claude")),
        }
    }

    if let Some(value) = runtimes.get("codex") {
        match value.as_table() {
            Some(codex) => {
                read_runtime_command(
                    codex,
                    "runtimes.codex.command",
                    &mut config.runtimes.codex.command,
                    problems,
                );
                read_bool_key(
                    codex,
                    "bypass_hook_trust",
                    "runtimes.codex.bypass_hook_trust",
                    &mut config.runtimes.codex_bypass_hook_trust,
                    problems,
                );
            }
            None => problems.push(not_a_table_problem("runtimes.codex")),
        }
    }
}

fn read_u64_in_range(
    table: &toml::Table,
    local_key: &str,
    full_key: &str,
    range: &std::ops::RangeInclusive<u64>,
    field: &mut u64,
    problems: &mut Vec<Problem>,
) {
    let Some(value) = table.get(local_key) else {
        return;
    };
    match value
        .as_integer()
        .and_then(|n| u64::try_from(n).ok())
        .filter(|n| range.contains(n))
    {
        Some(n) => *field = n,
        None => problems.push(Problem {
            key: full_key.to_string(),
            message: format!("must be between {} and {}", range.start(), range.end()),
            default: field.to_string(),
        }),
    }
}

/// One `git.ignore` entry's rule: a single path component, so it can be compared against
/// a path's components the way `DENY_COMPONENTS` is. Returns why it was rejected.
fn ignore_entry_problem(entry: &str) -> Option<&'static str> {
    if entry.is_empty() {
        return Some("must not be empty");
    }
    if entry.chars().count() > GIT_IGNORE_MAX_CHARS {
        return Some("must be at most 64 characters");
    }
    if entry.contains('/') {
        return Some("must be a single path component, with no /");
    }
    if entry == "." || entry == ".." {
        return Some("must not be . or ..");
    }
    None
}

/// Reads `git.ignore`. A bad entry costs that entry alone, never the whole list and
/// never the file: the list is a filter, and dropping every name because one was
/// mistyped would silently widen what the watcher accepts.
fn read_ignore(table: &toml::Table, config: &mut Config, problems: &mut Vec<Problem>) {
    let Some(value) = table.get("ignore") else {
        return;
    };
    let Some(array) = value.as_array() else {
        problems.push(Problem {
            key: "git.ignore".to_string(),
            message: "expected an array of strings".to_string(),
            default: "no extra ignores".to_string(),
        });
        return;
    };

    let mut ignore = Vec::new();
    for entry in array {
        if ignore.len() == GIT_IGNORE_MAX_ENTRIES {
            problems.push(Problem {
                key: "git.ignore".to_string(),
                message: format!("at most {GIT_IGNORE_MAX_ENTRIES} entries; the rest are ignored"),
                default: format!("the first {GIT_IGNORE_MAX_ENTRIES}"),
            });
            break;
        }
        let Some(text) = entry.as_str() else {
            problems.push(Problem {
                key: "git.ignore".to_string(),
                message: "expected a string".to_string(),
                default: "entry dropped".to_string(),
            });
            continue;
        };
        match ignore_entry_problem(text) {
            Some(message) => problems.push(Problem {
                key: "git.ignore".to_string(),
                message: format!("{text:?}: {message}"),
                default: "entry dropped".to_string(),
            }),
            None => ignore.push(text.to_string()),
        }
    }
    config.git.ignore = ignore;
}

fn read_git(table: &toml::Table, config: &mut Config, problems: &mut Vec<Problem>) {
    let Some(value) = table.get("git") else {
        return;
    };
    let Some(git) = value.as_table() else {
        problems.push(not_a_table_problem("git"));
        return;
    };

    read_bool_key(
        git,
        "enabled",
        "git.enabled",
        &mut config.git.enabled,
        problems,
    );
    read_u64_in_range(
        git,
        "poll_secs",
        "git.poll_secs",
        &GIT_POLL_SECS_RANGE,
        &mut config.git.poll_secs,
        problems,
    );
    read_u64_in_range(
        git,
        "debounce_ms",
        "git.debounce_ms",
        &GIT_DEBOUNCE_MS_RANGE,
        &mut config.git.debounce_ms,
        problems,
    );
    read_ignore(git, config, problems);
}

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

fn read_conversation(table: &toml::Table, config: &mut Config, problems: &mut Vec<Problem>) {
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

fn report_unknown_keys(table: &toml::Table, problems: &mut Vec<Problem>) {
    for (key, value) in table {
        match key.as_str() {
            "prefix" | "accent" | "default_runtime" | "scrollback_lines" | "sidebar_width"
            | "orchestrator" => {}
            "bell" => report_unknown_nested(value, "bell", KNOWN_BELL_KEYS, problems),
            "ui" => report_unknown_nested(value, "ui", KNOWN_UI_KEYS, problems),
            "panes" => report_unknown_nested(value, "panes", KNOWN_PANES_KEYS, problems),
            "git" => report_unknown_nested(value, "git", KNOWN_GIT_KEYS, problems),
            "runtimes" => report_unknown_runtimes(value, problems),
            "conversation" => report_unknown_conversation(value, problems),
            other => problems.push(unknown_key_problem(other)),
        }
    }
}

fn report_unknown_nested(
    value: &toml::Value,
    prefix: &str,
    known: &[&str],
    problems: &mut Vec<Problem>,
) {
    let Some(table) = value.as_table() else {
        // Reported already, as a wrong-type Problem, by the corresponding
        // `read_*` function; don't also report every key inside as unknown.
        return;
    };
    for key in table.keys() {
        if !known.contains(&key.as_str()) {
            problems.push(unknown_key_problem(&format!("{prefix}.{key}")));
        }
    }
}

fn report_unknown_runtimes(value: &toml::Value, problems: &mut Vec<Problem>) {
    let Some(table) = value.as_table() else {
        return;
    };
    for (key, sub) in table {
        match key.as_str() {
            "claude" => {
                report_unknown_nested(sub, "runtimes.claude", KNOWN_RUNTIME_COMMAND_KEYS, problems)
            }
            "codex" => {
                report_unknown_nested(sub, "runtimes.codex", KNOWN_RUNTIME_CODEX_KEYS, problems)
            }
            other => problems.push(unknown_key_problem(&format!("runtimes.{other}"))),
        }
    }
}

/// Mirrors `report_unknown_runtimes`'s shape, but one level deeper: `[conversation]` has
/// its own unknown keys, plus `badges`, which has its own three runtime tables.
fn report_unknown_conversation(value: &toml::Value, problems: &mut Vec<Problem>) {
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

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
