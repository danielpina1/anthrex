//! Milestone 9.8 decision 40 (M9.8.12): a save writes `[models]`, removes the old model
//! keys and keeps `config.toml.bak`. Every file lives under `tempfile::tempdir()`.

use crate::models::{HelperKind, ModelRef, Role, RoleChoice};
use crate::settings::{doc_of, save};

/// The brief's fixture, but for the `review` candidate: fix round 2 (N1) keeps a list
/// the reviewer did not come from, and the brief's `codex` / `gpt-6-sol` is on the
/// medium route's own runtime (`default_runtime = "codex"`), so it never was the
/// reviewer. Claude Sonnet is, as the old reviewer pick chose; the test's point (every
/// old key goes) holds.
const OLD: &str = r#"# my settings
prefix = "a"

[orchestrator]
default_runtime = "codex"
builtin_models = true
max_writers = 3 # keep this comment

[orchestrator.agent]
runtime = "claude"
model = "claude-opus-5-5"
effort = "high"

[orchestrator.planners]
strength = "frontier"
max_tool_calls = 300

[orchestrator.deciders]
mode = "claude"
effort = "low"

[[orchestrator.models]]
runtime = "codex"
model = "gpt-6-sol"
strength = "standard"

[orchestrator.routes.review]
candidates = [{ runtime = "claude", model = "claude-sonnet-5" }]

[testing]
"#;

#[test]
fn a_save_writes_models_and_removes_every_old_key() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, OLD).unwrap();
    let before = crate::parse(OLD).0.orchestrator;
    let doc = doc_of(&before);
    save(&path, &doc, &Default::default()).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    for gone in [
        "default_runtime",
        "builtin_models",
        "[orchestrator.agent]",
        "strength",
        "[[orchestrator.models]]",
        "[orchestrator.routes.review]",
        "mode = \"claude\"\neffort",
    ] {
        assert!(!text.contains(gone), "{gone} is still there:\n{text}");
    }
    for kept in [
        "# my settings",
        "prefix = \"a\"",
        "max_writers = 3 # keep this comment",
        "max_tool_calls = 300",
        "[orchestrator.deciders]\nmode = \"claude\"\n",
        "[testing]",
    ] {
        assert!(text.contains(kept), "{kept} was lost:\n{text}");
    }
    let after = crate::parse(&text).0.orchestrator;
    assert!(after.roles_notes.is_empty(), "{:?}", after.roles_notes);
    assert_eq!(
        after.roles, before.roles,
        "the saved table is the migrated one"
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("config.toml.bak")).unwrap(),
        OLD
    );
}

#[test]
fn saving_keeps_every_resolved_row() {
    // Review focus 1: old keys and an explicit [models] row for the same role.
    let text = format!("{OLD}\n[models.reviewer]\nmodel = \"claude:claude-opus-5-5\"\n");
    let before = crate::parse(&text).0.orchestrator;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, &text).unwrap();
    save(&path, &doc_of(&before), &Default::default()).unwrap();
    let after = crate::parse(&std::fs::read_to_string(&path).unwrap())
        .0
        .orchestrator;
    for role in Role::all() {
        assert_eq!(
            crate::models::resolve(role, None, &after.roles),
            crate::models::resolve(role, None, &before.roles),
            "{role:?}"
        );
    }
    assert_eq!(
        crate::models::resolve_brainstorm(None, &after.roles),
        crate::models::resolve_brainstorm(None, &before.roles)
    );
}

#[test]
fn a_save_with_no_old_key_writes_no_bak() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "[models.planner]\nmodel = \"codex:default\"\n").unwrap();
    let mut o = crate::parse("[models.planner]\nmodel = \"codex:default\"\n")
        .0
        .orchestrator;
    o.roles.rows.insert(
        Role::Research,
        RoleChoice {
            model: ModelRef::parse("claude:claude-sonnet-5").unwrap(),
            effort: Some("medium".into()),
            fallback: None,
        },
    );
    save(&path, &doc_of(&o), &Default::default()).unwrap();
    assert!(!dir.path().join("config.toml.bak").exists());
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(
        text.contains(
            "[models.research]\nmodel = \"claude:claude-sonnet-5\"\neffort = \"medium\"\n"
        ),
        "{text}"
    );
}

#[test]
fn a_models_key_the_writer_cannot_edit_refuses() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    let text = "models = { planner = { model = \"codex:default\" } }\n";
    std::fs::write(&path, text).unwrap();
    let o = crate::parse(text).0.orchestrator;
    let problems = save(&path, &doc_of(&o), &Default::default()).unwrap_err();
    assert!(
        problems
            .iter()
            .any(|p| p.starts_with("models is written in a form the settings screen does not edit")),
        "{problems:?}"
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
}

/// Task 3's review: a hub list naming no model in the roster, with `default_runtime`
/// having no frontier model, migrates to no row. A save keeps it (and what it is read
/// against), the note says why, and every role still resolves as before.
const UNMIGRATED: &str = r#"[orchestrator]
default_runtime = "codex"

[orchestrator.routes.hub]
candidates = [{ runtime = "codex", model = "gpt-6.1-sol" }]
"#;

#[test]
fn an_old_list_that_migrated_to_no_row_is_kept_and_says_why() {
    let before = crate::parse(UNMIGRATED).0.orchestrator;
    assert!(!before.roles.rows.contains_key(&Role::ImplementerHub));
    let note = "config: [orchestrator.routes.hub]: none of its models are known; [models.implementer.hub] uses claude:claude-opus-5-5 until you choose one in C-b S";
    assert!(
        before.roles_notes.iter().any(|n| n == note),
        "{:?}",
        before.roles_notes
    );
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, UNMIGRATED).unwrap();
    save(&path, &doc_of(&before), &Default::default()).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(
        text.contains("[orchestrator.routes.hub]\ncandidates = [{ runtime = \"codex\", model = \"gpt-6.1-sol\" }]\n"),
        "the list was removed:\n{text}"
    );
    assert!(text.contains("default_runtime = \"codex\""), "{text}");
    assert!(text.contains("[models.implementer.small]"), "{text}");
    let after = crate::parse(&text).0.orchestrator;
    assert!(
        after.roles_notes.iter().any(|n| n == note),
        "{:?}",
        after.roles_notes
    );
    for role in Role::all() {
        assert_eq!(
            crate::models::resolve(role, None, &after.roles),
            crate::models::resolve(role, None, &before.roles),
            "{role:?}"
        );
    }

    // Choosing the row in the screen lets the next save remove the list.
    let mut doc = doc_of(&after);
    doc.roles.rows.insert(
        Role::ImplementerHub,
        RoleChoice {
            model: ModelRef::parse("codex:gpt-6.1-sol").unwrap(),
            effort: Some("high".into()),
            fallback: None,
        },
    );
    save(&path, &doc, &Default::default()).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(!text.contains("[orchestrator.routes"), "{text}");
    assert!(!text.contains("default_runtime"), "{text}");
    let after = crate::parse(&text).0.orchestrator;
    assert!(after.roles_notes.is_empty(), "{:?}", after.roles_notes);
    assert_eq!(after.roles, doc.roles);
}

fn model(label: &str) -> ModelRef {
    ModelRef::parse(label).unwrap()
}

/// Task 3's review: an explicit helper kind wins over the migrated `decider` list, and
/// an explicit `[models.brainstorm]` over the migrated brainstorm list, before a save
/// and after it.
#[test]
fn explicit_helper_kinds_and_brainstorm_win_over_migrated_lists() {
    let text = r#"[orchestrator.routes.decider]
candidates = [{ runtime = "claude", model = "claude-sonnet-5" }]

[orchestrator.routes.brainstorm]
candidates = [{ runtime = "claude", model = "claude-sonnet-5" }, { runtime = "claude", model = "claude-haiku-4-5" }]

[models.helpers.run_name]
model = "codex:default"
effort = "low"

[models.brainstorm]
first = "claude:claude-opus-5-5"
second = "codex:gpt-6-sol"
"#;
    let check = |o: &crate::Orchestrator| {
        let kind = crate::models::resolve(Role::Helper(HelperKind::RunName), None, &o.roles);
        assert_eq!(
            (kind.model, kind.effort),
            (model("codex:default"), Some("low".into()))
        );
        let other = crate::models::resolve(Role::Helper(HelperKind::Triage), None, &o.roles);
        assert_eq!(
            other.model,
            model("claude:claude-sonnet-5"),
            "the list's row"
        );
        let b = crate::models::resolve_brainstorm(None, &o.roles);
        assert_eq!(
            (b.first, b.second, b.effort),
            (
                model("claude:claude-opus-5-5"),
                model("codex:gpt-6-sol"),
                None
            )
        );
    };
    let before = crate::parse(text).0.orchestrator;
    check(&before);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, text).unwrap();
    save(&path, &doc_of(&before), &Default::default()).unwrap();
    let saved = std::fs::read_to_string(&path).unwrap();
    assert!(!saved.contains("[orchestrator.routes"), "{saved}");
    let after = crate::parse(&saved).0.orchestrator;
    check(&after);
    assert_eq!(after.roles, before.roles);
}

/// An unchanged explicit row keeps its comments; a changed one keeps its line's.
#[test]
fn rows_are_edited_in_place() {
    let text = "[models.planner] # mine\n# why codex\nmodel = \"codex:default\" # cheap\neffort = \"low\"\n\n[models.research]\nmodel = \"claude:claude-haiku-4-5\" # fast\n";
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, text).unwrap();
    let mut doc = doc_of(&crate::parse(text).0.orchestrator);
    let research = doc.roles.rows.get_mut(&Role::Research).unwrap();
    research.model = model("claude:claude-sonnet-5");
    save(&path, &doc, &Default::default()).unwrap();
    let saved = std::fs::read_to_string(&path).unwrap();
    assert_eq!(
        saved,
        "[models.planner] # mine\n# why codex\nmodel = \"codex:default\" # cheap\neffort = \"low\"\n\n[models.research]\nmodel = \"claude:claude-sonnet-5\" # fast\n"
    );
}

fn unsupported(key: &str, table: &str) -> String {
    super::UNSUPPORTED_FORM
        .replace("{key}", key)
        .replace("{table}", table)
}

/// An old key or a `models` key the writers cannot find refuses; the file is untouched.
/// (The old-key cases moved here from `write_tests::each_unsupported_form_is_refused`.)
#[test]
fn each_unsupported_models_or_old_form_is_refused() {
    let cases = [
        (
            "[orchestrator]\nagent = { runtime = \"claude\" }\n",
            unsupported("orchestrator.agent.runtime", "orchestrator.agent"),
        ),
        (
            "[orchestrator]\nmodels = [{ runtime = \"claude\", model = \"claude-opus-5-5\", strength = \"frontier\" }]\n",
            unsupported("orchestrator.models", "[orchestrator.models]"),
        ),
        (
            "[orchestrator.routes]\nhub = { candidates = [{ runtime = \"claude\", model = \"claude-opus-5-5\" }] }\n",
            unsupported("orchestrator.routes.hub", "orchestrator.routes.hub"),
        ),
        (
            "[models]\nplanner = { model = \"codex:default\" }\n",
            unsupported("models.planner", "models.planner"),
        ),
        (
            "[models.helpers]\nrun_name.model = \"codex:default\"\n",
            unsupported("models.helpers.run_name.model", "models.helpers.run_name"),
        ),
        (
            "[[models.planner]]\nmodel = \"codex:default\"\n",
            unsupported("models.planner", "models.planner"),
        ),
    ];
    for (text, want) in cases {
        assert!(text.parse::<toml::Table>().is_ok(), "{text}");
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, text).unwrap();
        let mut doc = doc_of(&crate::parse(text).0.orchestrator);
        doc.limits.max_writers = 4;
        assert_eq!(
            save(&path, &doc, &Default::default()),
            Err(vec![want]),
            "{text}"
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
    }
}

/// Review probe (moved from `write_tests`): a literal multi-line string holding a
/// roster header survives the save that removes the real block.
#[test]
fn a_literal_string_holding_a_roster_header_survives() {
    let text = "notes = '''\n[[orchestrator.models]]\nruntime = \"codex\"\n'''\n\n[[orchestrator.models]]\nruntime = \"codex\"\nmodel = \"gpt-6-sol\"\nstrength = \"standard\"\n";
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, text).unwrap();
    save(
        &path,
        &doc_of(&crate::parse(text).0.orchestrator),
        &Default::default(),
    )
    .unwrap();
    let out = std::fs::read_to_string(&path).unwrap();
    assert_eq!(
        out,
        "notes = '''\n[[orchestrator.models]]\nruntime = \"codex\"\n'''\n"
    );
}

/// Review probe (moved from `write_tests`): a `#` inside a quoted value is not its
/// comment.
#[test]
fn a_hash_inside_a_quoted_model_is_not_a_comment() {
    let text = "[models.planner]\nmodel = \"codex:a#b\" # c\n";
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, text).unwrap();
    let mut doc = doc_of(&crate::parse(text).0.orchestrator);
    assert_eq!(doc.roles.rows[&Role::Planner].model, model("codex:a#b"));
    doc.roles.rows.get_mut(&Role::Planner).unwrap().model = model("codex:x#y");
    save(&path, &doc, &Default::default()).unwrap();
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "[models.planner]\nmodel = \"codex:x#y\" # c\n"
    );
}

/// A row the screen resets loses its table; a `deciders.mode` of `codex` would migrate
/// the helpers row back, so the reset row is written as what the screen showed.
#[test]
fn a_reset_row_goes_and_a_live_key_cannot_bring_it_back() {
    let text = "[orchestrator.deciders]\nmode = \"codex\"\n\n[models.planner]\nmodel = \"codex:default\"\n";
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, text).unwrap();
    let before = crate::parse(text).0.orchestrator;
    assert_eq!(
        before.roles.rows[&Role::Helpers].model,
        model("codex:default")
    );
    let mut doc = doc_of(&before);
    doc.roles.rows.remove(&Role::Planner);
    doc.roles.rows.remove(&Role::Helpers);
    save(&path, &doc, &Default::default()).unwrap();
    let out = std::fs::read_to_string(&path).unwrap();
    assert!(!out.contains("[models.planner]"), "{out}");
    assert!(
        out.starts_with("[orchestrator.deciders]\nmode = \"codex\"\n"),
        "{out}"
    );
    let after = crate::parse(&out).0.orchestrator;
    for role in Role::all() {
        assert_eq!(
            crate::models::resolve(role, None, &after.roles),
            crate::models::resolve(role, None, &doc.roles),
            "{role:?}\n{out}"
        );
    }
    assert!(
        !dir.path().join("config.toml.bak").exists(),
        "mode is not an old key"
    );
}

/// Fix round 1, I1: every list kind and the row it feeds.
const LISTS: [(&str, &str); 9] = [
    ("s", "implementer.small"),
    ("m", "implementer.medium"),
    ("hub", "implementer.hub"),
    ("review", "reviewer"),
    ("scout", "research"),
    ("decider", "helpers"),
    ("planner", "planner"),
    ("orchestrator", "orchestrator"),
    ("brainstorm", "brainstorm"),
];

fn uses(o: &crate::Orchestrator, row: &str) -> String {
    match Role::parse_key(row) {
        Some(role) => crate::models::resolve(role, None, &o.roles).model.label(),
        None => {
            let b = crate::models::resolve_brainstorm(None, &o.roles);
            format!("{} and {}", b.first.label(), b.second.label())
        }
    }
}

fn saved(text: &str) -> (crate::Orchestrator, String, crate::Orchestrator) {
    let before = crate::parse(text).0.orchestrator;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, text).unwrap();
    save(&path, &doc_of(&before), &Default::default()).unwrap();
    let out = std::fs::read_to_string(&path).unwrap();
    let after = crate::parse(&out).0.orchestrator;
    (before, out, after)
}

/// Fix round 1, I1 (controller ruling): a list none of whose models is known migrated
/// nothing; a save keeps it, and its note says what the row uses meanwhile.
#[test]
fn a_list_whose_models_are_all_unknown_is_kept_and_noted() {
    for (name, row) in LISTS {
        let text = format!(
            "[orchestrator.routes.{name}]\ncandidates = [{{ runtime = \"codex\", model = \"gpt-9-unknown\" }}]\n"
        );
        let (before, out, after) = saved(&text);
        let note = format!(
            "config: [orchestrator.routes.{name}]: none of its models are known; [models.{row}] uses {} until you choose one in C-b S",
            uses(&before, row)
        );
        assert!(
            before.roles_notes.contains(&note),
            "{name}: {:?}",
            before.roles_notes
        );
        assert!(out.contains(&text), "{name}: the list went:\n{out}");
        assert!(
            after.roles_notes.contains(&note),
            "{name}: {:?}",
            after.roles_notes
        );
        for role in Role::all() {
            assert_eq!(
                crate::models::resolve(role, None, &after.roles),
                crate::models::resolve(role, None, &before.roles),
                "{name}: {role:?}"
            );
        }
    }
}

/// Fix round 1, I1: one known model is enough; the list is migrated and removed.
#[test]
fn a_list_with_one_known_model_is_migrated_and_removed() {
    for (name, _) in LISTS {
        // The reviewer must be on the medium route's other runtime (N1).
        let known = match name {
            "review" => "{ runtime = \"codex\", model = \"gpt-6-sol\" }",
            _ => "{ runtime = \"claude\", model = \"claude-opus-5-5\" }",
        };
        let mut text = format!(
            "[orchestrator.routes.{name}]\ncandidates = [{{ runtime = \"codex\", model = \"gpt-9-unknown\" }}, {known}]\n"
        );
        if name == "review" {
            text.push_str("\n[[orchestrator.models]]\nruntime = \"codex\"\nmodel = \"gpt-6-sol\"\nstrength = \"standard\"\n");
        }
        let (_, out, after) = saved(&text);
        assert!(!out.contains("[orchestrator."), "{name}:\n{out}");
        assert!(
            after.roles_notes.is_empty(),
            "{name}: {:?}",
            after.roles_notes
        );
    }
}

/// Fix round 1, M1: the first save that removes old keys writes `config.toml.bak`; a
/// later one leaves it alone, so the file from before the role table is never lost.
#[test]
fn an_existing_bak_is_never_overwritten() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    let bak = dir.path().join("config.toml.bak");
    std::fs::write(&path, OLD).unwrap();
    save(
        &path,
        &doc_of(&crate::parse(OLD).0.orchestrator),
        &Default::default(),
    )
    .unwrap();
    assert_eq!(std::fs::read_to_string(&bak).unwrap(), OLD);
    // Old keys again (a hand edit), and a second removing save.
    let again = format!(
        "{}\n[orchestrator.scouts]\nstrength = \"standard\"\n",
        std::fs::read_to_string(&path).unwrap()
    );
    std::fs::write(&path, &again).unwrap();
    save(
        &path,
        &doc_of(&crate::parse(&again).0.orchestrator),
        &Default::default(),
    )
    .unwrap();
    assert!(!std::fs::read_to_string(&path).unwrap().contains("scouts"));
    assert_eq!(
        std::fs::read_to_string(&bak).unwrap(),
        OLD,
        "the first .bak stays"
    );
    // A `.bak` that is not a file is left alone too; the save still goes through.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::create_dir(dir.path().join("config.toml.bak")).unwrap();
    std::fs::write(&path, OLD).unwrap();
    save(
        &path,
        &doc_of(&crate::parse(OLD).0.orchestrator),
        &Default::default(),
    )
    .unwrap();
    assert!(dir.path().join("config.toml.bak").is_dir());
    let mut names: Vec<String> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert_eq!(
        names,
        ["config.toml", "config.toml.bak"],
        "no temporary file left"
    );
}

/// Fix round 1, M8: CRLF, no final newline and Unicode survive the models writer.
#[test]
fn crlf_no_final_newline_and_unicode_survive() {
    let text = "# réglages ✓\r\n[orchestrator]\r\nmax_writers = 2 # «deux»\r\nbuiltin_models = true\r\n\r\n[models.planner] # ü\r\nmodel = \"codex:default\"";
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, text).unwrap();
    let mut doc = doc_of(&crate::parse(text).0.orchestrator);
    doc.roles.rows.insert(
        Role::Research,
        RoleChoice {
            model: model("claude:claude-sonnet-5"),
            effort: None,
            fallback: None,
        },
    );
    save(&path, &doc, &Default::default()).unwrap();
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "# réglages ✓\r\n[orchestrator]\r\nmax_writers = 2 # «deux»\r\n\r\n[models.planner] # ü\r\nmodel = \"codex:default\"\r\n\r\n[models.research]\r\nmodel = \"claude:claude-sonnet-5\""
    );
}

/// Fix round 1, M8, M3, M4: more forms the writers cannot edit, each refused with its
/// `UNSUPPORTED_FORM` text and the file untouched.
#[test]
fn more_unsupported_forms_are_refused() {
    let cases = [
        (
            "orchestrator.agent.model = \"claude-opus-5-5\"\n",
            unsupported("orchestrator.agent.model", "orchestrator.agent"),
        ),
        (
            "[orchestrator]\nagent.model = \"claude-opus-5-5\"\n",
            unsupported("orchestrator.agent.model", "orchestrator.agent"),
        ),
        (
            "[models.helpers]\nrun_name = { model = \"codex:default\" }\n",
            unsupported("models.helpers.run_name", "models.helpers.run_name"),
        ),
        (
            "[models.\"implementer.small\"]\nmodel = \"codex:default\"\n",
            unsupported("models.implementer.small", "models.<row>"),
        ),
    ];
    for (text, want) in cases {
        assert!(text.parse::<toml::Table>().is_ok(), "{text}");
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, text).unwrap();
        let mut doc = doc_of(&crate::parse(text).0.orchestrator);
        doc.roles.rows.insert(
            Role::ImplementerSmall,
            RoleChoice {
                model: model("claude:claude-opus-5-5"),
                effort: None,
                fallback: None,
            },
        );
        assert_eq!(
            save(&path, &doc, &Default::default()),
            Err(vec![want]),
            "{text}"
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
    }
}

/// Fix round 1, M7: an emptied old table goes whole, its comment lines with it.
#[test]
fn an_emptied_old_table_takes_its_comments() {
    let text = "[orchestrator]\nmax_writers = 2\n\n[orchestrator.agent] # hdr\n# about the agent\nmodel = \"claude-opus-5-5\"\n\n[testing]\n";
    let (_, out, _) = saved(text);
    assert!(
        out.starts_with("[orchestrator]\nmax_writers = 2\n\n[testing]\n"),
        "{out}"
    );
}

/// Fix round 2 (N1): a `review` list counts as migrated only when the reviewer came
/// from it. Here its one model, in the roster, is on the medium route's own runtime,
/// so the reviewer is the old reviewer pick's `codex:default`: the list is kept, and its
/// note says why (M9.8.13 fix round 1, M4): its model is known, but cannot review
/// Claude's work.
#[test]
fn a_review_list_the_reviewer_did_not_come_from_is_kept() {
    let text = "[orchestrator.routes.review]\ncandidates = [{ runtime = \"claude\", model = \"claude-sonnet-5\" }]\n";
    let (before, out, after) = saved(text);
    assert_eq!(
        before.roles.rows[&Role::Reviewer].model,
        model("codex:default")
    );
    let note = "config: [orchestrator.routes.review]: none of its models can review claude's work; [models.reviewer] uses codex:default until you choose one in C-b S";
    assert!(
        before.roles_notes.iter().any(|n| n == note),
        "{:?}",
        before.roles_notes
    );
    assert!(out.contains(text), "the list went:\n{out}");
    assert!(
        after.roles_notes.iter().any(|n| n == note),
        "{:?}",
        after.roles_notes
    );
}

/// Fix round 2: an inline list naming no known model is kept as written.
#[test]
fn an_inline_unknown_hub_list_is_kept() {
    let text = "[orchestrator]\nmax_writers = 2\n\n[orchestrator.routes]\nhub = { candidates = [{ runtime = \"codex\", model = \"gpt-9-unknown\" }] }\n";
    let (before, out, after) = saved(text);
    assert!(
        out.contains(
            "hub = { candidates = [{ runtime = \"codex\", model = \"gpt-9-unknown\" }] }\n"
        ),
        "{out}"
    );
    let note = "config: [orchestrator.routes.hub]: none of its models are known; [models.implementer.hub] uses claude:claude-opus-5-5 until you choose one in C-b S";
    assert!(
        before.roles_notes.iter().any(|n| n == note),
        "{:?}",
        before.roles_notes
    );
    assert!(
        after.roles_notes.iter().any(|n| n == note),
        "{:?}",
        after.roles_notes
    );
}
