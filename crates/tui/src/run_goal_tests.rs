//! M9.15: the goal form's own rules, apart from the app (decision 44).

use super::*;

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn form_with(goal: &str) -> GoalForm {
    let mut form = GoalForm::new("/r/demo".into());
    form.goal = TextInput::new(goal);
    form
}

#[test]
fn a_model_needs_a_runtime() {
    let mut form = form_with("add a");
    form.focus = GoalField::Model;
    form.on_paste("gpt-\n5");
    assert_eq!(form.model.text(), "gpt-5");
    assert_eq!(form.on_key(key(KeyCode::Enter)), GoalOutcome::Stay);
    assert_eq!(form.error.as_deref(), Some(MODEL_NEEDS_RUNTIME));
    assert_eq!(form.focus, GoalField::Runtime);
    assert!(!form.submitting);
    // Choosing a runtime clears the model, which named another's.
    form.on_key(key(KeyCode::Right));
    assert_eq!(form.runtime, Some(Runtime::Claude));
    assert_eq!(form.model.text(), "");
}

#[test]
fn codex_with_no_model_is_its_configured_default() {
    let mut form = form_with("  add a  ");
    form.runtime = Some(Runtime::Codex);
    assert_eq!(
        form.request(),
        Ok(RunRequest::StartGoal {
            goal: "add a".into(),
            dir: "/r/demo".into(),
            yes: false,
            trust_project: false,
            unconfined_checks: false,
            orchestrator: Some(OrchestratorChoice {
                runtime: Runtime::Codex,
                model: None,
            }),
            delivery: None,
        })
    );
}

#[test]
fn a_goal_paste_keeps_its_lines_and_drops_controls() {
    let mut form = form_with("");
    form.on_paste("one\r\ntwo\x1b[31m\u{202E}");
    assert_eq!(form.goal.text(), "one↵two[31m");
    form.goal = TextInput::new("");
    form.on_paste(&"y".repeat(TEXT_MAX_CHARS + 10));
    assert_eq!(form.goal.text().chars().count(), TEXT_MAX_CHARS);
}

#[test]
fn space_toggles_trust_and_esc_cancels_even_while_submitting() {
    let mut form = form_with("add a");
    form.focus = GoalField::Trust;
    form.on_key(key(KeyCode::Char(' ')));
    assert!(form.trust_project);
    assert!(matches!(
        form.on_key(key(KeyCode::Enter)),
        GoalOutcome::Submit(_)
    ));
    assert!(form.submitting);
    assert_eq!(form.on_key(key(KeyCode::Enter)), GoalOutcome::Stay);
    assert_eq!(form.on_key(key(KeyCode::Esc)), GoalOutcome::Cancel);
}
