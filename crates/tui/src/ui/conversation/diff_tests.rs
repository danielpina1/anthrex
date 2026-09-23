//! Task M6.5.13: which tool inputs unfold as diffs. The `Edit` fixture's two strings
//! differ in length and content, so a diff that swapped `old_string` and `new_string`
//! fails; the three `MultiEdit` edits are distinct, so a reordering fails.

use super::*;
use serde_json::json;

const OLD: &str = "fn parse_all(src: &str) {";
const NEW: &str = "fn parse_all(src: &str) -> Result<Ast> {";

fn hunk(old: &str, new: &str) -> Hunk {
    Hunk {
        old: old.into(),
        new: new.into(),
    }
}

#[test]
fn edit_write_and_multiedit_produce_diffs() {
    assert_ne!(OLD.len(), NEW.len());
    let edit = from_tool_input(
        "Edit",
        &json!({"file_path": "crates/parse/src/lib.rs", "old_string": OLD, "new_string": NEW}),
    );
    assert_eq!(
        edit,
        Some(Diff {
            path: "crates/parse/src/lib.rs".into(),
            hunks: vec![hunk(OLD, NEW)],
        })
    );

    let multi = from_tool_input(
        "MultiEdit",
        &json!({
            "file_path": "src/multi.rs",
            "edits": [
                {"old_string": "alpha one", "new_string": "ALPHA 1"},
                {"old_string": "beta", "new_string": "BETA two two"},
                {"old_string": "gamma three three three", "new_string": "g3"},
            ],
        }),
    );
    assert_eq!(
        multi,
        Some(Diff {
            path: "src/multi.rs".into(),
            hunks: vec![
                hunk("alpha one", "ALPHA 1"),
                hunk("beta", "BETA two two"),
                hunk("gamma three three three", "g3"),
            ],
        })
    );

    let write = from_tool_input(
        "Write",
        &json!({"file_path": "notes.md", "content": "a\nb"}),
    );
    assert_eq!(
        write,
        Some(Diff {
            path: "notes.md".into(),
            hunks: vec![hunk("", "a\nb")],
        })
    );
}

#[test]
fn anything_else_is_none() {
    let edit_input = json!({"file_path": "x.rs", "old_string": OLD, "new_string": NEW});
    assert_eq!(from_tool_input("Bash", &json!({"command": "ls -la"})), None);
    assert_eq!(from_tool_input("Read", &json!({"file_path": "x.rs"})), None);
    assert_eq!(from_tool_input("Grep", &edit_input), None);
    assert_eq!(
        from_tool_input("Edit", &json!({"old_string": OLD, "new_string": NEW})),
        None
    );
    assert_eq!(
        from_tool_input("Edit", &json!({"file_path": "x.rs", "old_string": OLD})),
        None
    );
    assert_eq!(from_tool_input("Edit", &json!("not an object")), None);
    assert_eq!(from_tool_input("Write", &json!([1, 2, 3])), None);
    assert_eq!(from_tool_input("MultiEdit", &json!(null)), None);
}
