//! M8c.8: the run inspection's one-field-per-row layout (decision 29), drawn into a
//! real terminal buffer and compared cell by cell with the Interfaces mockups.

use super::super::render;
use crate::inspector::run_tests::{app_of, inspect_node};
use crate::inspector::{Field, FieldLayout, Inspection, RUN_INSPECTOR_HEIGHT};
use crate::tree::NodeKey;
use crate::tree::run_fixtures::{gemini_fixture, planner_fixture};
use proto::AgentRole;
use ratatui::backend::TestBackend;
use ratatui::text::Span;
use ratatui::{Terminal, layout::Rect};
use unicode_width::UnicodeWidthStr;

/// One string per row of the buffer, as a reader sees it: the cell a wide character
/// spills into is skipped, so a correct row is exactly `width` columns wide.
fn panel(inspection: &Inspection, width: u16, height: u16) -> Vec<String> {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| render(frame, inspection, Rect::new(0, 0, width, height)))
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    (0..height)
        .map(|y| {
            let mut row = String::new();
            let mut skip = 0;
            for x in 0..width {
                if skip > 0 {
                    skip -= 1;
                    continue;
                }
                let symbol = buffer[(x, y)].symbol();
                skip = UnicodeWidthStr::width(symbol).saturating_sub(1);
                row.push_str(symbol);
            }
            row
        })
        .collect()
}

fn node(fixture: (proto::RunsSnapshot, Vec<proto::WindowInfo>), key: NodeKey) -> Inspection {
    inspect_node(&app_of(fixture), &key)
}

fn run_key(id: &str) -> NodeKey {
    NodeKey::Run(id.into())
}

fn task_key(id: &str) -> NodeKey {
    NodeKey::Task {
        run: "r1".into(),
        id: id.into(),
    }
}

fn round_key(role: AgentRole, session: u32, round: u32) -> NodeKey {
    NodeKey::AgentRound {
        run: "r1".into(),
        task: "t2".into(),
        role,
        session,
        round,
    }
}

fn scout_key() -> NodeKey {
    NodeKey::Scout {
        run: "r1".into(),
        id: "S1".into(),
    }
}

fn rows(name: &str, right: Option<&str>, fields: Vec<Field>) -> Inspection {
    Inspection {
        glyph: Span::raw("●"),
        name: name.to_owned(),
        right: right.map(str::to_owned),
        fields,
        layout: FieldLayout::Rows,
    }
}

fn plain(label: &'static str, value: &str) -> Field {
    Field {
        label,
        value: value.to_owned(),
        wrap: false,
    }
}

fn wrapping(label: &'static str, value: &str) -> Field {
    Field {
        wrap: true,
        ..plain(label, value)
    }
}

const RUN: [&str; 12] = [
    "╭────────────────────────────────────────────────────────────────────────────────────╮",
    "│ ◉ r1  Add Gemini runtime                                           running · 1h12m │",
    "│ progress  ██████████░░░░░░░░  5/9 merged · 2 working · 1 review · 1 blocked        │",
    "│ path      critical path t0 → t6 → t2 → t3 · 2 tasks left · 1.2× the bound          │",
    "│ agents    workers 3/3 · readers 1/3 · claude ok · codex rate-limited 4m            │",
    "│ spend     tokens 1.8M (cache 71%) · tool calls 612 · est. left ~40m                │",
    "│ gate      plan approved 11:02 · 2 plan edits since · last: split t2                │",
    "│ attention t5 blocked: question — \"Gemini has no subagent-stop event\"               │",
    "│                                                                                    │",
    "│                                                                                    │",
    "│                                                                                    │",
    "╰────────────────────────────────────────────────────────────────────────────────────╯",
];

const PLANNER: [&str; 12] = [
    "╭────────────────────────────────────────────────────────────────────────────────────╮",
    "│ ✓ planner A  daemon                               finished · planned 3 tasks in 2m │",
    "│ progress  ███░░░░░░░  1/3 merged · 1 working · 1 waiting                           │",
    "│ area      crates/daemon/** · crates/cli/src/hook.rs                                │",
    "│ edits     3 accepted · 1 rejected (owns outside area) · re-planned once (t2 split) │",
    "│                                                                                    │",
    "│                                                                                    │",
    "│                                                                                    │",
    "│                                                                                    │",
    "│                                                                                    │",
    "│                                                                                    │",
    "╰────────────────────────────────────────────────────────────────────────────────────╯",
];

const TASK: [&str; 12] = [
    "╭────────────────────────────────────────────────────────────────────────────────────╮",
    "│ ◐ t2  map Gemini hook events to status                    M · tdd · review round 2 │",
    "│ stages    done ✓ → proof ✓ → check ✓ → review ● → merge ·                          │",
    "│ route     codex · standard · high effort  →  reviewer claude · frontier            │",
    "│ deps      waits on t0 ✓ t6 ✓ · unblocks t3, t7 · on critical path                  │",
    "│ budget    ███████░░░ 104/150 tool calls · 38/60 min · 410k tokens                  │",
    "│ tries     review 1/2 bounces · check 0/2 · escalation step 1                       │",
    "│ diff      4 files · +212 −31 · test `status::gemini_stop_marks_idle` red a1b2c3d ✓ │",
    "│ review    r1 ✗ 1 critical, 2 minor: status.rs:118 \"SubagentStop not paired\"        │",
    "│ history   12:31 review r1 changes · 12:20 check passed · 12:02 started             │",
    "│                                                                                    │",
    "╰────────────────────────────────────────────────────────────────────────────────────╯",
];

const TASK_NARROW: [&str; 12] = [
    "╭──────────────────────────────────────────────────────────╮",
    "│ ◐ t2  map Gemini hook events to status                   │",
    "│ stages    done ✓ → proof ✓ → check ✓ → review ● → merge… │",
    "│ route     codex · standard · high effort  →  reviewer c… │",
    "│ deps      waits on t0 ✓ t6 ✓ · unblocks t3, t7 · on cri… │",
    "│ budget    ███████░░░ 104/150 tool calls · 38/60 min · 4… │",
    "│ tries     review 1/2 bounces · check 0/2 · escalation s… │",
    "│ diff      4 files · +212 −31 · test `status::gemini_sto… │",
    "│ review    r1 ✗ 1 critical, 2 minor: status.rs:118 \"Suba… │",
    "│ history   12:31 review r1 changes · 12:20 check passed … │",
    "│                                                          │",
    "╰──────────────────────────────────────────────────────────╯",
];

const WORKER: [&str; 12] = [
    "╭────────────────────────────────────────────────────────────────────────────────────╮",
    "│ ⠋ worker #1 r2  codex · standard · high                          working · 6m · t2 │",
    "│ doing     last tool: apply_patch                                                   │",
    "│ activity  turns 14 · tool calls 41 · tokens 180k                                   │",
    "│ fixing    status.rs:118 critical — SubagentStop not paired                         │",
    "│ session   #7 · headless · worktree anthrex/r1/t2 · Enter: conversation             │",
    "│                                                                                    │",
    "│                                                                                    │",
    "│                                                                                    │",
    "│                                                                                    │",
    "│                                                                                    │",
    "╰────────────────────────────────────────────────────────────────────────────────────╯",
];

const REVIEWER: [&str; 12] = [
    "╭────────────────────────────────────────────────────────────────────────────────────╮",
    "│ ✗ review #1  claude · frontier · high                           finished · 9m · t2 │",
    "│ judging   worker #1 · codex · standard                                             │",
    "│ strength  frontier vs author standard                                              │",
    "│ verdict   changes (blocking)                                                       │",
    "│ findings  1 critical, 2 minor · status.rs:118 critical — SubagentStop not paired   │",
    "│ session   #9 · window closed                                                       │",
    "│                                                                                    │",
    "│                                                                                    │",
    "│                                                                                    │",
    "│                                                                                    │",
    "╰────────────────────────────────────────────────────────────────────────────────────╯",
];

const SCOUT: [&str; 12] = [
    "╭──────────────────────────────────────────────────────────╮",
    "│ ✓ scout S1                                 reported · 3m │",
    "│ question  where are Claude hook events parsed, and which │",
    "│           of them fire in -p mode?                       │",
    "│ state     reported                                       │",
    "│ took      3m                                             │",
    "│ report    1840 bytes                                     │",
    "│ files     crates/daemon/src/hooks.rs, crates/daemon/src… │",
    "│ session   #4 · window closed                             │",
    "│                                                          │",
    "│                                                          │",
    "╰──────────────────────────────────────────────────────────╯",
];

#[test]
fn run_panel_matches_the_mockup() {
    let inspection = node(gemini_fixture(), run_key("r1"));
    assert_eq!(panel(&inspection, 86, RUN_INSPECTOR_HEIGHT), RUN);
}

#[test]
fn planner_panel_matches_the_mockup() {
    let key = NodeKey::Planner {
        run: "r2".into(),
        epic: "A".into(),
    };
    let inspection = node(planner_fixture(), key);
    assert_eq!(panel(&inspection, 86, RUN_INSPECTOR_HEIGHT), PLANNER);
}

#[test]
fn task_panel_matches_the_mockup() {
    let inspection = node(gemini_fixture(), task_key("t2"));
    assert_eq!(panel(&inspection, 86, RUN_INSPECTOR_HEIGHT), TASK);
}

#[test]
fn task_panel_drops_the_right_text_and_elides_values() {
    let inspection = node(gemini_fixture(), task_key("t2"));
    assert_eq!(panel(&inspection, 60, RUN_INSPECTOR_HEIGHT), TASK_NARROW);
}

#[test]
fn worker_round_panel_matches_the_mockup() {
    let inspection = node(gemini_fixture(), round_key(AgentRole::Worker, 1, 2));
    assert_eq!(panel(&inspection, 86, RUN_INSPECTOR_HEIGHT), WORKER);
}

#[test]
fn reviewer_round_panel_matches() {
    let inspection = node(gemini_fixture(), round_key(AgentRole::Reviewer, 1, 1));
    assert_eq!(panel(&inspection, 86, RUN_INSPECTOR_HEIGHT), REVIEWER);
}

#[test]
fn scout_panel_wraps_the_question() {
    let inspection = node(gemini_fixture(), scout_key());
    assert_eq!(panel(&inspection, 60, RUN_INSPECTOR_HEIGHT), SCOUT);
}

/// Milestone 4.7's eight rows: the title and five fields, `attention` dropped from the
/// end.
#[test]
fn run_panel_in_eight_rows_drops_from_the_end() {
    let inspection = node(gemini_fixture(), run_key("r1"));
    let mut expected: Vec<&str> = RUN[..7].to_vec();
    expected.push(RUN[11]);
    assert_eq!(panel(&inspection, 86, 8), expected);
}

#[test]
fn the_wrapped_field_leaves_a_row_for_each_later_field() {
    let (mut snapshot, windows) = gemini_fixture();
    // Twenty nine-letter words: four to a 46-column line, so five lines are wanted and
    // four are left once the five later fields have a row each.
    snapshot.runs[0].scouts[0].question = format!("{}?", vec!["abcdefghi"; 20].join(" "));
    assert_eq!(snapshot.runs[0].scouts[0].question.len(), 200);
    let inspection = node((snapshot, windows), scout_key());
    let four = "abcdefghi abcdefghi abcdefghi abcdefghi";
    assert_eq!(
        panel(&inspection, 60, RUN_INSPECTOR_HEIGHT),
        [
            SCOUT[0].to_owned(),
            SCOUT[1].to_owned(),
            format!("│ question  {four:<46} │"),
            format!("│           {four:<46} │"),
            format!("│           {four:<46} │"),
            format!("│           {:<46} │", format!("{four}…")),
            SCOUT[4].to_owned(),
            SCOUT[5].to_owned(),
            SCOUT[6].to_owned(),
            SCOUT[7].to_owned(),
            SCOUT[8].to_owned(),
            SCOUT[11].to_owned(),
        ]
    );
}

/// With fewer rows than fields, the wrapped field still has its one row and the fields
/// after it are dropped from the end, as any others are.
#[test]
fn a_wrapped_field_in_too_few_rows_keeps_one_and_drops_the_end() {
    let inspection = rows(
        "scout S9",
        None,
        vec![
            plain("state", "working"),
            wrapping(
                "question",
                "one two three four five six seven eight nine ten",
            ),
            plain("took", "3m"),
            plain("report", "not yet"),
        ],
    );
    assert_eq!(
        panel(&inspection, 30, 5),
        [
            "╭────────────────────────────╮",
            "│ ● scout S9                 │",
            "│ state     working          │",
            "│ question  one two three…   │",
            "╰────────────────────────────╯",
        ]
    );
    // One row more gives the question's wrap nothing: `took` takes it.
    assert_eq!(
        panel(&inspection, 30, 6)[4],
        "│ took      3m               │"
    );
    // Three rows for it once the two later fields have theirs: the third line elided.
    assert_eq!(
        panel(&inspection, 30, 9)[3..8],
        [
            "│ question  one two three    │",
            "│           four five six    │",
            "│           seven eight nin… │",
            "│ took      3m               │",
            "│ report    not yet          │",
        ]
    );
    // And with room to spare the question wraps whole and nothing is dropped.
    assert_eq!(
        panel(&inspection, 30, 10),
        [
            "╭────────────────────────────╮",
            "│ ● scout S9                 │",
            "│ state     working          │",
            "│ question  one two three    │",
            "│           four five six    │",
            "│           seven eight nine │",
            "│           ten              │",
            "│ took      3m               │",
            "│ report    not yet          │",
            "╰────────────────────────────╯",
        ]
    );
}

/// The right text is shown exactly while the glyph, its space, the name, two spaces and
/// the right text fit; one column short and it goes, the name kept whole.
#[test]
fn the_right_text_shows_exactly_while_it_fits() {
    let inspection = rows("t1  spawn", Some("M · tdd"), vec![]);
    // "● t1  spawn" is 11, two spaces, "M · tdd" is 7: 20 columns of interior.
    assert_eq!(panel(&inspection, 24, 3)[1], "│ ● t1  spawn  M · tdd │");
    assert_eq!(panel(&inspection, 23, 3)[1], "│ ● t1  spawn         │");
    // Narrower than the name itself: the name is truncated.
    assert_eq!(panel(&inspection, 12, 3)[1], "│ ● t1  s… │");
}

#[test]
fn zero_fields_draw_the_title_alone() {
    let inspection = rows("r9  nothing", Some("running · 1s"), vec![]);
    assert_eq!(
        panel(&inspection, 31, 5),
        [
            "╭─────────────────────────────╮",
            "│ ● r9  nothing  running · 1s │",
            "│                             │",
            "│                             │",
            "╰─────────────────────────────╯",
        ]
    );
}

#[test]
fn a_label_longer_than_its_column_is_cut_and_the_values_stay_aligned() {
    let inspection = rows(
        "t1",
        None,
        vec![
            plain("a-much-too-long-label", "value"),
            // Exactly the column: it would touch its value, so it is cut too.
            plain("abcdefghij", "value"),
            plain("abcdefghi", "value"),
            plain("short", "value"),
        ],
    );
    assert_eq!(
        panel(&inspection, 24, 7),
        [
            "╭──────────────────────╮",
            "│ ● t1                 │",
            "│ a-much-t… value      │",
            "│ abcdefgh… value      │",
            "│ abcdefghi value      │",
            "│ short     value      │",
            "╰──────────────────────╯",
        ]
    );
}

#[test]
fn a_ten_thousand_character_value_is_elided_and_wraps_into_its_rows() {
    let long = "x".repeat(10_000);
    let words = "word ".repeat(2_000);
    let inspection = rows(
        &"n".repeat(10_000),
        Some(&"r".repeat(10_000)),
        vec![
            plain("files", &long),
            wrapping("question", &words),
            plain("took", "3m"),
        ],
    );
    let drawn = panel(&inspection, 30, 7);
    assert_eq!(
        drawn,
        [
            "╭────────────────────────────╮",
            &format!("│ ● {}… │", "n".repeat(23)),
            &format!("│ files     {}… │", "x".repeat(15)),
            "│ question  word word word   │",
            "│           word word word…  │",
            "│ took      3m               │",
            "╰────────────────────────────╯",
        ]
    );
    // A single unbroken 10k-character question is broken by width, not left to run.
    let inspection = rows("s", None, vec![wrapping("question", &long)]);
    assert_eq!(
        panel(&inspection, 30, 6)[2..5],
        [
            format!("│ question  {} │", "x".repeat(16)),
            format!("│           {} │", "x".repeat(16)),
            format!("│           {}… │", "x".repeat(15)),
        ]
    );
}

/// Wide characters at the elision boundary: a value, a name and a right text in CJK are
/// cut by display width, never mid-character, and never past the border.
#[test]
fn cjk_at_the_elision_boundary() {
    let inspection = rows(
        "t1  漢字漢字漢字漢字",
        Some("漢字"),
        vec![
            plain("route", &"漢".repeat(30)),
            // Sixteen columns of value room at 30 wide: `a`, seven wide characters and
            // the ellipsis are sixteen; an eighth would straddle the edge and is left out.
            plain("deps", &format!("a{}", "漢".repeat(9))),
            wrapping("question", &"字".repeat(30)),
        ],
    );
    let drawn = panel(&inspection, 30, 8);
    for row in &drawn {
        assert_eq!(UnicodeWidthStr::width(row.as_str()), 30, "{drawn:#?}");
    }
    assert_eq!(
        drawn[1..7],
        [
            // The name, two spaces and `漢字` need 28 columns of the 26: the right text
            // goes and the name stays whole.
            "│ ● t1  漢字漢字漢字漢字     │".to_owned(),
            format!("│ route     {}…  │", "漢".repeat(7)),
            format!("│ deps      a{}… │", "漢".repeat(7)),
            format!("│ question  {} │", "字".repeat(8)),
            format!("│           {} │", "字".repeat(8)),
            format!("│           {}…  │", "字".repeat(7)),
        ]
    );
}

/// Every size from nothing up, with and without a wrapping field: nothing panics, and
/// nothing is drawn past the panel's own rect.
#[test]
fn no_panic_at_degenerate_panel_sizes() {
    let inspection = rows(
        "t2  map Gemini hook events to status",
        Some("M · tdd · review round 2"),
        vec![
            plain("stages", "done ✓ → proof ✓"),
            wrapping("question", "where are Claude hook events parsed"),
            plain("a-much-too-long-label", "漢字漢字"),
        ],
    );
    for width in 0..=16 {
        for height in 0..=14 {
            let mut terminal = Terminal::new(TestBackend::new(width + 3, height + 3)).unwrap();
            terminal
                .draw(|frame| render(frame, &inspection, Rect::new(1, 1, width, height)))
                .unwrap();
            let buffer = terminal.backend().buffer();
            for y in 0..height + 3 {
                for x in 0..width + 3 {
                    let inside = (1..=width).contains(&x) && (1..=height).contains(&y);
                    assert!(
                        inside || buffer[(x, y)].symbol() == " ",
                        "{width}x{height}: drew outside the panel at ({x}, {y})"
                    );
                }
            }
        }
    }
}
