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

    fn rank(&self) -> (u8, &str) {
        match self.as_str() {
            "" => (0, ""),
            "low" => (1, ""),
            "medium" => (2, ""),
            "high" => (3, ""),
            other => (4, other),
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
