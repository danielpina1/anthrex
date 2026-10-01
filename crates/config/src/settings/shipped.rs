//! The models anthrex ships a row for on the Settings screen (decision 32, spec §3), one
//! `const` per runtime. Every built-in roster entry is among them
//! (`every_builtin_model_is_shipped`), so the built-in roster always shows as rows.

use proto::{Runtime, Strength};

/// One shipped model: its runtime, name, strength and the label its row shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShippedModel {
    pub runtime: Runtime,
    pub model: &'static str,
    pub strength: Strength,
    pub label: &'static str,
}

const fn shipped(
    runtime: Runtime,
    model: &'static str,
    strength: Strength,
    label: &'static str,
) -> ShippedModel {
    ShippedModel {
        runtime,
        model,
        strength,
        label,
    }
}

pub const SHIPPED_CLAUDE: [ShippedModel; 3] = [
    shipped(
        Runtime::Claude,
        "claude-haiku-4-5",
        Strength::Fast,
        "claude-haiku-4-5",
    ),
    shipped(
        Runtime::Claude,
        "claude-sonnet-5",
        Strength::Standard,
        "claude-sonnet-5",
    ),
    shipped(
        Runtime::Claude,
        "claude-opus-5-5",
        Strength::Frontier,
        "claude-opus-5-5",
    ),
];

/// The last row is model `""`: whatever Codex itself is configured to use.
pub const SHIPPED_CODEX: [ShippedModel; 8] = [
    shipped(
        Runtime::Codex,
        "gpt-6.1-sol",
        Strength::Frontier,
        "gpt-6.1-sol",
    ),
    shipped(Runtime::Codex, "gpt-6-sol", Strength::Standard, "gpt-6-sol"),
    shipped(
        Runtime::Codex,
        "gpt-6-astra",
        Strength::Standard,
        "gpt-6-astra",
    ),
    shipped(Runtime::Codex, "gpt-6-luna", Strength::Fast, "gpt-6-luna"),
    shipped(
        Runtime::Codex,
        "gpt-5.6-sol",
        Strength::Standard,
        "gpt-5.6-sol",
    ),
    shipped(
        Runtime::Codex,
        "gpt-5.6-terra",
        Strength::Standard,
        "gpt-5.6-terra",
    ),
    shipped(
        Runtime::Codex,
        "gpt-5.6-luna",
        Strength::Fast,
        "gpt-5.6-luna",
    ),
    shipped(Runtime::Codex, "", Strength::Standard, "Codex default"),
];
