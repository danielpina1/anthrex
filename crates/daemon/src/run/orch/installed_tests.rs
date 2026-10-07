use proto::models::ModelTable;
use proto::{OrchestratorChoice, Runtime};

use super::*;

/// Only `absent`'s binary is missing.
fn without(absent: &[Runtime]) -> impl Fn(Runtime) -> Option<String> {
    let absent = absent.to_vec();
    move |r: Runtime| {
        absent
            .contains(&r)
            .then(|| format!("/nonexistent/{}", r.label()))
    }
}

/// The built-in table: the orchestrator row is Claude Opus.
fn builtin() -> RunModels {
    RunModels::resolve(&ModelTable::default(), None)
}

#[test]
fn an_installed_runtime_resolves_to_the_row() {
    let missing = without(&[Runtime::Codex]);
    let got = resolve_installed(None, &builtin(), &missing);
    assert_eq!(got, Ok(orchestrator_route(None, &builtin())));
}

/// Milestone 9.8 (D2, MR §7): the row's runtime not installed refuses the start; no
/// other model is taken in its place. A chosen runtime is refused the same way.
#[test]
fn a_rows_or_a_chosen_runtime_that_is_not_installed_refuses_the_start() {
    let missing = without(&[Runtime::Claude]);
    let text = resolve_installed(None, &builtin(), &missing).expect_err("refused");
    assert_eq!(
        text,
        "the orchestrator's runtime claude is not installed (/nonexistent/claude is not an \
         executable file); install it, or choose another model for the orchestrator in C-b S, or another runtime with --orchestrator"
    );
    let missing = without(&[Runtime::Codex]);
    let choice = OrchestratorChoice {
        runtime: Runtime::Codex,
        model: None,
        effort: None,
    };
    let text = resolve_installed(Some(&choice), &builtin(), &missing).expect_err("refused");
    assert!(
        text.starts_with("the orchestrator's runtime codex is not installed (/nonexistent/codex"),
        "{text}"
    );
}

#[test]
fn not_installed_names_who_the_runtime_and_the_binary() {
    let missing = without(&[Runtime::Codex]);
    assert_eq!(
        not_installed("the sub-planners'", Runtime::Claude, &missing, "x"),
        None
    );
    assert_eq!(
        not_installed(
            "the sub-planners'",
            Runtime::Codex,
            &missing,
            "choose another model for the planner in C-b S"
        )
        .as_deref(),
        Some(
            "the sub-planners' runtime codex is not installed (/nonexistent/codex is not an \
             executable file); install it, or choose another model for the planner in C-b S"
        )
    );
}
