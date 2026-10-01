use super::*;
use proto::{ModelEntry, Runtime, Strength};

// The brief's `prefix = "a"` is not a valid prefix (`parse` wants `C-` and a letter), so
// the read-back `problems.is_empty()` below could never hold; `C-a` keeps the case's
// intent (an unrelated top-level key with a comment). Preflight F15 adds `# budgets`.
const HAND_WRITTEN: &str = r#"# my anthrex config
prefix = "C-a"   # tmux habit

[orchestrator]
git_timeout_secs = 5   # slow disk
max_writers = 2
deciders.mode = "off"

[orchestrator.cache_dirs]
"/tmp/repo" = ["/tmp/cache"]

# models I trust
[[orchestrator.models]]
runtime = "claude"
model = "claude-opus-5-5"
strength = "frontier"

# budgets
[orchestrator.budget.s]
tool_calls = 30   # tight
minutes = 10

[testing]
test_slots = 4
"#;

fn model(runtime: Runtime, name: &str, strength: Strength) -> ModelEntry {
    ModelEntry {
        runtime,
        model: name.into(),
        strength,
        note: String::new(),
    }
}

fn unsupported(key: &str, table: &str) -> String {
    UNSUPPORTED_FORM
        .replace("{key}", key)
        .replace("{table}", table)
}

/// Review focus 2.
#[test]
fn a_save_keeps_every_unrelated_line_and_comment() {
    let (before, _) = crate::parse(HAND_WRITTEN);
    let mut doc = doc_of(&before.orchestrator);
    doc.limits.max_writers = 4;
    doc.limits.budget_s.tool_calls = 25;
    doc.models.push(ModelEntry {
        runtime: Runtime::Codex,
        model: "gpt-6-sol".into(),
        strength: Strength::Standard,
        note: String::new(),
    });
    let text = edit_text(HAND_WRITTEN, &doc).unwrap();
    for line in [
        "# my anthrex config",
        "prefix = \"C-a\"   # tmux habit",
        "git_timeout_secs = 5   # slow disk",
        "deciders.mode = \"off\"",
        "[orchestrator.cache_dirs]",
        "\"/tmp/repo\" = [\"/tmp/cache\"]",
        "# models I trust",
        "# budgets",
        "[orchestrator.budget.s]",
        "[testing]",
        "test_slots = 4",
    ] {
        assert!(text.lines().any(|l| l == line), "{line:?} lost:\n{text}");
    }
    assert!(text.lines().any(|l| l == "max_writers = 4"));
    assert!(text.lines().any(|l| l == "tool_calls = 25   # tight"));
    assert!(text.lines().any(|l| l == "builtin_models = false"));
    // F15: the comment above the next table stays above it.
    assert!(
        text.contains("# budgets\n[orchestrator.budget.s]\n"),
        "{text}"
    );
    let (after, problems) = crate::parse(&text);
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(doc_of(&after.orchestrator), doc);
    assert_eq!(after.orchestrator.git_timeout_secs, 5);
    assert_eq!(
        after.orchestrator.cache_dirs,
        before.orchestrator.cache_dirs
    );
    assert_eq!(after.orchestrator.deciders, before.orchestrator.deciders);
    assert_eq!(after.testing, before.testing);
    assert_eq!(after.prefix, before.prefix);
}

#[test]
fn an_untouched_default_stays_absent() {
    let (before, _) = crate::parse(HAND_WRITTEN);
    let doc = doc_of(&before.orchestrator);
    let text = edit_text(HAND_WRITTEN, &doc).unwrap();
    assert!(!text.contains("stall_after_secs"), "{text}");
    assert_eq!(
        origin_of(&text.parse().unwrap())[proto::settings::key::STALL_AFTER_SECS],
        proto::Origin::Default
    );
}

/// Preflight F16: an unchanged doc leaves the file byte for byte, roster included.
#[test]
fn an_unchanged_doc_leaves_the_file_byte_for_byte() {
    let doc = doc_of(&crate::parse(HAND_WRITTEN).0.orchestrator);
    assert_eq!(edit_text(HAND_WRITTEN, &doc).unwrap(), HAND_WRITTEN);
    let builtin = doc_of(&crate::Orchestrator::default());
    assert_eq!(edit_text("", &builtin).unwrap(), "");
    let other = "# only a comment\n[testing]\ntest_slots = 4";
    assert_eq!(edit_text(other, &builtin).unwrap(), other);
}

#[test]
fn a_missing_file_gets_only_what_differs_from_the_defaults() {
    let mut doc = doc_of(&crate::Orchestrator::default());
    doc.limits.max_readers = 5;
    let text = edit_text("", &doc).unwrap();
    assert!(text.starts_with("[orchestrator]\n"), "{text}");
    assert!(text.lines().any(|l| l == "max_readers = 5"));
    assert_eq!(doc_of(&crate::parse(&text).0.orchestrator), doc);
    assert_eq!(text, "[orchestrator]\nmax_readers = 5\n");
}

#[test]
fn crlf_and_a_missing_final_newline_survive() {
    let text = "[orchestrator]\r\nmax_writers = 2\r\n# end";
    let mut doc = doc_of(&crate::parse(text).0.orchestrator);
    doc.limits.max_writers = 3;
    let out = edit_text(text, &doc).unwrap();
    assert!(
        out.contains("max_writers = 3\r\n") && out.ends_with("# end"),
        "{out:?}"
    );
    doc.limits.max_bounces = 4;
    doc.limits.budget_l.minutes = 200;
    let out = edit_text(text, &doc).unwrap();
    assert_eq!(
        out,
        "[orchestrator]\r\nmax_writers = 3\r\nmax_bounces = 4\r\n# end\r\n\r\n[orchestrator.budget.l]\r\nminutes = 200"
    );
    assert_eq!(doc_of(&crate::parse(&out).0.orchestrator), doc);
}

/// Preflight F17: `RunHarness` indents its table headers by 17 spaces.
#[test]
fn an_indented_header_is_still_a_header() {
    let text = "[orchestrator]\ngit_timeout_secs = 5\n\n\n                 [orchestrator.cache_dirs]\n\"/tmp/repo\" = [\"/tmp/cache\"]\n\n                 [orchestrator.profile]\ncheck_timeout_secs = 10\n";
    let before = crate::parse(text).0.orchestrator;
    let mut doc = doc_of(&before);
    doc.models = vec![model(Runtime::Codex, "gpt-6-sol", Strength::Standard)];
    doc.limits.max_writers = 1;
    let out = edit_text(text, &doc).unwrap();
    assert!(
        out.starts_with(
            "[orchestrator]\ngit_timeout_secs = 5\nbuiltin_models = false\nmax_writers = 1\n\n\n                 [orchestrator.cache_dirs]\n"
        ),
        "{out}"
    );
    let after = crate::parse(&out).0.orchestrator;
    assert_eq!(doc_of(&after), doc);
    assert_eq!(after.cache_dirs, before.cache_dirs);
    assert_eq!(after.profile, before.profile);

    // An indented owned table and key are edited in place, their indentation kept.
    let text = "  [orchestrator.budget.m]\n  minutes = 5 # short\n";
    let mut doc = doc_of(&crate::parse(text).0.orchestrator);
    doc.limits.budget_m.minutes = 6;
    doc.limits.budget_m.tool_calls = 7;
    let out = edit_text(text, &doc).unwrap();
    assert_eq!(
        out,
        "  [orchestrator.budget.m]\n  minutes = 6 # short\n  tool_calls = 7\n"
    );
}

#[test]
fn each_unsupported_form_is_refused() {
    let doc = doc_of(&crate::Orchestrator::default());
    let cases = [
        (
            "[orchestrator]\nbudget.s.minutes = 5\n",
            unsupported("orchestrator.budget.s.minutes", "orchestrator.budget.s"),
        ),
        (
            "[orchestrator]\nbudget = { s = { tool_calls = 1, minutes = 1 } }\n",
            unsupported("orchestrator.budget", "orchestrator.budget.<s|m|l>"),
        ),
        (
            "[orchestrator]\nagent = { runtime = \"claude\" }\n",
            unsupported("orchestrator.agent", "orchestrator.agent"),
        ),
        (
            "[orchestrator]\nmodels = [{ runtime = \"claude\", model = \"claude-opus-5-5\", strength = \"frontier\" }]\n",
            unsupported("orchestrator.models", "[orchestrator.models]"),
        ),
        (
            "[orchestrator.budget]\ns = { tool_calls = 1, minutes = 1 }\n",
            unsupported("orchestrator.budget", "orchestrator.budget.<s|m|l>"),
        ),
        (
            "[orchestrator]\nmax_writers = [\n  2,\n]\n",
            unsupported("orchestrator.max_writers", "orchestrator"),
        ),
        (
            "orchestrator.max_writers = 2\n",
            unsupported("orchestrator.max_writers", "orchestrator"),
        ),
    ];
    for (text, want) in cases {
        assert!(text.parse::<toml::Table>().is_ok(), "{text}");
        assert_eq!(edit_text(text, &doc), Err(vec![want]), "{text}");
    }
    assert_eq!(
        edit_text("[orchestrator\n", &doc).unwrap_err().len(),
        1,
        "invalid TOML is refused"
    );
}

#[test]
fn strings_are_quoted_and_escaped() {
    let mut doc = doc_of(&crate::Orchestrator::default());
    let mut m = model(Runtime::Claude, "claude-opus-5-5", Strength::Frontier);
    m.note = r#"say "hi" \ then"#.into();
    doc.models = vec![m];
    doc.orchestrator = proto::OrchestratorDefault {
        runtime: Some(Runtime::Claude),
        model: "claude-opus-5-5".into(),
    };
    let text = edit_text("", &doc).unwrap();
    assert!(
        text.lines()
            .any(|l| l == r#"note = "say \"hi\" \\ then""# || l == r#"note = 'say "hi" \ then'"#),
        "{text}"
    );
    assert_eq!(doc_of(&crate::parse(&text).0.orchestrator), doc);
}

#[test]
fn the_orchestrator_default_goes_under_its_table() {
    let text = "[orchestrator]\nmax_writers = 2\n\n[[orchestrator.models]]\nruntime = \"codex\"\nmodel = \"gpt-6-sol\"\nstrength = \"standard\"\n";
    let mut doc = doc_of(&crate::parse(text).0.orchestrator);
    doc.orchestrator = proto::OrchestratorDefault {
        runtime: Some(Runtime::Codex),
        model: "gpt-6-sol".into(),
    };
    let out = edit_text(text, &doc).unwrap();
    let block = "[[orchestrator.models]]\nruntime = \"codex\"\nmodel = \"gpt-6-sol\"\nstrength = \"standard\"\n";
    assert!(out.contains(block), "the models block changed:\n{out}");
    let agent = out.find("[orchestrator.agent]\n").expect(&out);
    assert!(agent > out.find("max_writers = 2").unwrap(), "{out}");
    assert!(
        out[agent..]
            .starts_with("[orchestrator.agent]\nruntime = \"codex\"\nmodel = \"gpt-6-sol\"\n"),
        "{out}"
    );
    let after = crate::parse(&out).0.orchestrator;
    assert_eq!(doc_of(&after), doc);

    // Back to no runtime: the runtime line goes; the model is written empty.
    let mut back = doc.clone();
    back.orchestrator = proto::OrchestratorDefault {
        runtime: None,
        model: String::new(),
    };
    let again = edit_text(&out, &back).unwrap();
    assert!(
        again.ends_with("[orchestrator.agent]\nmodel = \"\"\n"),
        "{again}"
    );
    assert_eq!(doc_of(&crate::parse(&again).0.orchestrator), back);
}

#[test]
fn a_changed_roster_replaces_every_block_in_place() {
    let text = "[orchestrator]\nmax_writers = 2\n\n# first\n[[orchestrator.models]]\nruntime = \"codex\"\n# inside\nmodel = \"gpt-6-sol\"\nstrength = \"standard\"\n\n# second\n[[orchestrator.models]]\nruntime = \"codex\"\nmodel = \"gpt-6-luna\"\nstrength = \"fast\"\n\n[testing]\ntest_slots = 4\n";
    let mut doc = doc_of(&crate::parse(text).0.orchestrator);
    doc.models = vec![
        model(Runtime::Claude, "claude-sonnet-5", Strength::Standard),
        model(Runtime::Codex, "gpt-6-luna", Strength::Fast),
    ];
    let out = edit_text(text, &doc).unwrap();
    assert_eq!(
        out,
        "[orchestrator]\nmax_writers = 2\nbuiltin_models = false\n\n# first\n[[orchestrator.models]]\nruntime = \"claude\"\nmodel = \"claude-sonnet-5\"\nstrength = \"standard\"\n\n[[orchestrator.models]]\nruntime = \"codex\"\nmodel = \"gpt-6-luna\"\nstrength = \"fast\"\n\n# second\n\n[testing]\ntest_slots = 4\n"
    );
    assert_eq!(doc_of(&crate::parse(&out).0.orchestrator), doc);
}

/// Controller ruling (fix round 1): a roster rewrite never drops a key it does not write.
#[test]
fn a_roster_rewrite_refuses_a_block_key_it_would_drop() {
    let text = "[[orchestrator.models]]\nruntime = \"codex\"\nmodel = \"gpt-6-sol\"\nstrength = \"standard\"\neffort = \"high\"\n";
    let mut doc = doc_of(&crate::parse(text).0.orchestrator);
    doc.limits.max_writers = 4;
    let kept = edit_text(text, &doc).unwrap();
    assert!(
        kept.starts_with(text),
        "an unchanged roster keeps the block: {kept}"
    );
    doc.models
        .push(model(Runtime::Codex, "gpt-6-luna", Strength::Fast));
    assert_eq!(
        edit_text(text, &doc),
        Err(vec![
            "orchestrator.models.effort is a key the settings screen does not edit; edit config.toml by hand, or remove it from [[orchestrator.models]]".to_string()
        ])
    );
}

/// Review probe: header- and key-like lines inside a multi-line basic string are value.
#[test]
fn a_multi_line_string_holding_a_header_and_a_key_is_left_alone() {
    let text = "notes = \"\"\"\n[orchestrator]\nmax_writers = 9\n\"\"\"\n[orchestrator]\nmax_writers = 2\n";
    let mut doc = doc_of(&crate::parse(text).0.orchestrator);
    doc.limits.max_writers = 4;
    let out = edit_text(text, &doc).unwrap();
    assert_eq!(out, text.replace("max_writers = 2", "max_writers = 4"));
    let table: toml::Table = out.parse().unwrap();
    assert_eq!(
        table["notes"].as_str(),
        Some("[orchestrator]\nmax_writers = 9\n")
    );
}

/// Review probe: a literal multi-line string holding a models header survives a roster
/// change; only the real block is replaced.
#[test]
fn a_literal_string_holding_a_models_header_survives_a_roster_change() {
    let text = "notes = '''\n[[orchestrator.models]]\nruntime = \"codex\"\n'''\n\n[[orchestrator.models]]\nruntime = \"codex\"\nmodel = \"gpt-6-sol\"\nstrength = \"standard\"\n";
    let mut doc = doc_of(&crate::parse(text).0.orchestrator);
    doc.models = vec![model(
        Runtime::Claude,
        "claude-opus-5-5",
        Strength::Frontier,
    )];
    let out = edit_text(text, &doc).unwrap();
    assert!(
        out.starts_with("notes = '''\n[[orchestrator.models]]\nruntime = \"codex\"\n'''\n"),
        "{out}"
    );
    assert!(!out.contains("gpt-6-sol"), "{out}");
    assert_eq!(doc_of(&crate::parse(&out).0.orchestrator), doc);
}

/// Review probe: a `#` inside a quoted owned value is not its comment.
#[test]
fn a_hash_inside_a_quoted_value_is_not_a_comment() {
    let text = "[orchestrator.agent]\nruntime = \"claude\"\nmodel = \"a#b\" # c\n";
    let mut doc = doc_of(&crate::parse(text).0.orchestrator);
    assert_eq!(doc.orchestrator.model, "a#b");
    doc.orchestrator.model = "x#y".into();
    let out = edit_text(text, &doc).unwrap();
    assert_eq!(
        out,
        "[orchestrator.agent]\nruntime = \"claude\"\nmodel = \"x#y\" # c\n"
    );
}

/// Review probe: an owned key's name in another table is not the owned key.
#[test]
fn a_same_named_key_in_another_table_is_untouched() {
    let text = "[[profiles]]\nmax_writers = 9\n";
    let mut doc = doc_of(&crate::parse(text).0.orchestrator);
    doc.limits.max_writers = 4;
    let out = edit_text(text, &doc).unwrap();
    assert_eq!(
        out,
        "[[profiles]]\nmax_writers = 9\n\n[orchestrator]\nmax_writers = 4\n"
    );
}

/// Review probe: a multi-line inline table (TOML 1.1) is one value; an owned one refuses.
#[test]
fn a_multi_line_inline_table_is_one_value() {
    let text = "extra = {\n  a = 1,\n  max_writers = 9,\n}\n[orchestrator]\nmax_writers = 2\n";
    assert!(text.parse::<toml::Table>().is_ok());
    let mut doc = doc_of(&crate::parse(text).0.orchestrator);
    doc.limits.max_writers = 4;
    let out = edit_text(text, &doc).unwrap();
    assert_eq!(out, text.replace("max_writers = 2", "max_writers = 4"));

    let owned = "[orchestrator]\nagent = {\n  runtime = \"claude\",\n}\n";
    assert!(owned.parse::<toml::Table>().is_ok());
    assert_eq!(
        edit_text(owned, &doc),
        Err(vec![unsupported(
            "orchestrator.agent",
            "orchestrator.agent"
        )])
    );
}
