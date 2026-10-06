//! The run title change, `build.rs`'s side: the id comes from the model's slug, today's
//! id from the goal on any fallback, and decision 15's redraw loop applies to either.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use proto::{DeciderSource, TokenUsage};

use super::RunName;
use crate::decider::fallback::fallback_decision;
use crate::decider::{DeciderAnswer, DeciderKind, DeciderRequest, Decision, RunNameInput};
use crate::manager::{GitRoots, ManagerConfig, WindowManager};
use crate::run::driver::RunService;
use crate::run::plan::slug;

const GOAL: &str = "in anthrex, at the bottom I want a short title";

fn request() -> DeciderRequest {
    DeciderRequest::RunName(RunNameInput { goal: GOAL.into() })
}

fn answered(title: &str, slug: &str) -> Decision {
    Decision {
        kind: DeciderKind::RunName,
        answer: DeciderAnswer::RunName {
            title: title.into(),
            slug: slug.into(),
        },
        source: DeciderSource::Decider,
        fallback_reason: None,
        usage: Some(TokenUsage {
            input: 120,
            output: 9,
            ..TokenUsage::default()
        }),
        secs: 1,
    }
}

/// The id `pick_id` would slug for `named`, with a fixed suffix.
fn id_of(named: &RunName) -> String {
    slug(named.id_head(GOAL), 0x0a1b)
}

#[test]
fn the_id_comes_from_the_model_slug() {
    let named = RunName::from_decision(&answered("Short run titles", "short-run-titles"));
    assert_eq!(named.title, "Short run titles");
    assert_eq!(named.id_head(GOAL), "short-run-titles");
    assert_eq!(id_of(&named), "short-run-titles-0a1b");
    assert_eq!(named.usage.map(|u| u.input), Some(120));
}

#[test]
fn a_failure_or_a_timeout_keeps_todays_id_and_no_title() {
    let today = slug(GOAL, 0x0a1b);
    assert_eq!(today, "in-anthrex-at-the-bottom-i-want-0a1b");
    for reason in [
        "the decider exited before answering (code 2)",
        "the decider timed out after 15 s",
        "the decider's answer does not match the schema: title: must have 2 to 8 words",
        "deciders are off",
    ] {
        let named = RunName::from_decision(&fallback_decision(&request(), reason.into()));
        assert_eq!(named.title, "", "{reason}");
        assert_eq!(named.id_head(GOAL), GOAL, "{reason}");
        assert_eq!(id_of(&named), today, "{reason}");
    }
    // A fallback's turn usage is still metered.
    let mut fell_back = fallback_decision(&request(), "the decider's turn failed: x".into());
    fell_back.usage = Some(TokenUsage {
        input: 5,
        ..TokenUsage::default()
    });
    assert_eq!(
        RunName::from_decision(&fell_back).usage.map(|u| u.input),
        Some(5)
    );
    // An answer the decider gave under a fallback source is never used.
    let mut odd = answered("Short run titles", "short-run-titles");
    odd.source = DeciderSource::Fallback;
    assert_eq!(RunName::from_decision(&odd).id_head(GOAL), GOAL);
}

#[test]
fn the_redraw_loop_applies_to_a_model_slug() {
    struct NoRoots;
    impl GitRoots for NoRoots {
        fn register(&self, _: PathBuf) {}
        fn unregister(&self, _: &Path) {}
    }
    let dir = tempfile::tempdir().unwrap();
    let config = ManagerConfig::for_tests(dir.path().join("d.sock"), "/bin/sh".into());
    let (manager, _events) = WindowManager::new(config);
    let service = RunService::for_manager(&manager, dir.path().join("data"), Arc::new(NoRoots));
    let named = RunName::from_decision(&answered("Short run titles", "short-run-titles"));
    let id = service.pick_id(named.id_head(GOAL), &[]).unwrap();
    assert!(id.starts_with("short-run-titles-"), "{id}");
    assert_eq!(id.len(), "short-run-titles-".len() + 4);

    // Every suffix of the slug's id taken by a branch: no free id, as for a goal's.
    let refs: Vec<String> = (0..=u16::MAX)
        .map(|n| format!("refs/heads/anthrex/short-run-titles-{n:04x}"))
        .collect();
    assert_eq!(
        service.pick_id(named.id_head(GOAL), &refs),
        Err("could not pick a free run id".to_string())
    );
    // The goal's own ids are still free.
    assert!(service.pick_id(GOAL, &refs).is_ok());
}
