//! M9.0.7.12: the grouped help's rows and its render (decision 34).

use super::*;
use crate::app::Modal;
use crate::settings::UiSettings;
use crate::ui::audit;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

fn app_with(prefix: &str, scroll: u16) -> App {
    let settings = UiSettings {
        prefix_label: prefix.into(),
        ..UiSettings::default()
    };
    let mut app = App::new(vec![], "/tmp".into(), settings);
    app.modal = Some(Modal::Help(HelpView {
        scroll,
        ..HelpView::default()
    }));
    app
}

fn drawn(app: &App, w: u16, h: u16) -> Vec<String> {
    audit::rows(&audit::draw(app, w, h))
}

/// A row as the help draws it: indented, the key padded to the widest key (11
/// columns: `PgUp / PgDn`), two spaces, the words.
fn row_text(key: &str, words: &str) -> String {
    format!("  {key:<11}  {words}")
}

#[test]
fn groups_are_the_specs_in_order() {
    let names: Vec<&str> = help_groups("C-b").iter().map(|g| g.name).collect();
    assert_eq!(
        names,
        [
            "global",
            "sidebar",
            "run view",
            "plan review",
            "document gate",
            "alerts",
            "conversation",
            "action menu",
            "profile",
            "settings",
        ]
    );
}

fn group(groups: &[HelpGroup], name: &str) -> Vec<(String, String)> {
    groups
        .iter()
        .find(|g| g.name == name)
        .unwrap_or_else(|| panic!("no group {name}"))
        .rows
        .clone()
}

/// Every prefix key of 9.0.6's `help_lines` (the keymap's own, `keymap.rs:170-196`),
/// the three of 9.0.6 decision 43 and `<prefix> ?`, all with the configured prefix.
#[test]
fn global_lists_every_prefix_key() {
    let groups = help_groups("C-a");
    let global = group(&groups, "global");
    let want = [
        ("C-a j / k", "next / previous agent"),
        ("C-a 1-9", "focus agent by number"),
        ("C-a c", "new agent"),
        ("C-a ,", "rename agent"),
        ("C-a R", "restart agent"),
        ("C-a x", "kill agent"),
        ("C-a X", "remove agent (and worktree)"),
        ("C-a m", "conversation"),
        ("C-a t", "tree"),
        ("C-a T", "overview"),
        ("C-a g", "start a goal"),
        ("C-a a", "alerts"),
        ("C-a P", "profile"),
        ("C-a S", "settings"),
        ("C-a s", "toggle sidebar"),
        ("C-a < / >", "sidebar width"),
        ("C-a r", "reconnect"),
        ("C-a d", "detach (agents keep running)"),
        ("C-a Q", "stop daemon and all agents"),
        ("C-a C-a", "send a literal C-a"),
        ("C-a ?", "this help"),
    ];
    let want: Vec<(String, String)> = want
        .iter()
        .map(|(k, w)| (k.to_string(), w.to_string()))
        .collect();
    assert_eq!(global, want);
    // Every key that names the prefix is in `global`, and no other group names it.
    for g in &groups[1..] {
        assert!(
            g.rows.iter().all(|(k, _)| !k.contains("C-a")),
            "{}: {:?}",
            g.name,
            g.rows
        );
    }
}

/// Milestone 9.0.5 (moved from `ui/modal.rs`): the help lists `C-b a`, with the
/// configured prefix.
#[test]
fn help_lists_the_alerts_key() {
    let global = group(&help_groups("C-a"), "global");
    assert!(
        global.contains(&("C-a a".to_owned(), "alerts".to_owned())),
        "{global:?}"
    );
}

/// Milestone 9.0.6 decision 43 (moved from `ui/modal.rs`): the three new keys, with the
/// configured prefix; `.` now in the sidebar's and the run view's groups (decision 34).
#[test]
fn help_lists_the_new_keys() {
    let groups = help_groups("C-a");
    let dot = (
        ".".to_owned(),
        "actions on the selected run, stage or task".to_owned(),
    );
    for name in ["sidebar", "run view"] {
        assert!(group(&groups, name).contains(&dot), "{name}");
    }
    let global = group(&groups, "global");
    for (key, what) in [("C-a P", "profile"), ("C-a S", "settings")] {
        assert!(
            global.contains(&(key.to_owned(), what.to_owned())),
            "{key}: {global:?}"
        );
    }
    let text = drawn(&app_with("C-a", 0), 100, 30).join("\n");
    assert!(!text.contains("C-b"), "{text}");
}

/// The first page at each size, exactly: the `keys` title, `global` bold, the aligned
/// key column, the `↓ n more` mark, the hint row.
#[test]
fn the_help_renders_at_80x24_and_120x40() {
    let app = app_with("C-b", 0);
    let rows = drawn(&app, 80, 24);
    let inside = |rows: &[String], at: usize| -> Vec<String> {
        rows.iter()
            .map(|r| {
                r.chars()
                    .skip(at)
                    .take(64)
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect()
    };
    let mut want = vec![format!("┌ keys {}┐", "─".repeat(56))];
    let mut body = vec!["global".to_string()];
    for (k, w) in &group(&help_groups("C-b"), "global")[..19] {
        body.push(row_text(k, w));
    }
    body.push("↓ 96 more".into());
    body.push("j/k scroll · tab group · esc close".into());
    for line in &body {
        want.push(format!("│ {line:<60} │"));
    }
    want.push(format!("└{}┘", "─".repeat(62)));
    assert_eq!(inside(&rows, 8), want, "{rows:#?}");
    let buffer = audit::draw(&app, 80, 24);
    let bold = buffer[(10, 1)].modifier.contains(Modifier::BOLD);
    assert!(bold, "the group name is bold");
    let accent = role(Role::Accent, app.palette()).fg;
    assert_eq!(
        buffer[(12, 2)].fg,
        accent.unwrap(),
        "the key is in the accent"
    );
    assert_eq!(
        buffer[(25, 2)].fg,
        ratatui::style::Color::Reset,
        "the words are not"
    );

    let rows = drawn(&app, 120, 40);
    let mut want = vec![format!("┌ keys {}┐", "─".repeat(56))];
    let groups = help_groups("C-b");
    let mut body = vec!["global".to_string()];
    body.extend(group(&groups, "global").iter().map(|(k, w)| row_text(k, w)));
    body.push("sidebar".into());
    body.extend(
        group(&groups, "sidebar")
            .iter()
            .map(|(k, w)| row_text(k, w)),
    );
    body.push("run view".into());
    body.extend(
        group(&groups, "run view")[..5]
            .iter()
            .map(|(k, w)| row_text(k, w)),
    );
    body.push("↓ 80 more".into());
    body.push("j/k scroll · tab group · esc close".into());
    for line in &body {
        want.push(format!("│ {line:<60} │"));
    }
    want.push(format!("└{}┘", "─".repeat(62)));
    assert_eq!(inside(&rows, 28), want, "{rows:#?}");
}

/// One dialog, 64 columns wide, centred, and the one accented frame.
#[test]
fn the_help_is_one_dialog() {
    let app = app_with("C-b", 0);
    for (w, h) in [(80, 24), (120, 40)] {
        let buffer = audit::draw(&app, w, h);
        let x = (w - 64) / 2;
        assert_eq!(buffer[(x, 0)].symbol(), "┌", "{w}x{h}");
        assert_eq!(buffer[(x + 63, 0)].symbol(), "┐", "{w}x{h}");
        assert_eq!(audit::accented_frames(&buffer, app.palette()), 1, "{w}x{h}");
    }
}

/// Replaces 9.0.6's fit-at-80×24 guarantee: `j` from the top reaches every row of every
/// group, and the last press shows the last group's last row; nothing says `any key`.
#[test]
fn every_help_row_is_reachable_at_80x24() {
    let mut app = app_with("C-a", 0);
    app.set_body_area(ratatui::layout::Rect::new(0, 0, 80, 23));
    let mut seen: Vec<String> = Vec::new();
    let mut last_rows = drawn(&app, 80, 24);
    seen.extend(last_rows.iter().cloned());
    loop {
        let before = app.modal.clone();
        app.on_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE));
        if app.modal == before {
            break;
        }
        last_rows = drawn(&app, 80, 24);
        seen.extend(last_rows.iter().cloned());
    }
    let groups = help_groups("C-a");
    for g in &groups {
        assert!(
            seen.iter().any(|r| r.contains(&format!("│ {}", g.name))),
            "{}",
            g.name
        );
        for (k, w) in &g.rows {
            let row = row_text(k, w);
            assert!(seen.iter().any(|r| r.contains(&row)), "{row:?}");
        }
    }
    let (k, w) = groups.last().unwrap().rows.last().unwrap();
    let last = row_text(k, w);
    assert!(
        last_rows.iter().any(|r| r.contains(&last)),
        "{last_rows:#?}"
    );
    assert!(!seen.concat().contains("any key"));
}

#[test]
fn help_in_ascii() {
    let mut app = app_with("C-b", 0);
    app.settings.badges.ascii = true;
    app.set_body_area(ratatui::layout::Rect::new(0, 0, 80, 23));
    let groups = help_groups("C-b");
    let area = ratatui::layout::Rect::new(0, 0, 80, 24);
    let top = max_scroll(&groups, &HelpView::default(), area, app.palette());
    for scroll in [0, 30, top as u16] {
        app.modal = Some(Modal::Help(HelpView {
            scroll,
            ..HelpView::default()
        }));
        let buffer = audit::draw(&app, 80, 24);
        assert_eq!(
            audit::first_non_ascii(&buffer),
            None,
            "{:#?}",
            audit::rows(&buffer)
        );
    }
    let rows = drawn(&app, 80, 24).join("\n");
    assert!(rows.contains("+ keys ---"), "{rows}");
    assert!(rows.contains("^ 96 more"), "{rows}");
    assert!(
        rows.contains("  <- / ->      scope: everywhere / this repo"),
        "{rows}"
    );
    assert!(
        rows.contains("  enter        choose the row's model"),
        "{rows}"
    );
    assert!(
        rows.contains("  f            its fallback (if it struggles)"),
        "{rows}"
    );
    assert!(!rows.contains("strength"), "{rows}");
    assert!(
        rows.contains("j/k scroll - tab group - esc close"),
        "{rows}"
    );
}

#[test]
fn no_panic_at_tiny_sizes() {
    for scroll in [0, 50, u16::MAX] {
        let app = app_with("C-b", scroll);
        for (w, h) in [(20, 5), (1, 1), (5, 40), (40, 3)] {
            let _ = audit::draw(&app, w, h);
        }
    }
}

/// Task 7's handover (decision 33's help half): at 40 columns the hint row keeps
/// `esc close`, and every help fixture of the audit draws one accented frame and, in
/// ASCII, nothing but ASCII, at 80×24 and 40 columns.
#[test]
fn the_help_keeps_esc_and_one_accent_at_40_columns() {
    let helps: Vec<_> = audit::fixtures()
        .into_iter()
        .filter(|(name, _)| name.contains("help"))
        .collect();
    assert!(helps.len() >= 2, "the audit has help fixtures");
    let sizes = [(80, 24), (40, 24), (40, 12), (30, 10)];
    for (name, mut app) in helps {
        for (w, h) in sizes {
            let buffer = audit::draw(&app, w, h);
            let rows = audit::rows(&buffer).join("\n");
            let frames = audit::accented_frames(&buffer, app.palette());
            assert_eq!(frames, 1, "{name} {w}x{h}\n{rows}");
            let esc = audit::find(&buffer, "esc close");
            assert!(!esc.is_empty(), "{name} {w}x{h}\n{rows}");
        }
        app.settings.badges =
            crate::ui::badge::BadgeSet::from_config(&app.settings.badges_config, true);
        for (w, h) in sizes {
            let buffer = audit::draw(&app, w, h);
            let rows = audit::rows(&buffer).join("\n");
            assert_eq!(
                audit::first_non_ascii(&buffer),
                None,
                "{name} {w}x{h}\n{rows}"
            );
        }
    }
    let rows = drawn(&app_with("C-b", 0), 40, 24);
    assert!(
        rows[22].contains("j/k scroll · tab group · esc close"),
        "{rows:#?}"
    );
    let rows = drawn(&app_with("C-b", 0), 30, 24);
    assert!(rows[22].contains("j/k scroll · esc close"), "{rows:#?}");
}

/// Global Constraint 4: the prefix label is configuration, drawn through `one_line`;
/// hostile characters planted inside its visible columns never reach the screen. The
/// help alone is drawn (the sidebar and pane hints under it are not this renderer's).
#[test]
fn the_prefix_label_is_sanitised() {
    let app = app_with("C-\u{202E}x\u{200D}y\u{200B}", 0);
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 30)).unwrap();
    terminal
        .draw(|frame| render(frame, &app, &HelpView::default(), frame.area()))
        .unwrap();
    let text = audit::rows(terminal.backend().buffer()).join("\n");
    let row = row_text("C-xy j / k", "next / previous agent");
    assert!(text.contains(&row), "{text}");
    assert!(
        text.contains(&row_text("C-xy C-xy", "send a literal C-xy")),
        "{text}"
    );
    let cells = text.replace('\n', "");
    assert_eq!(
        crate::safe_text::tests::first_hostile(&cells),
        None,
        "{text:?}"
    );
}

/// The help names real keys: with `C-a` as the prefix, every key of `global` is one
/// the keymap runs a command for (`keymap.rs`), and `C-a C-a` sends the prefix.
#[test]
fn every_global_key_is_a_prefix_command() {
    use crate::keymap::{KeyAction, Keymap};
    let ctrl_a = KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL);
    for (key, words) in group(&help_groups("C-a"), "global") {
        let rest = key.strip_prefix("C-a ").expect("names the prefix");
        let presses: Vec<KeyEvent> = match rest {
            "C-a" => vec![ctrl_a],
            "1-9" => ['1', '9'].map(|c| KeyEvent::from(KeyCode::Char(c))).into(),
            _ => rest
                .split(" / ")
                .map(|k| KeyEvent::from(KeyCode::Char(k.chars().next().unwrap())))
                .collect(),
        };
        for press in presses {
            let mut keymap = Keymap::new((KeyCode::Char('a'), KeyModifiers::CONTROL));
            assert_eq!(keymap.handle(ctrl_a, false), KeyAction::AwaitPrefix);
            let action = keymap.handle(press, false);
            let ran = matches!(action, KeyAction::Run(_))
                || (rest == "C-a" && action == KeyAction::Send(vec![0x01]));
            assert!(ran, "{key} ({words}): {action:?}");
        }
    }
}

/// Task M9.6.18, ruling T18-4: the plan review's group lists a design plan gate's Tab
/// (the Plan doc tab), `c` and `b`.
#[test]
fn the_plan_review_group_lists_the_design_gate_keys() {
    let review = group(&help_groups("C-b"), "plan review");
    for key in ["tab", "c", "b"] {
        assert!(review.iter().any(|(k, _)| k == key), "{key}: {review:?}");
    }
}

/// Final fix wave FW-76 (WB-D m2, 9.0.7 decision 34): the document gate screen has its
/// own group, listing every key its handler takes, and the run view's group names the
/// keys that open it at a brainstorm or spec gate.
#[test]
fn the_document_gate_group_lists_its_keys() {
    let groups = help_groups("C-b");
    let gate = group(&groups, "document gate");
    let keys: Vec<&str> = gate.iter().map(|(k, _)| k.as_str()).collect();
    assert_eq!(
        keys,
        [
            "a", "c", "e", "r", "b", "x", "d", "f", "g", "j / k", "q / esc"
        ]
    );
    let run_view = group(&groups, "run view");
    assert!(
        run_view.contains(&(
            "a / p".to_string(),
            "review the document (brainstorm or spec gate)".to_string()
        )),
        "{run_view:?}"
    );
}
