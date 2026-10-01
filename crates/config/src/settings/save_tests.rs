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
    doc.models.clear();
    let problems = save(&path, &doc, &AtomicBool::new(false)).unwrap_err();
    assert_eq!(problems, validate(&doc));
    assert_eq!(problems, ["enable at least one model"]);
    assert_eq!(std::fs::read(&path).unwrap(), text.as_bytes());
    assert_eq!(entries(dir.path()), ["config.toml"]);
}

#[test]
fn a_cancelled_save_does_not_rename() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    let text = "[orchestrator]\nmax_writers = 2\n";
    std::fs::write(&path, text).unwrap();
    let mut doc = doc_of(&crate::load(&path).0.orchestrator);
    doc.limits.max_writers = 5;
    let cancel = AtomicBool::new(true);
    assert!(save(&path, &doc, &cancel).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), text.as_bytes());
    assert_eq!(entries(dir.path()), ["config.toml"]);
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
