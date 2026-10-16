//! Milestone 9.16 decision 23: which context window a session's fill is measured
//! against. Pure.

mod window_table {
    use proto::Runtime;

    use super::super::window_for;
    use crate::transcript::fill::Fill;

    fn claude(model: &str) -> Fill {
        Fill {
            tokens: 170_000,
            window: None,
            model: Some(model.into()),
        }
    }

    /// The learned windows a headless session reported: `claude-opus-5-5` → 200,000 and
    /// `claude-opus-5` → 180,000, matched exactly here (the canonical rules are
    /// `Quota::learned_window`'s, task 7).
    fn learned(model: &str, _route: &str) -> Option<u64> {
        match model {
            "claude-opus-5-5" => Some(200_000),
            "claude-opus-5" => Some(180_000),
            _ => None,
        }
    }

    #[test]
    fn config_override_wins() {
        let codex = Fill {
            tokens: 1,
            window: Some(258_400),
            model: None,
        };
        for (runtime, route, fill) in [
            (
                Runtime::Claude,
                "claude-opus-5-5[1m]",
                claude("claude-opus-5-5"),
            ),
            (Runtime::Claude, "claude-opus-5", claude("claude-opus-5")),
            (Runtime::Codex, "gpt-6", codex),
        ] {
            assert_eq!(
                window_for(runtime, 500_000, route, &fill, learned),
                Some(500_000),
                "{route}"
            );
        }
    }

    #[test]
    fn one_m_tag_sets_window_1_000_000() {
        let fill = claude("claude-opus-5-5");
        assert_eq!(
            window_for(Runtime::Claude, 0, "claude-opus-5-5[1m]", &fill, learned),
            Some(1_000_000)
        );
    }

    #[test]
    fn learned_context_window_overrides_default() {
        let fill = claude("claude-opus-5");
        assert_eq!(
            window_for(Runtime::Claude, 0, "claude-opus-5", &fill, learned),
            Some(180_000)
        );
    }

    #[test]
    fn default_is_200_000() {
        let none = |_: &str, _: &str| None;
        let fill = claude("claude-sonnet-5");
        assert_eq!(
            window_for(Runtime::Claude, 0, "claude-sonnet-5", &fill, none),
            Some(200_000)
        );
        // A line with no model learns nothing either.
        let bare = Fill {
            model: None,
            ..claude("x")
        };
        assert_eq!(
            window_for(Runtime::Claude, 0, "claude-opus-5", &bare, learned),
            Some(200_000)
        );
    }

    #[test]
    fn codex_window_comes_from_the_rollout() {
        let fill = Fill {
            tokens: 23_276,
            window: Some(258_400),
            model: None,
        };
        assert_eq!(
            window_for(Runtime::Codex, 0, "gpt-6", &fill, learned),
            Some(258_400)
        );
        let unknown = Fill {
            window: None,
            ..fill
        };
        assert_eq!(
            window_for(Runtime::Codex, 0, "gpt-6", &unknown, learned),
            None
        );
    }
}
