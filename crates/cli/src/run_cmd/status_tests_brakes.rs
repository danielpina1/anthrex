//! Milestone 9.16 decision 34: `anthrex run status` prints the brake lines first while
//! dispatch is held or a runtime is braked, and exactly what it printed before otherwise.

use common::numbers::Zone;
use proto::Runtime;
use proto::brakes::{
    BrakeTripInfo, BrakesInfo, DispatchHold, Permille, QuotaInfo, QuotaState, QuotaWindow,
};

use super::tests::example;
use super::{listing, render};

fn snapshot(brakes: BrakesInfo) -> proto::RunsSnapshot {
    proto::RunsSnapshot {
        revision: 1,
        runs: vec![example()],
        now: 100_000,
        proposals: Vec::new(),
        idle_orchestrators: Vec::new(),
        queued_goals: Vec::new(),
        brakes,
    }
}

fn quota(runtime: Runtime, trip: Option<BrakeTripInfo>) -> QuotaInfo {
    QuotaInfo {
        runtime,
        state: QuotaState::Unmeasured,
        windows: Vec::new(),
        read_at: None,
        trip,
    }
}

#[test]
fn run_status_prints_brake_lines_when_held_or_braked() {
    let plain = render(&[example()], 0, Zone::Local(0));
    // Nothing held, the brake on and nothing braked: the text is unchanged.
    let calm = BrakesInfo {
        hold: None,
        quota_brake: true,
        quota: vec![quota(Runtime::Claude, None), quota(Runtime::Codex, None)],
    };
    assert_eq!(listing(&snapshot(calm.clone()), Zone::Local(0)), plain);
    assert_eq!(
        listing(&snapshot(BrakesInfo::default()), Zone::Local(0)),
        plain
    );

    // Held: the brake lines, a blank line, then the runs.
    let mut held = calm.clone();
    held.hold = Some(DispatchHold {
        since: 86_400 + 14 * 3600 + 30 * 60,
        by: "user".into(),
    });
    let text = listing(&snapshot(held), Zone::Local(0));
    let head = "hold: on since 14:30 +00:00 (user)\nclaude quota: unmeasured · not braked\ncodex quota: unmeasured · not braked\n\n";
    assert_eq!(text, format!("{head}{plain}"));

    // Braked, with no hold.
    let mut braked = calm;
    braked.quota[1].state = QuotaState::Braked;
    braked.quota[1].read_at = Some(99_880);
    braked.quota[1].windows = vec![proto::brakes::QuotaWindowInfo {
        window: QuotaWindow::SevenDay,
        used: Permille(960),
        resets_at: Some(86_400),
    }];
    braked.quota[1].trip = Some(BrakeTripInfo {
        window: QuotaWindow::SevenDay,
        used: Permille(960),
        threshold_percent: 95,
        until: 86_400,
    });
    let text = listing(&snapshot(braked), Zone::Local(0));
    assert!(text.starts_with("hold: off\n"), "{text}");
    assert!(text.ends_with(&plain), "{text}");
    assert!(
        text.contains(
            "codex quota: 7d 96% (read 2 min ago) · braked until 00:00 +00:00 (7d at 96% ≥ 95%)\n"
        ),
        "{text}"
    );
}
