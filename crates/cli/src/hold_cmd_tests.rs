//! Milestone 9.16 decision 34: `brake_lines` in every state, and the request's bound
//! (ruling C34). Pure.

use proto::Runtime;
use proto::brakes::{
    BrakeTripInfo, BrakesInfo, DispatchHold, Permille, QuotaInfo, QuotaState, QuotaWindow,
    QuotaWindowInfo,
};

use super::{HOLD_REQUEST_TIMEOUT, brake_lines};

/// 1970-01-02 14:30 UTC.
const AT: u64 = 86_400 + 14 * 3600 + 30 * 60;
const NOW: u64 = AT + 3 * 60 + 59;

fn window(window: QuotaWindow, used: u16) -> QuotaWindowInfo {
    QuotaWindowInfo {
        window,
        used: Permille(used),
        resets_at: None,
    }
}

fn unmeasured(runtime: Runtime) -> QuotaInfo {
    QuotaInfo {
        runtime,
        state: QuotaState::Unmeasured,
        windows: Vec::new(),
        read_at: None,
        trip: None,
    }
}

fn fresh(runtime: Runtime, five: u16, seven: u16) -> QuotaInfo {
    QuotaInfo {
        runtime,
        state: QuotaState::Fresh,
        windows: vec![
            window(QuotaWindow::FiveHour, five),
            window(QuotaWindow::SevenDay, seven),
        ],
        read_at: Some(AT),
        trip: None,
    }
}

#[test]
fn brake_lines_render_every_state() {
    // Off, and the quota brake disabled.
    let mut info = BrakesInfo::default();
    assert_eq!(brake_lines(&info, NOW), ["hold: off", "quota brake: off"]);

    // On with its `by`; Claude unmeasured, Codex fresh and not braked.
    info.hold = Some(DispatchHold {
        since: AT,
        by: "user".into(),
    });
    info.quota_brake = true;
    info.quota = vec![unmeasured(Runtime::Claude), fresh(Runtime::Codex, 421, 637)];
    assert_eq!(
        brake_lines(&info, NOW),
        [
            "hold: on since 14:30 UTC (user)",
            "claude quota: unmeasured · not braked",
            "codex quota: 5h 42% · 7d 63% (read 3 min ago) · not braked",
        ]
    );

    // Braked, until the trip's time, naming the window that tripped.
    info.hold = None;
    let mut braked = fresh(Runtime::Claude, 921, 500);
    braked.state = QuotaState::Braked;
    braked.trip = Some(BrakeTripInfo {
        window: QuotaWindow::FiveHour,
        used: Permille(921),
        threshold_percent: 90,
        until: AT + 3600,
    });
    info.quota = vec![braked, unmeasured(Runtime::Codex)];
    assert_eq!(
        brake_lines(&info, NOW),
        [
            "hold: off",
            "claude quota: 5h 92% · 7d 50% (read 3 min ago) · braked until 15:30 UTC (5h at 92% ≥ 90%)",
            "codex quota: unmeasured · not braked",
        ]
    );
}

/// Ruling C34: the daemon writes `brakes.json` within `BRAKES_IO_WAIT` (5 s, the
/// client's default bound), so `hold` waits the run requests' bound.
#[test]
fn hold_waits_the_run_request_timeout() {
    assert_eq!(HOLD_REQUEST_TIMEOUT, crate::run_cmd::RUN_REQUEST_TIMEOUT);
    assert!(HOLD_REQUEST_TIMEOUT > std::time::Duration::from_secs(5));
}

/// Task 10's review I1: `hold off` names the queued goals it releases, only when it
/// released a hold and goals are queued.
#[test]
fn hold_off_names_the_queued_goals_it_starts() {
    use super::queued_line;
    assert_eq!(
        queued_line(true, 3).as_deref(),
        Some("3 queued goals will start")
    );
    assert_eq!(
        queued_line(true, 1).as_deref(),
        Some("1 queued goal will start")
    );
    assert_eq!(queued_line(true, 0), None);
    // Not held: `hold off` released nothing, so it starts nothing new.
    assert_eq!(queued_line(false, 3), None);
}
