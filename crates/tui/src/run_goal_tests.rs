//! M9.15: the goal form's own rules, apart from the app (decision 44), and 9.0.6
//! decision 39's toggles and model picker.

use super::*;

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn form_with(goal: &str) -> GoalForm {
    let mut form = GoalForm::new("/r/demo".into());
    form.goal = TextArea::from_text(goal);
    form
}

fn entry(runtime: Runtime, model: &str) -> ModelEntry {
    ModelEntry {
        runtime,
        model: model.into(),
        strength: proto::Strength::Standard,
        note: String::new(),
    }
}

/// M9.0.6: with the runtime configured the picker holds `default` only, so a model can
/// no longer be named without a runtime (the old `MODEL_NEEDS_RUNTIME` is gone); a
/// choice of runtime clears the model, which named another's.
#[test]
fn a_model_belongs_to_a_runtime() {
    let mut form = form_with("add a");
    form.set_roster(vec![entry(Runtime::Claude, "claude-opus-5-5")]);
    form.focus = GoalField::Model;
    form.on_paste("gpt-5");
    assert_eq!(form.model, GoalModel::Default, "a paste has nowhere to go");
    assert_eq!(form.model_options(), ["default"]);
    form.focus = GoalField::Runtime;
    form.on_key(key(KeyCode::Right));
    assert_eq!(form.runtime, Some(Runtime::Claude));
    form.focus = GoalField::Model;
    form.on_key(key(KeyCode::Right));
    assert_eq!(form.chosen_model().as_deref(), Some("claude-opus-5-5"));
    form.focus = GoalField::Runtime;
    form.on_key(key(KeyCode::Right));
    assert_eq!(form.model, GoalModel::Default);
    assert_eq!(form.chosen_model(), None);
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
        })
    );
}

#[test]
fn a_goal_paste_keeps_its_lines_and_drops_controls() {
    let mut form = form_with("");
    form.on_paste("one\r\ntwo\x1b[31m\u{202E}");
    assert_eq!(form.goal.text(), "one\ntwo[31m");
    form.goal = TextArea::new();
    form.on_paste(&"y".repeat(TEXT_MAX_CHARS + 10));
    assert_eq!(form.goal.text().chars().count(), TEXT_MAX_CHARS);
}

#[test]
fn ctrl_j_is_a_newline_in_the_goal() {
    let mut form = form_with("a");
    form.on_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::CONTROL));
    form.on_key(key(KeyCode::Char('b')));
    assert_eq!(form.request().unwrap(), {
        let mut want = form_with("a\nb").request().unwrap();
        if let RunRequest::StartGoal { goal, .. } = &mut want {
            assert_eq!(goal, "a\nb");
        }
        want
    });
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

#[test]
fn tab_visits_the_six_fields_in_the_dialogs_order() {
    let mut form = form_with("add a");
    let mut seen = vec![form.focus];
    for _ in 0..5 {
        form.on_key(key(KeyCode::Tab));
        seen.push(form.focus);
    }
    assert_eq!(
        seen,
        [
            GoalField::Goal,
            GoalField::Runtime,
            GoalField::Model,
            GoalField::Trust,
            GoalField::Yes,
            GoalField::UnconfinedChecks,
        ]
    );
    form.on_key(key(KeyCode::Tab));
    assert_eq!(form.focus, GoalField::Goal);
    assert_eq!(
        seen.iter().map(|f| field_label(*f)).collect::<Vec<_>>(),
        [
            "goal",
            "runtime",
            "model",
            "trust",
            "approve at once",
            "unconfined checks"
        ]
    );
}
