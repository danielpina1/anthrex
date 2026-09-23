//! `Edit`, `Write` and `MultiEdit` unfold as diffs (spec §6). Pure and client-side: the
//! input it reads is already on the wire, so this works while a conversation is degraded.

use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diff {
    pub path: String,
    pub hunks: Vec<Hunk>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hunk {
    pub old: String,
    pub new: String,
}

/// `Edit` → one hunk from `old_string`/`new_string`; `MultiEdit` → one per `edits[i]`;
/// `Write` → one hunk with an empty `old` and `content` as `new`. Anything else, or a
/// missing `file_path`, is `None`.
pub fn from_tool_input(name: &str, input: &Value) -> Option<Diff> {
    let path = input.get("file_path")?.as_str()?.to_owned();
    let hunks = match name {
        "Edit" => vec![edit_hunk(input)?],
        "MultiEdit" => input
            .get("edits")?
            .as_array()?
            .iter()
            .map(edit_hunk)
            .collect::<Option<Vec<_>>>()?,
        "Write" => vec![Hunk {
            old: String::new(),
            new: input.get("content")?.as_str()?.to_owned(),
        }],
        _ => return None,
    };
    Some(Diff { path, hunks })
}

/// One `{old_string, new_string}` pair; both must be strings.
fn edit_hunk(edit: &Value) -> Option<Hunk> {
    Some(Hunk {
        old: edit.get("old_string")?.as_str()?.to_owned(),
        new: edit.get("new_string")?.as_str()?.to_owned(),
    })
}

#[cfg(test)]
#[path = "diff_tests.rs"]
mod tests;
