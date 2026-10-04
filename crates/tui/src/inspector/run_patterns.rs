//! Milestone 9.5 decision 29 (Interfaces "Run view"): what the inspector shows of a
//! race and a pair. The task's `race` and `pair` rows and its state word, a racer
//! round's `race` field and a test-writer round's `test` field. Pure: each reads the
//! snapshot's `TaskInfo` and returns text, its glyphs through 9.0.7's kit.

use crate::theme::{Glyph, glyph};
use proto::{AgentRole, LaneInfo, LaneState, PairPhase, RaceInfo, RaceLane, TaskInfo, TaskState};

/// A sha's first seven characters, as git abbreviates it.
fn short(sha: &str) -> String {
    sha.chars().take(7).collect()
}

fn lane_of(race: &RaceInfo, lane: RaceLane) -> Option<&LaneInfo> {
    race.lanes.iter().find(|info| info.lane == lane)
}

/// The task's `race` row: `a <runtime> <glyph> · b <runtime> <glyph>`, then
/// ` · won by <lane>` or ` · <lane> adopted` once the race is decided.
pub(super) fn race_row(task: &TaskInfo, ascii: bool) -> Option<String> {
    let race = task.race.as_ref()?;
    let lanes: Vec<String> = race
        .lanes
        .iter()
        .map(|info| {
            let mark = match info.state {
                LaneState::Preparing | LaneState::Working => Glyph::Live,
                LaneState::Proof | LaneState::Check => Glyph::Checking,
                LaneState::Review => Glyph::Review,
                LaneState::Won | LaneState::Adopted => Glyph::Passed,
                LaneState::Lost | LaneState::Out => Glyph::Ended,
            };
            let runtime = info.route.runtime.label();
            format!("{} {runtime} {}", info.lane.label(), glyph(mark, ascii))
        })
        .collect();
    let mut text = lanes.join(" · ");
    match race.winner {
        Some(winner) if race.adopted => text.push_str(&format!(" · {} adopted", winner.label())),
        Some(winner) => text.push_str(&format!(" · won by {}", winner.label())),
        None => {}
    }
    Some(text)
}

/// The task's `pair` row: `test writer <mark>`, ` red <red7>` once there is a red
/// commit, then ` → implementer <mark>`.
pub(super) fn pair_row(task: &TaskInfo, ascii: bool) -> Option<String> {
    let pair = task.pair.as_ref()?;
    let writing = pair.phase == PairPhase::Writing;
    let writer = match task.state {
        TaskState::Blocked if writing => Glyph::Failed,
        _ if writing => Glyph::Live,
        _ => Glyph::Passed,
    };
    let implementer = if task.state == TaskState::Merged {
        Glyph::Passed
    } else if !writing && task.rounds.iter().any(|r| r.role == AgentRole::Worker) {
        Glyph::Live
    } else {
        Glyph::NotStarted
    };
    let mut text = format!("test writer {}", glyph(writer, ascii));
    if let Some(red) = &pair.red {
        text.push_str(&format!(" red {}", short(red)));
    }
    let arrow = if ascii { "->" } else { "→" };
    text.push_str(&format!(
        " {arrow} implementer {}",
        glyph(implementer, ascii)
    ));
    Some(text)
}

/// A racer round's `race` field: where its lane stands, and its salvage once it left
/// the race.
pub(super) fn race_field(task: &TaskInfo, lane: Option<RaceLane>) -> Option<String> {
    let race = task.race.as_ref()?;
    let info = lane_of(race, lane?)?;
    let x = info.lane.label();
    let y = info.lane.other().label();
    let salvage = || match &info.salvage_ref {
        Some(salvage) => format!(" · salvaged {salvage}"),
        None => " · salvage pending".to_owned(),
    };
    Some(match info.state {
        LaneState::Won => format!("lane {x} · won"),
        LaneState::Adopted => format!("lane {x} · adopted after lane {y} went out"),
        LaneState::Lost => {
            let winner = race.winner.map_or(y, RaceLane::label);
            format!("lane {x} · lost to {winner}{}", salvage())
        }
        LaneState::Out => {
            let reason = info.reason.as_deref().unwrap_or("stopped");
            let reason = crate::safe_text::one_line(reason);
            format!("lane {x} · out: {reason}{}", salvage())
        }
        _ => format!("lane {x} · racing"),
    })
}

/// A test-writer round's `test` field: `writing <test>` until it has a red commit,
/// then `<test> · red <red7>` and how the red check went.
pub(super) fn test_field(task: &TaskInfo, ascii: bool) -> Option<String> {
    let pair = task.pair.as_ref()?;
    let test = pair
        .test
        .as_deref()
        .map_or_else(|| "the test".to_owned(), crate::safe_text::one_line);
    let Some(red) = &pair.red else {
        return Some(format!("writing {test}"));
    };
    let mut text = format!("{test} · red {}", short(red));
    match pair.red_checked {
        Some(true) => text.push_str(&format!(" · fails at red {}", glyph(Glyph::Passed, ascii))),
        Some(false) => text.push_str(&format!(" · passed at red {}", glyph(Glyph::Failed, ascii))),
        None => {}
    }
    Some(text)
}

/// The task's state word while a pattern is under way: `racing` while its race has no
/// winner, `writing the test` while its test writer works. `None` otherwise, and for a
/// task not at work (waiting, blocked, finished), which keeps its own word.
pub(super) fn state_word(task: &TaskInfo) -> Option<&'static str> {
    let at_work = matches!(
        task.state,
        TaskState::Preparing
            | TaskState::Working
            | TaskState::Proof
            | TaskState::Check
            | TaskState::Review
    );
    if !at_work {
        return None;
    }
    if task.race.as_ref().is_some_and(|race| race.winner.is_none()) {
        return Some("racing");
    }
    task.pair
        .as_ref()
        .filter(|pair| pair.phase == PairPhase::Writing)
        .map(|_| "writing the test")
}
