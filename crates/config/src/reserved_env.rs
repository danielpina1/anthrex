//! Environment variables a profile's `env` may not set (M8a final fix batch F2, finding
//! C-I4), and the lists the daemon's scrubs remove from the environment it inherits.
//! Both come from here, so a variable the launch scrubs or sets on purpose can never
//! be put back by a profile.
//!
//! A profile's `env` is for per-worktree variables of the project (spec §17), such as
//! `CARGO_TARGET_DIR`. It is refused for anything that chooses which settings, config
//! or credentials an agent loads, which program runs as the agent, what is loaded into
//! it before its sandbox applies, where its traffic goes, or where git reads and writes.
//! Process-wide values such as `PATH`, `HOME` or a corporate proxy reach every session
//! and command from the daemon's own environment, unchanged.

/// AGENTS.md rule 11's git location variables. Every daemon git call and every headless
/// session drops the inherited ones.
pub const GIT_LOCATION_VARS: [&str; 5] = [
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_COMMON_DIR",
    "GIT_INDEX_FILE",
    "GIT_PREFIX",
];

/// Decision 26: inherited variables removed from every headless session and every
/// engine command, by prefix ...
pub const SCRUBBED_PREFIXES: &[&str] = &["CLAUDE_CODE_"];
/// ... and by name.
pub const SCRUBBED_NAMES: &[&str] = &["CLAUDECODE"];

/// Set on every session by the launch itself, after the profile's `env`: its own
/// window id and socket (decision 26). Inherited values are scrubbed first.
pub const SESSION_IDENTITY: &[&str] = &["ANTHREX_WINDOW_ID", "ANTHREX_SOCKET"];

/// Set last by the launch for a worker and for a confined command: the task's own short
/// temporary directory (final fix batch F1d, R5).
pub const TASK_TMPDIR: &str = "TMPDIR";

/// API credentials removed from every session that does not authenticate with them
/// (final fix batch F2, review C minor M2): with `auth = "login"`, `claude -p` would
/// otherwise prefer a key the daemon happened to inherit over the user's login.
pub const API_CREDENTIALS: &[&str] = &["ANTHROPIC_API_KEY", "ANTHROPIC_AUTH_TOKEN"];

/// Reserved families, matched on the upper-cased name, with the reason a profile may
/// not set them.
const RESERVED_PREFIXES: &[(&str, &str)] = &[
    (
        "CLAUDE",
        "it chooses which Claude Code settings, session or credentials load",
    ),
    ("CODEX_", "it chooses which Codex config or home loads"),
    (
        "ANTHROPIC_",
        "it chooses the model provider's endpoint or credentials",
    ),
    (
        "OPENAI_",
        "it chooses the model provider's endpoint or credentials",
    ),
    (
        "GIT_",
        "it redirects git (AGENTS.md rule 11) or its configuration",
    ),
    ("ANTHREX_", "anthrex sets its own variables"),
    (
        "DYLD_",
        "it loads code into the agent before its sandbox applies",
    ),
    (
        "LD_",
        "it loads code into the agent before its sandbox applies",
    ),
];

/// Reserved names, matched on the upper-cased name, with the reason.
const RESERVED_NAMES: &[(&str, &str)] = &[
    ("HOME", "it chooses which user settings load"),
    ("XDG_CONFIG_HOME", "it chooses which user settings load"),
    ("PATH", "it chooses which program runs as the agent"),
    (
        "NODE_OPTIONS",
        "it loads code into the agent before its sandbox applies",
    ),
    ("TMPDIR", "anthrex gives each task its own"),
    (
        "HTTP_PROXY",
        "it routes the agent's traffic, credentials included",
    ),
    (
        "HTTPS_PROXY",
        "it routes the agent's traffic, credentials included",
    ),
    (
        "ALL_PROXY",
        "it routes the agent's traffic, credentials included",
    ),
    (
        "NO_PROXY",
        "it routes the agent's traffic, credentials included",
    ),
    (
        "NODE_EXTRA_CA_CERTS",
        "it chooses which TLS certificates the agent trusts",
    ),
    (
        "SSL_CERT_FILE",
        "it chooses which TLS certificates the agent trusts",
    ),
    (
        "SSL_CERT_DIR",
        "it chooses which TLS certificates the agent trusts",
    ),
];

/// Why a profile's `env` may not set `key`, or `None` when it may. Case-insensitive:
/// several programs read a lower-case proxy variable too.
pub fn reserved_env(key: &str) -> Option<&'static str> {
    let upper = key.to_ascii_uppercase();
    RESERVED_NAMES
        .iter()
        .find(|(name, _)| upper == *name)
        .or_else(|| {
            RESERVED_PREFIXES
                .iter()
                .find(|(prefix, _)| upper.starts_with(prefix))
        })
        .map(|(_, reason)| *reason)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The one-list property: everything a scrub removes or the launch sets on purpose
    /// is refused in a profile.
    #[test]
    fn every_scrubbed_or_pinned_name_is_reserved() {
        let names = GIT_LOCATION_VARS
            .iter()
            .chain(SCRUBBED_NAMES)
            .chain(SESSION_IDENTITY)
            .chain(&[TASK_TMPDIR])
            .chain(API_CREDENTIALS);
        for name in names {
            assert!(reserved_env(name).is_some(), "{name} is not reserved");
        }
        for prefix in SCRUBBED_PREFIXES {
            let name = format!("{prefix}ANYTHING");
            assert!(reserved_env(&name).is_some(), "{name} is not reserved");
        }
    }

    #[test]
    fn settings_loaders_and_redirects_are_reserved() {
        for key in [
            "CLAUDE_CONFIG_DIR",
            "CODEX_HOME",
            "GIT_CONFIG_GLOBAL",
            "GIT_CONFIG_PARAMETERS",
            "GIT_SSH_COMMAND",
            "ANTHROPIC_BASE_URL",
            "OPENAI_BASE_URL",
            "ANTHREX_DATA_DIR",
            "DYLD_INSERT_LIBRARIES",
            "LD_PRELOAD",
            "PATH",
            "HOME",
            "https_proxy",
            "Tmpdir",
        ] {
            assert!(reserved_env(key).is_some(), "{key} is not reserved");
        }
        for key in [
            "CARGO_TARGET_DIR",
            "RUST_LOG",
            "NODE_ENV",
            "PATHS",
            "MYHOME",
        ] {
            assert_eq!(reserved_env(key), None, "{key}");
        }
    }

    /// The user's `[orchestrator.profile.env]` gets the same refusal at load, each
    /// reserved key a problem and dropped, so every plan the daemon builds with it is
    /// not refused in turn.
    #[test]
    fn the_users_profile_env_drops_reserved_keys_at_load() {
        let (config, problems) = crate::parse(
            r#"
[orchestrator.profile.env]
CARGO_TARGET_DIR = "{worktree}/target"
CLAUDE_CONFIG_DIR = "/x"
GIT_DIR = "/y"
"#,
        );
        let keys: Vec<&str> = problems.iter().map(|p| p.key.as_str()).collect();
        assert_eq!(
            keys,
            [
                "orchestrator.profile.env.CLAUDE_CONFIG_DIR",
                "orchestrator.profile.env.GIT_DIR"
            ]
        );
        assert!(
            problems[0].message.contains("may not be set"),
            "{}",
            problems[0]
        );
        let env = config.orchestrator.profile.env.unwrap();
        assert_eq!(env.keys().collect::<Vec<_>>(), ["CARGO_TARGET_DIR"]);
    }
}
