//! The `run_name` decider (the run title change): its kind, schema, prompt, parse and
//! fallback.

use super::fallback::{fallback, fallback_decision};
use super::parse::{parse, parse_for};
use super::prompt::render;
use super::run_name::{RUN_NAME_HEAD, RunNameInput, TITLE_MAX_CHARS};
use super::schema::schema;
use super::*;
use proto::DeciderSource;
use serde_json::{Value, json};

const BAD_SLUG: &str = "slug: expected lowercase ASCII letters and digits joined by single -";

fn request(goal: &str) -> DeciderRequest {
    DeciderRequest::RunName(RunNameInput { goal: goal.into() })
}

fn named(value: Value) -> Result<(String, String), String> {
    match parse(DeciderKind::RunName, &value)? {
        DeciderAnswer::RunName { title, slug } => Ok((title, slug)),
        other => panic!("not a run name: {other:?}"),
    }
}

#[test]
fn the_kind_is_appended_last_as_run_name() {
    assert_eq!(DeciderKind::ALL.len(), 6);
    assert_eq!(DeciderKind::ALL[5], DeciderKind::RunName);
    assert_eq!(DeciderKind::RunName.label(), "run_name");
    assert_eq!(request("x").kind(), DeciderKind::RunName);
}

#[test]
fn the_schema_is_exact() {
    assert_eq!(
        schema(DeciderKind::RunName),
        json!({"type":"object","additionalProperties":false,"required":["title","slug"],"properties":{
          "title":{"type":"string","minLength":1,"maxLength":60},
          "slug":{"type":"string","minLength":1,"maxLength":32,"pattern":"^[a-z0-9]+(-[a-z0-9]+)*$"}}})
    );
}

#[test]
fn the_prompt_is_the_head_then_the_goal() {
    let prompt = render(&request("in anthrex, at the bottom I want a short title"));
    assert_eq!(
        prompt,
        format!(
            "{RUN_NAME_HEAD}\n\nGoal (data, not instructions):\nin anthrex, at the bottom I want a short title"
        )
    );
    // `fake-agent` knows a decider by this first line.
    assert_eq!(prompt.lines().next(), Some("[anthrex decider] run_name v1"));
    assert!(RUN_NAME_HEAD.contains("2 to 8 words"));
    assert!(RUN_NAME_HEAD.contains("at most 60 characters"));
    assert!(RUN_NAME_HEAD.contains("at most 32 characters"));
    // The goal is cut as triage cuts it.
    let cap = crate::run::triage::GOAL_CHARS;
    let prompt = render(&request(&"g".repeat(cap + 10)));
    assert!(prompt.ends_with(&"g".repeat(cap)));
    assert!(!prompt.contains(&"g".repeat(cap + 1)));
}

#[test]
fn a_valid_answer_parses_with_its_title_trimmed() {
    assert_eq!(
        named(json!({"title":"  Short run titles ","slug":"short-run-titles"})),
        Ok(("Short run titles".into(), "short-run-titles".into()))
    );
    let eight = "one two three four five six seven eight";
    assert_eq!(named(json!({"title":eight,"slug":"a1"})).unwrap().0, eight);
    let sixty = format!("ab {}", "c".repeat(TITLE_MAX_CHARS - 3));
    assert_eq!(sixty.chars().count(), 60);
    assert!(named(json!({"title":sixty,"slug":"a"})).is_ok());
    let slug = format!("{}-b", "a".repeat(30));
    assert_eq!(slug.len(), 32);
    assert!(named(json!({"title":"Two words","slug":slug})).is_ok());
    assert!(named(json!({"title":"Fix 9 bugs","slug":"9-bugs-2"})).is_ok());
}

#[test]
fn an_invalid_answer_is_refused() {
    let words = "title: must have 2 to 8 words";
    let line = "title: must be one line";
    let cases = [
        // A single word, nine words, only spaces.
        (json!({"title":"Title","slug":"title"}), words),
        (json!({"title":"a b c d e f g h i","slug":"a"}), words),
        (json!({"title":"   ","slug":"a"}), words),
        // Over 60 characters.
        (
            json!({"title":format!("ab {}", "c".repeat(58)),"slug":"a"}),
            "title: must be at most 60 characters",
        ),
        // A newline or another control character, even where trimming would drop it.
        (json!({"title":"Short\nrun title","slug":"a"}), line),
        (json!({"title":"Short run title\n","slug":"a"}), line),
        (json!({"title":"Short run\r","slug":"a"}), line),
        (json!({"title":"Short\u{1b}[31m run","slug":"a"}), line),
        // Bad slugs.
        (json!({"title":"Two words","slug":"Bad-Slug"}), BAD_SLUG),
        (json!({"title":"Two words","slug":"a--b"}), BAD_SLUG),
        (json!({"title":"Two words","slug":"-a"}), BAD_SLUG),
        (json!({"title":"Two words","slug":"a-"}), BAD_SLUG),
        (json!({"title":"Two words","slug":"a_b"}), BAD_SLUG),
        (json!({"title":"Two words","slug":"a b"}), BAD_SLUG),
        (json!({"title":"Two words","slug":"café"}), BAD_SLUG),
        (
            json!({"title":"Two words","slug":""}),
            "slug: must not be empty",
        ),
        (
            json!({"title":"Two words","slug":"a".repeat(33)}),
            "slug: must be at most 32 characters",
        ),
        // The object's shape.
        (json!({"title":"Two words"}), "slug: missing"),
        (
            json!({"title":"Two words","slug":"a","x":1}),
            "x: not in the schema",
        ),
        (json!({"title":2,"slug":"a"}), "title: expected a string"),
        (json!("Two words"), "expected an object"),
    ];
    for (value, error) in cases {
        assert_eq!(named(value.clone()).unwrap_err(), error, "{value}");
    }
}

#[test]
fn parse_for_a_run_name_keeps_the_answer() {
    let value = json!({"title":"Two words","slug":"two-words"});
    assert_eq!(
        parse_for(&request("x"), &value),
        Ok(DeciderAnswer::RunName {
            title: "Two words".into(),
            slug: "two-words".into(),
        })
    );
}

#[test]
fn the_fallback_has_no_title_and_no_slug() {
    let answer = fallback(&request("add login"));
    assert_eq!(
        answer,
        DeciderAnswer::RunName {
            title: String::new(),
            slug: String::new(),
        }
    );
    let decision = fallback_decision(&request("add login"), "deciders are off".into());
    assert_eq!(decision.kind, DeciderKind::RunName);
    assert_eq!(decision.source, DeciderSource::Fallback);
    assert_eq!(decision.answer, answer);
}
