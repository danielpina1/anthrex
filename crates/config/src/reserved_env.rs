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
/// engine command, by prefix ... Since F2 round 2 (N1) also exported shell functions
/// (`BASH_FUNC_<name>%%`), which bash imports at start-up.
pub const SCRUBBED_PREFIXES: &[&str] = &["CLAUDE_CODE_", "BASH_FUNC_"];
/// ... and by name. Since F2 round 2 (N1) also the shell start-up inlets that exist
/// only to run or reshape code at a shell's start (a user's own `ZDOTDIR`, `SHELL` or
/// `PYTHONPATH` is kept: it is their own setting, and only a profile may not set it).
pub const SCRUBBED_NAMES: &[&str] = &[
    "CLAUDECODE",
    "BASH_ENV",
    "ENV",
    "SHELLOPTS",
    "BASHOPTS",
    "PS4",
    "IFS",
    "CDPATH",
];

/// Set on every session by the launch itself, after the profile's `env`: its own
/// window id and socket (decision 26). Inherited values are scrubbed first.
pub const SESSION_IDENTITY: &[&str] = &["ANTHREX_WINDOW_ID", "ANTHREX_SOCKET"];

/// Set last by the launch for a worker and for a confined command: the task's own short
/// temporary directory (final fix batch F1d, R5).
pub const TASK_TMPDIR: &str = "TMPDIR";

/// Anthropic API credentials, removed from every session that does not authenticate
/// with them (final fix batch F2, review C minor M2): with `auth = "login"`, `claude -p`
/// would otherwise prefer a key the daemon happened to inherit over the user's login.
pub const API_CREDENTIALS: &[&str] = &["ANTHROPIC_API_KEY", "ANTHROPIC_AUTH_TOKEN"];

/// OpenAI and Codex env credentials (F2 round 2, N2). anthrex has no Codex auth setting:
/// its Codex sessions use the user's own `codex login` (`~/.codex/auth.json`), so every
/// session loses these, and a Codex session never bills an inherited key.
pub const OPENAI_CREDENTIALS: &[&str] = &["OPENAI_API_KEY", "CODEX_API_KEY", "CODEX_ACCESS_TOKEN"];

/// Reserved families, matched on the upper-cased name, with the reason a profile may
/// not set them.
/// Why a start-up inlet is reserved.
const START_UP: &str =
    "a shell, an interpreter or the agent reads it at start-up, outside the sandbox";

const RESERVED_PREFIXES: &[(&str, &str)] = &[
    ("BASH_FUNC_", START_UP),
    ("PYTHON", START_UP),
    ("PERL5", START_UP),
    ("RUBY", START_UP),
    ("NPM_CONFIG_", START_UP),
    ("XDG_", "it chooses which user settings and state load"),
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
    // F2 round 2 (N1): shell and interpreter start-up inlets. Claude Code 2.1.280 lists
    // the same names as unsandboxed-exec inlets; that list is the floor.
    ("BASH_ENV", START_UP),
    ("ENV", START_UP),
    ("ZDOTDIR", START_UP),
    ("SHELL", START_UP),
    ("SHELLOPTS", START_UP),
    ("BASHOPTS", START_UP),
    ("PS4", START_UP),
    ("IFS", START_UP),
    ("CDPATH", START_UP),
    ("BUN_OPTIONS", START_UP),
    ("NODE_PATH", START_UP),
    ("JAVA_TOOL_OPTIONS", START_UP),
    ("_JAVA_OPTIONS", START_UP),
    ("EDITOR", START_UP),
    ("VISUAL", START_UP),
    ("PAGER", START_UP),
    ("LESSOPEN", START_UP),
    ("LESSCLOSE", START_UP),
    ("SSH_ASKPASS", START_UP),
    ("SUDO_ASKPASS", START_UP),
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

/// Names inside a reserved family that take a plain value, never a path, a module or a
/// command, so a profile may set them (final fix batch F4, the F2 re-review's M3):
/// Python's common determinism switches. `PYTHONWARNINGS` is not among them (a warning
/// category names a module Python imports).
const ALLOWED_IN_FAMILY: &[&str] = &[
    "PYTHONUNBUFFERED",
    "PYTHONDONTWRITEBYTECODE",
    "PYTHONHASHSEED",
    "PYTHONUTF8",
    "PYTHONIOENCODING",
];

/// Why a profile's `env` may not set `key`, or `None` when it may. Case-insensitive:
/// several programs read a lower-case proxy variable too.
pub fn reserved_env(key: &str) -> Option<&'static str> {
    let upper = key.to_ascii_uppercase();
    if ALLOWED_IN_FAMILY.contains(&upper.as_str()) {
        return None;
    }
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
            .chain(API_CREDENTIALS)
            .chain(OPENAI_CREDENTIALS);
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
            // F2 round 2 (N1): shell and interpreter start-up inlets, which the agent
            // CLIs' own shells and start-up commands read outside the sandbox.
            "BASH_ENV",
            "ENV",
            "ZDOTDIR",
            "SHELL",
            "SHELLOPTS",
            "BASHOPTS",
            "PS4",
            "IFS",
            "CDPATH",
            "BASH_FUNC_x%%",
            "BUN_OPTIONS",
            "NODE_PATH",
            "PYTHONSTARTUP",
            "PYTHONPATH",
            "PYTHONHOME",
            "RUBYOPT",
            "RUBYLIB",
            "PERL5OPT",
            "PERL5LIB",
            "XDG_DATA_HOME",
            "XDG_STATE_HOME",
            "EDITOR",
            "VISUAL",
            "PAGER",
            "LESSOPEN",
            "GIT_EDITOR",
            "JAVA_TOOL_OPTIONS",
            "npm_config_script_shell",
            "SSH_ASKPASS",
        ] {
            assert!(reserved_env(key).is_some(), "{key} is not reserved");
        }
        for key in [
            "CARGO_TARGET_DIR",
            "RUST_LOG",
            "NODE_ENV",
            "PATHS",
            "MYHOME",
            // F4 (the F2 re-review's M3): value-only determinism switches.
            "PYTHONUNBUFFERED",
            "PYTHONDONTWRITEBYTECODE",
            "PYTHONHASHSEED",
            "PYTHONUTF8",
            "PYTHONIOENCODING",
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
