//! Milestone 9.9 decision 9: who performs an action the user and the orchestrator share
//! (`retry`, `override`, an approved hold). Pure.

use serde::{Deserialize, Serialize};

/// The actor of a shared engine core; the user's texts are unchanged.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Actor {
    #[default]
    User,
    Orchestrator,
}

impl Actor {
    /// `the user` or `the orchestrator`, as a sentence names them.
    pub fn who(self) -> &'static str {
        match self {
            Actor::User => "the user",
            Actor::Orchestrator => "the orchestrator",
        }
    }

    /// `user` or `orchestrator`, as a record names them.
    pub fn label(self) -> &'static str {
        match self {
            Actor::User => "user",
            Actor::Orchestrator => "orchestrator",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_actor_names_itself() {
        assert_eq!(Actor::default(), Actor::User);
        assert_eq!(
            (Actor::User.who(), Actor::User.label()),
            ("the user", "user")
        );
        assert_eq!(Actor::Orchestrator.who(), "the orchestrator");
        assert_eq!(Actor::Orchestrator.label(), "orchestrator");
    }
}
