//! Milestone 9.16 decision 31 (ruling C23): the brakes' alerts, the daemon's, from
//! `RunsSnapshot.brakes`: one for the hold, one per quota-braked runtime, each at
//! priority 2 and always the user's (`RouteKind::DispatchHeld`). The quota text is the
//! daemon's own attention text (`common::brakes_text`, ruling C13). Mounted from
//! `alerts.rs`. Pure: no I/O.

use super::{Alert, AlertKey, AlertWho, DispatchHeldKind};
use crate::app::App;
use crate::safe_text::one_line;

/// Decision 31: the hold's alert, `dispatch held daemon-wide · <prefix> H releases it`.
pub(crate) fn hold_text(prefix: &str) -> String {
    format!(
        "dispatch held daemon-wide \u{b7} {} H releases it",
        one_line(prefix)
    )
}

pub(super) fn brake_alerts(app: &App, out: &mut Vec<Alert>) {
    let brakes = &app.runs.brakes;
    let mut push = |kind, text: String, age| {
        out.push(Alert {
            priority: 2,
            key: AlertKey::DispatchHeld(kind),
            who: AlertWho::Daemon,
            task: None,
            detail: text.clone(),
            text,
            age,
            you: false,
        });
    };
    if let Some(hold) = &brakes.hold {
        let text = hold_text(&app.settings.prefix_label);
        push(DispatchHeldKind::Hold, text, Some(app.run_age(hold.since)));
    }
    for quota in &brakes.quota {
        if let Some(trip) = &quota.trip {
            let text = common::brakes_text::quota_held_text(quota.runtime, trip);
            push(DispatchHeldKind::Quota(quota.runtime), text, None);
        }
    }
}
