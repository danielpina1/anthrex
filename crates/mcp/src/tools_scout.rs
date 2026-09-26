//! The scout's one tool, `submit_scout_report` (milestone 8b decisions 13 and 15). Its
//! schema is a closed object at every level and has no property for a confinement
//! setting: those stay in the user's own config (decision 5). The daemon validates the
//! arguments again (`daemon::scout::report::validate`).

use rmcp::model::Tool;
use serde_json::{Value, json};

use crate::tools::{closed, one_of, text};

pub const SUBMIT_SCOUT_REPORT: &str = "submit_scout_report";

/// The pattern every `profile.env` key matches.
pub const ENV_KEY_PATTERN: &str = "^[A-Za-z_][A-Za-z0-9_]*$";

/// `submit_scout_report`, the only tool of `AgentRole::Scout`.
pub fn submit_scout_report() -> Tool {
    let file = closed(
        json!({"path": text(500), "why": text(300)}),
        &["path", "why"],
    );
    Tool::new(
        SUBMIT_SCOUT_REPORT,
        "Submit your findings. Call it once, then stop.",
        closed(
            json!({
                "summary": text(8000),
                "files": {"type": "array", "maxItems": 60, "items": file},
                "modules": list(40, 200),
                "interfaces": list(40, 500),
                "risks": list(20, 500),
                "profile": Value::Object(profile()),
            }),
            &["summary", "files"],
        ),
    )
}

fn profile() -> rmcp::model::JsonObject {
    let env = json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {},
        "maxProperties": 20,
        "patternProperties": {
            ENV_KEY_PATTERN: {"type": "string", "minLength": 0, "maxLength": 1000},
        },
    });
    closed(
        json!({
            "languages": list(10, 40),
            "modules": list(40, 300),
            "hub": list(40, 300),
            "source": list(40, 300),
            "generated": list(40, 300),
            "protected": list(40, 300),
            "setup": text(2000),
            "check": text(2000),
            "check_timeout_secs": {"type": "integer", "minimum": 10, "maximum": 14400},
            "single_test": text(1000),
            "test_passed": text(300),
            "sample_test": text(300),
            "output_filter": one_of(&["failures-only", "tail", "none"]),
            "filter_prefixes": list(10, 100),
            "conventions": list(20, 300),
            "manifests": list(50, 300),
            "env": env,
        }),
        &[],
    )
}

/// An array of at most `max` strings of 1 to `chars` characters.
fn list(max: u64, chars: u64) -> Value {
    json!({"type": "array", "maxItems": max, "items": text(chars)})
}

#[cfg(test)]
#[path = "tools_scout_tests.rs"]
mod tests;
