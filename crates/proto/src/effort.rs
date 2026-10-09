//! How much reasoning a route asks for (milestone 9.8, MR §3.2).

use std::borrow::Cow;
use std::cmp::Ordering;

use serde::{Deserialize, Serialize};

/// How much reasoning a route asks for: the model's own effort name (`low` ... `xhigh`,
/// `max`), or [`Effort::DEFAULT`] (empty) for the model's default, which passes no
/// flag. A string since milestone 9.8 (MR §3.2); a protocol-18 value decodes as is.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(transparent)]
pub struct Effort(Cow<'static, str>);

impl Effort {
    pub const DEFAULT: Effort = Effort(Cow::Borrowed(""));
    pub const LOW: Effort = Effort(Cow::Borrowed("low"));
    pub const MEDIUM: Effort = Effort(Cow::Borrowed("medium"));
    pub const HIGH: Effort = Effort(Cow::Borrowed("high"));

    /// `name` as given; callers check it with `models::valid_effort` first.
    pub fn new(name: impl Into<String>) -> Effort {
        Effort(Cow::Owned(name.into()))
    }

    /// `Some(name)` is that effort; `None` is [`Effort::DEFAULT`].
    pub fn of(name: Option<&str>) -> Effort {
        name.map_or(Effort::DEFAULT, Effort::new)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn is_default(&self) -> bool {
        self.0.is_empty()
    }

    /// The efforts escalation climbs, in order (real-CLI manual check fix, decision 4).
    /// An effort outside it (Codex's `ultra`, "automatic task delegation") is chosen by
    /// hand only.
    pub const LADDER: [&'static str; 6] = ["minimal", "low", "medium", "high", "xhigh", "max"];

    /// `name`'s place on [`Effort::LADDER`], `None` off it.
    pub fn ladder_rank(name: &str) -> Option<usize> {
        Effort::LADDER.iter().position(|e| *e == name)
    }

    /// `DEFAULT`, then the ladder, then any other name by name.
    fn rank(&self) -> (usize, &str) {
        match self.as_str() {
            "" => (0, ""),
            name => match Effort::ladder_rank(name) {
                Some(at) => (at + 1, ""),
                None => (Effort::LADDER.len() + 1, name),
            },
        }
    }
}

impl Ord for Effort {
    fn cmp(&self, other: &Self) -> Ordering {
        self.rank().cmp(&other.rank())
    }
}

impl PartialOrd for Effort {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl std::fmt::Display for Effort {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.is_default() {
            f.write_str("default")
        } else {
            f.write_str(self.as_str())
        }
    }
}
