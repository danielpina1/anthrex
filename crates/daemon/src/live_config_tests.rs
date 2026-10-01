//! Decision 29: a swap replaces only the settings-owned fields, and a value taken
//! before a swap is a snapshot.

use std::collections::BTreeMap;

use proto::Origin;

use super::LiveSettings;

fn parsed(text: &str) -> config::Orchestrator {
    let (config, problems) = config::parse(text);
    assert!(problems.is_empty(), "{problems:?}");
    config.orchestrator
}

#[test]
fn live_swap_replaces_only_owned_fields() {
    let live = LiveSettings::defaults_of(parsed(
        "[orchestrator]\nworker_sandbox = false\nmax_writers = 3\n",
    ));
    let reparsed = parsed("[orchestrator]\nworker_sandbox = true\nmax_writers = 1\n");
    let origin: BTreeMap<String, Origin> =
        BTreeMap::from([("orchestrator.max_writers".to_string(), Origin::File)]);
    live.swap_owned(&reparsed, origin.clone());
    let now = live.current();
    assert_eq!(now.orchestrator.max_writers, 1);
    assert!(
        !now.orchestrator.worker_sandbox,
        "a key Settings does not own stays as loaded"
    );
    assert_eq!(now.origin, origin);
}

#[test]
fn current_is_a_snapshot() {
    let live = LiveSettings::defaults_of(parsed("[orchestrator]\nmax_writers = 3\n"));
    let before = live.current();
    live.swap_owned(
        &parsed("[orchestrator]\nmax_writers = 1\n"),
        BTreeMap::new(),
    );
    assert_eq!(before.orchestrator.max_writers, 3);
    assert_eq!(live.current().orchestrator.max_writers, 1);
}

#[test]
fn defaults_of_marks_every_key_default() {
    let live = LiveSettings::defaults_of(config::Orchestrator::default());
    let origin = &live.current().origin;
    assert_eq!(origin.len(), proto::settings::SETTINGS_KEYS.len());
    assert!(origin.values().all(|o| *o == Origin::Default), "{origin:?}");
}
