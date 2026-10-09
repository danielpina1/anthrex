//! Every file here lives under `tempfile::tempdir()`: no test reads or writes the user's
//! config (global constraint 10).

use std::sync::atomic::AtomicBool;

use super::*;

fn entries(dir: &std::path::Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[test]
fn an_owned_key_in_another_form_refuses_and_the_file_is_byte_identical() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    let text = "[orchestrator]\nbudget.s = { tool_calls = 30, minutes = 10 }\n# keep me\n";
    std::fs::write(&path, text).unwrap();
    let (config, _) = crate::load(&path);
    let mut doc = doc_of(&config.orchestrator);
    doc.limits.max_writers = 4;
    let problems = save(&path, &doc, &AtomicBool::new(false)).unwrap_err();
    assert_eq!(problems.len(), 1);
    assert!(
        problems[0].starts_with("orchestrator.budget.s is written in a form"),
        "{problems:?}"
    );
    assert_eq!(std::fs::read(&path).unwrap(), text.as_bytes());
    assert_eq!(
        std::fs::read_dir(dir.path()).unwrap().count(),
        1,
        "a temporary file was left"
    );
}

/// Preflight F18: `save` asks `validate` first and returns its problems untouched.
#[test]
fn a_refused_save_leaves_no_temporary_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    let text = "# mine\r\n[orchestrator]\r\nmax_writers = 2";
    std::fs::write(&path, text).unwrap();
    let mut doc = doc_of(&crate::load(&path).0.orchestrator);
    doc.limits.max_writers = 0;
    let problems = save(&path, &doc, &AtomicBool::new(false)).unwrap_err();
    assert_eq!(problems, validate(&doc));
    assert_eq!(
        problems,
        ["orchestrator.max_writers: must be between 1 and 8"]
    );
    assert_eq!(std::fs::read(&path).unwrap(), text.as_bytes());
    assert_eq!(entries(dir.path()), ["config.toml"]);
}

#[test]
fn a_cancelled_save_does_not_rename() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    // M9.8.12: an old key, so the save would also write `config.toml.bak`; it does not.
    let text = "[orchestrator]\nmax_writers = 2\nbuiltin_models = true\n";
    std::fs::write(&path, text).unwrap();
    let mut doc = doc_of(&crate::load(&path).0.orchestrator);
    doc.limits.max_writers = 5;
    let cancel = AtomicBool::new(true);
    assert!(save(&path, &doc, &cancel).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), text.as_bytes());
    assert_eq!(entries(dir.path()), ["config.toml"]);
}

/// Milestone 9.0.6 task 9 (the timeout's window): a save claims the rename by setting
/// the flag, so a caller that sets it afterwards and reads `true` knows the rename was
/// already under way, and one that reads `false` knows it never will be.
#[test]
fn a_save_claims_the_flag_before_it_renames() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "[orchestrator]\nmax_writers = 2\n").unwrap();
    let mut doc = doc_of(&crate::load(&path).0.orchestrator);
    doc.limits.max_writers = 5;
    let flag = AtomicBool::new(false);
    save(&path, &doc, &flag).unwrap();
    assert!(flag.load(std::sync::atomic::Ordering::SeqCst));
}

#[cfg(unix)]
#[test]
fn a_save_keeps_the_file_mode() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "[orchestrator]\nmax_writers = 2\n").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    let mut doc = doc_of(&crate::load(&path).0.orchestrator);
    doc.limits.max_writers = 5;
    save(&path, &doc, &AtomicBool::new(false)).unwrap();
    let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "[orchestrator]\nmax_writers = 5\n"
    );
    assert_eq!(entries(dir.path()), ["config.toml"]);
}

#[test]
fn a_missing_file_is_created() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("anthrex").join("config.toml");
    let mut doc = doc_of(&crate::Orchestrator::default());
    doc.limits.stall_after_secs = 120;
    let saved = save(&path, &doc, &AtomicBool::new(false)).unwrap();
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "[orchestrator]\nstall_after_secs = 120\n"
    );
    assert_eq!(doc_of(&saved.orchestrator), doc);
    assert_eq!(
        saved.origin[proto::settings::key::STALL_AFTER_SECS],
        proto::Origin::File
    );
    assert_eq!(
        saved.origin[proto::settings::key::MAX_WRITERS],
        proto::Origin::Default
    );
    assert_eq!(entries(&dir.path().join("anthrex")), ["config.toml"]);
}

#[test]
fn load_with_origin_reads_the_same_config_as_load() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    let text = "prefix = \"C-a\"\nbogus = 1\n[orchestrator]\nmax_readers = 4\n[orchestrator.budget.l]\nminutes = 0\n";
    std::fs::write(&path, text).unwrap();
    let (config, problems, origin) = load_with_origin(&path);
    assert_eq!((config, problems), crate::load(&path));
    assert_eq!(origin, origin_of(&text.parse().unwrap()));
    assert_eq!(
        origin[proto::settings::key::MAX_READERS],
        proto::Origin::File
    );
    assert_eq!(
        origin[proto::settings::key::BUDGET_L_MINUTES],
        proto::Origin::File
    );
}

/// Review fix round 1: a symlinked config stays a link; its target gets the new text.
#[cfg(unix)]
#[test]
fn a_symlinked_config_stays_a_link() {
    let dir = tempfile::tempdir().unwrap();
    let real_dir = dir.path().join("dotfiles");
    std::fs::create_dir(&real_dir).unwrap();
    let target = real_dir.join("anthrex.toml");
    std::fs::write(&target, "[orchestrator]\nmax_writers = 2\n").unwrap();
    let link = dir.path().join("config.toml");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    let mut doc = doc_of(&crate::load(&link).0.orchestrator);
    doc.limits.max_writers = 6;
    save(&link, &doc, &AtomicBool::new(false)).unwrap();
    assert!(
        std::fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(
        std::fs::read_to_string(&target).unwrap(),
        "[orchestrator]\nmax_writers = 6\n"
    );
    assert_eq!(entries(dir.path()), ["config.toml", "dotfiles"]);
    assert_eq!(entries(&real_dir), ["anthrex.toml"]);
}

/// M9.2.6 fix round 2 kept a stored roster note with `⚠️` from blocking a save. Since
/// M9.8.12 the roster is an old key a save removes: the note goes with its block, every
/// comment stays, and `config.toml.bak` holds the file as it was (decision 40).
#[test]
fn a_roster_with_a_variation_selector_goes_to_the_bak() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    let text = "# my settings\n[orchestrator]\nmax_writers = 2 # mine\n\n[[orchestrator.models]]\nruntime = \"codex\"\nmodel = \"gpt-6-sol\"\nstrength = \"standard\"\nnote = \"\u{26A0}\u{FE0F} careful\"\n";
    std::fs::write(&path, text).unwrap();
    let mut doc = doc_of(&crate::load(&path).0.orchestrator);
    doc.limits.max_writers = 3;
    let saved = save(&path, &doc, &AtomicBool::new(false)).expect("the save goes through");
    let on_disk = std::fs::read_to_string(&path).unwrap();
    assert_eq!(
        on_disk,
        "# my settings\n[orchestrator]\nmax_writers = 3 # mine\n"
    );
    let read = doc_of(&crate::load(&path).0.orchestrator);
    assert_eq!(read.limits.max_writers, 3);
    assert_eq!(doc_of(&saved.orchestrator), read);
    assert_eq!(entries(dir.path()), ["config.toml", "config.toml.bak"]);
    let bak = std::fs::read_to_string(dir.path().join("config.toml.bak")).unwrap();
    assert_eq!(bak, text);
}

/// A row a save would write with a bad effort refuses, and the file is untouched.
#[test]
fn a_bad_row_refuses_and_the_file_is_untouched() {
    use crate::models::{ModelRef, Role, RoleChoice};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    let text = "[orchestrator]\nmax_writers = 2\nbuiltin_models = true\n";
    std::fs::write(&path, text).unwrap();
    let mut doc = doc_of(&crate::load(&path).0.orchestrator);
    doc.roles.rows.insert(
        Role::Planner,
        RoleChoice {
            model: ModelRef::parse("codex:default").unwrap(),
            effort: Some("a\x07b".into()),
            fallback: None,
        },
    );
    let problems = save(&path, &doc, &AtomicBool::new(false)).unwrap_err();
    assert_eq!(
        problems,
        [
            "models.planner.effort: \"a\\u{7}b\" is not an effort name (1 to 16 of a-z, 0-9, _ and -)"
        ]
    );
    assert_eq!(std::fs::read(&path).unwrap(), text.as_bytes());
    assert_eq!(entries(dir.path()), ["config.toml"]);
}

/// Ruling T18-7: `design_default` is the daemon's report, never the screen's to save.
/// A default hand-edited while the daemon runs, so the screen's copy is stale (or a
/// client that sends none), still saves the owned keys, and the file keeps the edit.
#[test]
fn a_hand_edited_design_default_still_saves() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "[orchestrator]\nmax_writers = 2\n").unwrap();
    // The daemon read the file before the edit: `full`, the default's default.
    let mut doc = doc_of(&crate::load(&path).0.orchestrator);
    assert_eq!(doc.design_default, Some(proto::DesignMode::Full));
    let edited = "[orchestrator]\nmax_writers = 2\n\n[orchestrator.design]\ndefault = \"off\"\n";
    std::fs::write(&path, edited).unwrap();
    for sent in [Some(proto::DesignMode::Full), None] {
        doc.design_default = sent;
        doc.limits.max_writers += 1;
        save(&path, &doc, &AtomicBool::new(false)).unwrap_or_else(|p| panic!("{sent:?}: {p:?}"));
        let saved = crate::load(&path).0.orchestrator;
        assert_eq!(saved.max_writers, doc.limits.max_writers);
        assert_eq!(
            doc_of(&saved).design_default,
            Some(proto::DesignMode::Off),
            "the hand edit stays"
        );
    }
}

/// Final review I2: with a `config.toml.bak` already there (a hand-made backup, say), the
/// save that removes old keys still keeps the file as it was, as `config.toml.bak.1`,
/// and never replaces the existing backup; the next one takes `.bak.2`.
#[test]
fn an_existing_bak_does_not_stop_the_backup_of_the_old_keys() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    let text =
        "[orchestrator]\nmax_writers = 2\n\n[orchestrator.planners]\nstrength = \"frontier\"\n";
    std::fs::write(&path, text).unwrap();
    std::fs::write(dir.path().join("config.toml.bak"), "mine").unwrap();
    let doc = doc_of(&crate::load(&path).0.orchestrator);
    save(&path, &doc, &AtomicBool::new(false)).expect("the save goes through");
    let now = std::fs::read_to_string(&path).unwrap();
    assert!(!now.contains("planners"), "{now}");
    assert_eq!(
        std::fs::read_to_string(dir.path().join("config.toml.bak")).unwrap(),
        "mine"
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("config.toml.bak.1")).unwrap(),
        text
    );
    // A second removing save (old keys again) takes the next free name.
    let again = format!(
        "{}\n[orchestrator.scouts]\nstrength = \"standard\"\n",
        std::fs::read_to_string(&path).unwrap()
    );
    std::fs::write(&path, &again).unwrap();
    let doc = doc_of(&crate::load(&path).0.orchestrator);
    save(&path, &doc, &AtomicBool::new(false)).unwrap();
    assert_eq!(
        std::fs::read_to_string(dir.path().join("config.toml.bak.2")).unwrap(),
        again
    );
    assert_eq!(
        entries(dir.path()),
        [
            "config.toml",
            "config.toml.bak",
            "config.toml.bak.1",
            "config.toml.bak.2"
        ]
    );
}

/// Final review I1: `changed_since_loaded` is silent for the file the settings were read
/// from, and names the change once a `[models]` row is hand-edited.
#[test]
fn a_file_edited_after_it_was_read_is_noticed() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "[models.reviewer]\nmodel = \"codex:gpt-6-sol\"\n").unwrap();
    let live = crate::load(&path).0.orchestrator;
    assert_eq!(changed_since_loaded(&path, &live), None);
    std::fs::write(
        &path,
        "[models.reviewer]\nmodel = \"claude:claude-opus-5-5\"\n",
    )
    .unwrap();
    assert_eq!(
        changed_since_loaded(&path, &live).as_deref(),
        Some(CHANGED_SINCE_LOADED)
    );
}
