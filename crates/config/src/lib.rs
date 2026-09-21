//! Config parsing for anthrex: `config.toml`, read by the daemon, the client
//! and the CLI. Parsing never fails: each key is validated on its own, and an
//! invalid or unknown key produces a [`Problem`] while its value keeps the
//! built-in default. See milestone 6 decisions 1-7.

use std::path::Path;
use std::str::FromStr;

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

    report_unknown_keys(&table, &mut problems);

    (config, problems)
}

/// Load and parse the config file at `path`. A missing file is not a
/// problem: it yields defaults, silently, same as any other unreadable
/// file. A file the daemon, client or CLI cannot read must never stop them
/// from starting.
pub fn load(path: &Path) -> (Config, Vec<Problem>) {
    match std::fs::read_to_string(path) {
        Ok(text) => parse(&text),
        Err(_) => (Config::default(), Vec::new()),
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

fn report_unknown_keys(table: &toml::Table, problems: &mut Vec<Problem>) {
    for (key, value) in table {
        match key.as_str() {
            "prefix" | "accent" | "default_runtime" | "scrollback_lines" | "sidebar_width"
            | "orchestrator" => {}
            "bell" => report_unknown_nested(value, "bell", KNOWN_BELL_KEYS, problems),
            "ui" => report_unknown_nested(value, "ui", KNOWN_UI_KEYS, problems),
            "panes" => report_unknown_nested(value, "panes", KNOWN_PANES_KEYS, problems),
            "runtimes" => report_unknown_runtimes(value, problems),
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

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
