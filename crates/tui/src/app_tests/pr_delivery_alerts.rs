//! Milestone 9.2 ruling R-13 (task M9.2.15's fix round): a `pr` run's delivery alerts
//! at priority 3, from the snapshot's typed `DeliveryInfo.alerts`, never from its
//! attention text; Enter on a held op preselects `resume`; the Alerts view's detail
//! names the kind; the host's text is cleaned where it is drawn.

use super::actions::{action, flow};
use super::alerts::listed;
use super::runs::app_with_runs;
use super::*;
use crate::app::AlertKey;
use crate::safe_text::tests::first_hostile;
use crate::tree::pr_fixtures::pr_fixture;
use crate::tree::run_fixtures::RUN_ID;
use proto::{ActionKind, DeliveryAlert, DeliveryAlertKind, RunsSnapshot};

fn alert(kind: DeliveryAlertKind, stage: Option<u16>, text: &str) -> DeliveryAlert {
    DeliveryAlert {
        kind,
        stage,
        text: text.into(),
        user_only: false,
    }
}

/// The `pr` fixture with one alert of every kind (two over the cap on stage 2), an
/// attention line for unaddressed threads, and `resume` and `cancel run` listed.
fn with_alerts(alerts: Vec<DeliveryAlert>) -> (RunsSnapshot, Vec<WindowInfo>) {
    let (mut snap, windows) = pr_fixture();
    let run = &mut snap.runs[0];
    run.attention = vec!["PR #142: 2 threads not addressed".into()];
    run.actions = vec![
        action(ActionKind::Cancel, "cancel run", None),
        action(ActionKind::Resume, "resume", None),
    ];
    run.delivery.as_mut().expect("pr mode").alerts = alerts;
    (snap, windows)
}

fn every_kind() -> Vec<DeliveryAlert> {
    use DeliveryAlertKind::*;
    vec![
        alert(GhLoggedOut, None, "gh is logged out: run gh auth login"),
        alert(CiHandedToUser, Some(2), "PR #142: CI test handed to you"),
        alert(
            ReviewRoundsOverCap,
            Some(2),
            "PR #142: thread a over the cap",
        ),
        alert(
            ReviewRoundsOverCap,
            Some(2),
            "PR #142: thread b over the cap",
        ),
        alert(HostOpHeld, Some(3), "PR #143: push keeps failing: boom"),
        alert(
            PrClosedUnmerged,
            Some(1),
            "PR #141 was closed without merging",
        ),
    ]
}

fn app_of(alerts: Vec<DeliveryAlert>) -> App {
    let (snap, windows) = with_alerts(alerts);
    app_with_runs(windows, snap)
}

fn keys(app: &App) -> Vec<AlertKey> {
    crate::app::alerts::alerts(app)
        .into_iter()
        .map(|a| a.key)
        .collect()
}

fn delivery_key(kind: DeliveryAlertKind, stage: Option<u16>, n: usize) -> AlertKey {
    AlertKey::Delivery {
        run: RUN_ID.into(),
        kind,
        stage,
        n,
    }
}

/// Each typed alert is one priority-3 alert with its text, in the daemon's order; the
/// unaddressed threads' attention line is not one; two of one kind and stage are two.
#[test]
fn each_delivery_alert_is_a_priority_3_alert() {
    let app = app_of(every_kind());
    let want: Vec<(u8, String, String)> = every_kind()
        .into_iter()
        .map(|a| (3, RUN_ID.to_owned(), a.text))
        .collect();
    assert_eq!(listed(&app), want);
    let cap = |n| delivery_key(DeliveryAlertKind::ReviewRoundsOverCap, Some(2), n);
    let keys = keys(&app);
    assert!(keys.contains(&cap(0)) && keys.contains(&cap(1)), "{keys:?}");
    // The attention text alone raises nothing, whatever it says.
    let mut app = app_of(Vec::new());
    if let Some(run) = app.runs.runs.first_mut() {
        run.attention
            .push("gh is logged out: run gh auth login".into());
    }
    assert_eq!(listed(&app), []);
}

/// A local run raises none, whatever its snapshot carries.
#[test]
fn a_local_run_raises_no_delivery_alert() {
    let mut app = app_of(every_kind());
    if let Some(d) = app.runs.runs[0].delivery.as_mut() {
        d.mode = proto::DeliveryMode::Local;
    }
    assert_eq!(listed(&app), []);
}

/// `C-b a`, then `j` until `key` is selected.
fn select_alert(app: &mut App, key: &AlertKey) {
    prefix(app);
    press(app, KeyCode::Char('a'), KeyModifiers::NONE);
    for _ in 0..20 {
        let selected = app.alerts_focus.as_ref().and_then(|f| f.selected.clone());
        if selected.as_ref() == Some(key) {
            return;
        }
        press(app, KeyCode::Char('j'), KeyModifiers::NONE);
    }
    panic!("{key:?} is not listed");
}

/// Enter on a held op opens the run's menu on `resume`; on the others, the first entry
/// (the user acts on GitHub or with `gh`).
#[test]
fn enter_on_a_held_op_preselects_resume() {
    for (kind, stage, want) in [
        (DeliveryAlertKind::HostOpHeld, Some(3), ActionKind::Resume),
        (DeliveryAlertKind::GhLoggedOut, None, ActionKind::Cancel),
        (
            DeliveryAlertKind::CiHandedToUser,
            Some(2),
            ActionKind::Cancel,
        ),
    ] {
        let mut app = app_of(every_kind());
        select_alert(&mut app, &delivery_key(kind, stage, 0));
        assert!(press(&mut app, KeyCode::Enter, KeyModifiers::NONE).is_empty());
        let f = flow(&app);
        assert_eq!(f.run_id, RUN_ID, "{kind:?}");
        assert_eq!(f.items[f.selected].kind, want, "{kind:?}");
    }
}

/// The Alerts view's detail names the kind and the stage, at 80×24 and 120×40, in
/// Unicode and ASCII; the host's text planted with a ZWJ, bidi controls and an escape
/// is cleaned in the list and the detail.
#[test]
fn the_alerts_view_names_a_delivery_alert_and_cleans_its_text() {
    let hostile = alert(
        DeliveryAlertKind::HostOpHeld,
        Some(3),
        "PR #143: pu\u{200D}sh keeps\u{202E} failing: bo\u{1b}[2Jom\nnext",
    );
    for ascii in [false, true] {
        let mut app = app_of(vec![hostile.clone()]);
        app.settings.badges.ascii = ascii;
        select_alert(
            &mut app,
            &delivery_key(DeliveryAlertKind::HostOpHeld, Some(3), 0),
        );
        let phase = if ascii {
            "delivery - held"
        } else {
            "delivery · held"
        };
        for (w, h) in [(80, 24), (120, 40)] {
            let rows = crate::ui::audit::rows(&crate::ui::audit::draw(&app, w, h));
            let text = rows.join("\n");
            assert!(text.contains(phase), "{w}x{h}:\n{text}");
            assert!(text.contains("3 of 3"), "{w}x{h}:\n{text}");
            assert!(
                text.contains("PR #143: push keeps failing"),
                "{w}x{h}:\n{text}"
            );
            for row in &rows {
                assert_eq!(first_hostile(row), None, "{w}x{h}: {row:?}");
                assert!(!ascii || row.is_ascii(), "{w}x{h}: {row:?}");
            }
        }
    }
}
