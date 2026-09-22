use super::*;

#[derive(Debug, Clone)]
pub struct ManagerConfig {
    pub socket_path: PathBuf,
    pub shell: String,
    pub exe: PathBuf,
    pub claude_bin: String,
    pub codex_bin: String,
    pub codex_hook_source: Option<String>,
    /// `runtimes.codex.bypass_hook_trust` (config decision 4), copied into every window's
    /// `LaunchContext`. A changed config needs a daemon restart to take effect.
    pub codex_bypass_hook_trust: bool,
    /// `<data_dir>/worktrees`, the `worktrees_root` every window's linked worktree is
    /// laid out under. `new` and `from_vars` pick a temporary directory so a manager
    /// built without a data directory — every test that does not exercise worktrees —
    /// still has somewhere harmless to point; `lifecycle::run` overrides it.
    pub worktrees_root: PathBuf,
    /// `worktree::OPERATION_TIMEOUT`, injected here rather than read from the constant
    /// directly, exactly like `worktrees_root` above: production always gets the real
    /// value (`new`, `from_vars`), and a test that wants to drive `spawn_window`'s create
    /// path to its deadline without waiting out the real 30 s sets this field instead
    /// (fix wave C item 7 — the cheap version of
    /// `new_worktree_waits_out_the_daemons_whole_create_budget`).
    pub operation_timeout: Duration,
    /// `worktree::CLEANUP_TIMEOUT`, `operation_timeout`'s companion: the other term the
    /// create path's worst-case budget is built from, for the `git worktree add` failure
    /// or timeout that follows.
    pub cleanup_timeout: Duration,
    /// `crate::process::KILL_GRACE`, injected here for the same reason as
    /// `operation_timeout` above: production always gets the real value (`new`,
    /// `from_vars`), and `remove_with_worktree`'s `kill_and_await_exit` reads this field
    /// rather than the constant directly, so a test can widen it far past its own removal
    /// bound instead of racing the two. See `an_exited_window_is_removed_without_waiting`
    /// (`daemon/tests/manager_worktree/removal/ordering.rs`), which sets this to 60 s and
    /// asserts the removal finishes in under 10 s — a 6x separation between "waited" and
    /// "didn't wait" instead of the ~1x a shared 1 s bound gave it.
    pub kill_grace: Duration,
    /// `restart`'s own `wait_for_exit` deadline (design decision 18): how long phase B
    /// waits for a killed child to be confirmed gone before refusing the restart.
    /// Production sets this to `crate::process::KILL_GRACE + 2s` — comfortably past the
    /// real escalation's own worst case, so a genuine child cannot cause a false timeout
    /// (`wait_for_exit`'s own doc comment).
    ///
    /// Fix wave 8, Minor: kept as its own field rather than derived from `kill_grace` at
    /// the call site, the shape it had before this fix. That shape gave
    /// `restart`'s timeout test only a 900ms margin between an injected `kill_grace` and
    /// the real, hardcoded `crate::process::KILL_GRACE` its own child's death actually
    /// depends on — an injected-versus-hardcoded near-equality, the exact shape
    /// `docs/timing-budgets.md`'s standing rule 1 warns about, even though the margin
    /// happened to be provably positive by construction. A test that wants a genuine
    /// timeout can now shrink this field alone, to any margin it likes, without touching
    /// `kill_grace` — which the real child's own escalation still runs against, unaffected
    /// and unconfigurable — instead of racing two constants that merely happened not to
    /// coincide.
    pub restart_wait_deadline: Duration,
    /// The gate both launch paths — [`super::WindowManager::create`]'s phase B and
    /// [`super::WindowManager::restart`]'s phase C — wait on before they spawn anything.
    ///
    /// Defaults to an already-open gate ([`launch::LaunchGate::open_already`]), so every
    /// manager built anywhere but `lifecycle::run` launches with no gate at all; only
    /// `lifecycle::run` replaces it with a closed one and hands the Codex version probe
    /// the job of opening it. See `crate::launch::gate`'s module doc for why the probe
    /// gates launches specifically rather than everything `server::serve` does.
    pub launch_gate: launch::LaunchGate,
}

impl ManagerConfig {
    pub fn new(socket_path: PathBuf, shell: String) -> Self {
        Self {
            socket_path,
            shell,
            exe: PathBuf::from("anthrex"),
            claude_bin: "claude".to_string(),
            codex_bin: "codex".to_string(),
            codex_hook_source: None,
            codex_bypass_hook_trust: false,
            worktrees_root: std::env::temp_dir().join("anthrex-worktrees"),
            operation_timeout: worktree::OPERATION_TIMEOUT,
            cleanup_timeout: worktree::CLEANUP_TIMEOUT,
            kill_grace: crate::process::KILL_GRACE,
            restart_wait_deadline: crate::process::KILL_GRACE + Duration::from_secs(2),
            launch_gate: launch::LaunchGate::open_already(),
        }
    }

    /// Applies config decision 6's precedence, highest first: an `ANTHREX_*_BIN`
    /// environment variable, then `runtimes.*.command`, then the built-in default. Each
    /// runtime's command is resolved independently, so an override for one never affects
    /// the other.
    pub fn from_vars(
        socket_path: PathBuf,
        shell: String,
        exe: PathBuf,
        var: impl Fn(&str) -> Option<String>,
        runtimes: &::config::Runtimes,
    ) -> Self {
        let nonempty = |key| var(key).filter(|value| !value.is_empty());
        Self {
            socket_path,
            shell,
            exe,
            claude_bin: nonempty("ANTHREX_CLAUDE_BIN")
                .unwrap_or_else(|| runtimes.claude.command.clone()),
            codex_bin: nonempty("ANTHREX_CODEX_BIN")
                .unwrap_or_else(|| runtimes.codex.command.clone()),
            codex_hook_source: None,
            codex_bypass_hook_trust: runtimes.codex_bypass_hook_trust,
            worktrees_root: std::env::temp_dir().join("anthrex-worktrees"),
            operation_timeout: worktree::OPERATION_TIMEOUT,
            cleanup_timeout: worktree::CLEANUP_TIMEOUT,
            kill_grace: crate::process::KILL_GRACE,
            restart_wait_deadline: crate::process::KILL_GRACE + Duration::from_secs(2),
            launch_gate: launch::LaunchGate::open_already(),
        }
    }

    pub fn from_env(
        socket_path: PathBuf,
        shell: String,
        runtimes: &::config::Runtimes,
    ) -> anyhow::Result<Self> {
        let exe = std::env::current_exe()?;
        let mut config = Self::from_vars(
            socket_path,
            shell,
            exe,
            |key| std::env::var(key).ok(),
            runtimes,
        );
        config.codex_hook_source = launch::codex::default_hook_source();
        Ok(config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn bin_overrides_come_from_the_environment_variables() {
        let vars = HashMap::from([
            ("ANTHREX_CLAUDE_BIN", "/opt/agents/claude"),
            ("ANTHREX_CODEX_BIN", "/opt/agents/codex"),
        ]);
        let config = ManagerConfig::from_vars(
            "/tmp/a.sock".into(),
            "/bin/zsh".into(),
            "/opt/anthrex/bin/anthrex".into(),
            |key| vars.get(key).map(|value| (*value).to_string()),
            &::config::Config::default().runtimes,
        );
        assert_eq!(config.claude_bin, "/opt/agents/claude");
        assert_eq!(config.codex_bin, "/opt/agents/codex");

        let defaults = ManagerConfig::from_vars(
            "/tmp/a.sock".into(),
            "/bin/zsh".into(),
            "/opt/anthrex/bin/anthrex".into(),
            |key| (key == "ANTHREX_CLAUDE_BIN").then(String::new),
            &::config::Config::default().runtimes,
        );
        assert_eq!(defaults.claude_bin, "claude");
        assert_eq!(defaults.codex_bin, "codex");
    }

    /// Config decision 6's precedence, highest first: an `ANTHREX_*_BIN` environment
    /// variable, then `runtimes.*.command`, then the built-in default. An override for
    /// one runtime never leaks onto the other, and `codex_bypass_hook_trust` is copied
    /// straight from the config (decision 4).
    #[test]
    fn commands_resolve_env_over_config_over_default() {
        let runtimes = ::config::Runtimes {
            claude: ::config::RuntimeCommand {
                command: "/opt/config/claude".to_string(),
            },
            codex: ::config::RuntimeCommand {
                command: "/opt/config/codex".to_string(),
            },
            codex_bypass_hook_trust: true,
        };

        // Nothing set: config wins over the default.
        let from_config = ManagerConfig::from_vars(
            "/tmp/a.sock".into(),
            "/bin/zsh".into(),
            "/opt/anthrex/bin/anthrex".into(),
            |_| None,
            &runtimes,
        );
        assert_eq!(from_config.claude_bin, "/opt/config/claude");
        assert_eq!(from_config.codex_bin, "/opt/config/codex");
        assert!(from_config.codex_bypass_hook_trust);

        // ANTHREX_CLAUDE_BIN wins over config, for Claude only.
        let vars = HashMap::from([("ANTHREX_CLAUDE_BIN", "/opt/env/claude")]);
        let mixed = ManagerConfig::from_vars(
            "/tmp/a.sock".into(),
            "/bin/zsh".into(),
            "/opt/anthrex/bin/anthrex".into(),
            |key| vars.get(key).map(|value| (*value).to_string()),
            &runtimes,
        );
        assert_eq!(mixed.claude_bin, "/opt/env/claude");
        assert_eq!(mixed.codex_bin, "/opt/config/codex");

        // ANTHREX_CODEX_BIN wins over config, for Codex only — the mirror image of the
        // Claude-only case above. Coverage gap flagged by the previous task's review:
        // without this case, a `from_vars` that shared one fallback expression between
        // `claude_bin` and `codex_bin` (reading only `ANTHREX_CLAUDE_BIN`, say) would
        // still pass every test in this file, because nothing exercised "Codex overridden
        // while config also differs from default" on its own.
        let vars = HashMap::from([("ANTHREX_CODEX_BIN", "/opt/env/codex")]);
        let mixed_codex = ManagerConfig::from_vars(
            "/tmp/a.sock".into(),
            "/bin/zsh".into(),
            "/opt/anthrex/bin/anthrex".into(),
            |key| vars.get(key).map(|value| (*value).to_string()),
            &runtimes,
        );
        assert_eq!(mixed_codex.claude_bin, "/opt/config/claude");
        assert_eq!(mixed_codex.codex_bin, "/opt/env/codex");

        // Nothing set, no config either: the built-in default.
        let defaults = ManagerConfig::from_vars(
            "/tmp/a.sock".into(),
            "/bin/zsh".into(),
            "/opt/anthrex/bin/anthrex".into(),
            |_| None,
            &::config::Config::default().runtimes,
        );
        assert_eq!(defaults.claude_bin, "claude");
        assert_eq!(defaults.codex_bin, "codex");
        assert!(!defaults.codex_bypass_hook_trust);
    }
}
