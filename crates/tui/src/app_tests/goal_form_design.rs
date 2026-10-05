//! Milestone 9.6 task 18, ruling T18-2: the goal dialog's design row names the settings'
//! default while no mode is chosen, `configured (<mode>)`, and still sends none (T3-1).

use super::goal_form::{app, doc, form, gets, open_form, reply, roster};
use super::*;
use proto::{DesignMode, Origin, SettingsReply};
use std::collections::BTreeMap;

/// An app whose settings fetch was answered with `default` as the design default.
fn app_with_default(default: Option<DesignMode>) -> App {
    let mut app = app();
    let id = gets(&[app.settings_fetch()])[0];
    let current = SettingsReply::Current {
        doc: proto::SettingsDoc {
            design_default: default,
            ..doc(roster())
        },
        origin: BTreeMap::from([("design.default".to_string(), Origin::File)]),
        path: "/cfg/config.toml".into(),
    };
    assert!(app.on_daemon(reply(current, id)).is_empty());
    app
}

fn design_row(app: &App) -> String {
    crate::ui::run_goal::option_lines(form(app), 60, crate::theme::Palette::PLAIN)
        .iter()
        .map(|line| line.to_string())
        .find(|line| line.contains("design"))
        .expect("a design row")
        .trim_end()
        .to_string()
}

#[test]
fn the_design_row_names_the_configured_default() {
    for (default, want) in [
        (Some(DesignMode::Full), "configured (full)"),
        (Some(DesignMode::Off), "configured (off)"),
        (None, "configured"),
    ] {
        let mut app = app_with_default(default);
        open_form(&mut app);
        assert_eq!(form(&app).design, None);
        assert_eq!(
            design_row(&app),
            format!("  design            ‹ {want} ›"),
            "{default:?}"
        );
    }
}

/// A settings reply that lands while the dialog is open names the default there too.
#[test]
fn a_later_settings_reply_names_the_default_in_an_open_dialog() {
    let mut app = app();
    let id = gets(&[app.settings_fetch()])[0];
    open_form(&mut app);
    assert_eq!(design_row(&app), "  design            ‹ configured ›");
    let current = SettingsReply::Current {
        doc: proto::SettingsDoc {
            design_default: Some(DesignMode::Off),
            ..doc(roster())
        },
        origin: BTreeMap::new(),
        path: "/cfg/config.toml".into(),
    };
    let _ = app.on_daemon(reply(current, id));
    assert_eq!(design_row(&app), "  design            ‹ configured (off) ›");
}
