//! Milestone 9.0.7 task 9: the run view's sized inspector (decision 17), its stages
//! (decision 18), deps (decision 19), selection (decision 20) and title (decision 21).

use crate::app::App;
use crate::inspector::{INSPECTOR_HEIGHT, RUN_CANVAS_MIN};
use crate::settings::UiSettings;
use crate::theme::{Role, role};
use crate::tree::run_fixtures::{PROJECT, RUN_ID, pty, run, snapshot, task, three_task_fixture};
use crate::tree::stage_fixtures::staged_fixture;
use crate::tree::{self, NodeKey};
use crate::ui::audit;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use proto::{DaemonMsg, FullState, RunReply, RunState, RunsSnapshot, Size, TaskState, WindowInfo};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Modifier;

pub(super) fn app_of((snap, windows): (RunsSnapshot, Vec<WindowInfo>), ascii: bool) -> App {
    let mut settings = UiSettings::default();
    settings.badges.ascii = ascii;
    let mut app = App::new(windows, "/tmp".into(), settings);
    let _ = app.set_terminal_size(120, 40);
    let _ = app.run_subscription();
    app.on_daemon(DaemonMsg::Run(RunReply::Snapshot(snap)));
    app
}

pub(super) fn key(app: &mut App, code: KeyCode) {
    app.on_key(KeyEvent::new(code, KeyModifiers::NONE));
}

/// `C-b T`, the run's node selected, then `l`: the run view on `run_id`.
pub(super) fn run_view(mut app: App, run_id: &str) -> App {
    app.on_key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL));
    key(&mut app, KeyCode::Char('T'));
    let rows = tree::build_with_runs(&app.windows, &app.runs.runs, &app.tree);
    app.tree.select(&rows, NodeKey::Run(run_id.into()));
    key(&mut app, KeyCode::Char('l'));
    assert!(app.run_view.is_some(), "the run view opened");
    app
}

pub(super) fn select(app: &mut App, key: NodeKey) {
    let rows = app.nav_rows();
    let keys: Vec<NodeKey> = rows.iter().map(|row| row.key.clone()).collect();
    assert!(keys.contains(&key), "{key:?} is on the canvas");
    drop(rows);
    app.tree.selected = Some(key);
}

pub(super) fn task_key(run: &str, id: &str) -> NodeKey {
    NodeKey::Task {
        run: run.into(),
        id: id.into(),
    }
}

/// One frame as `lib::draw` lays it out: both viewports set from the frame's layout,
/// then drawn whole.
pub(super) fn frame(app: &mut App, w: u16, h: u16) -> (Buffer, crate::ui::Layout) {
    let layout = crate::ui::layout_for(app, Rect::new(0, 0, w, h));
    app.set_tree_viewports(layout.sidebar_list.height, layout.main_inner.height);
    app.set_graph_viewport(layout.main);
    (audit::draw(app, w, h), layout)
}

/// The rows of `rect` in `buffer`, trailing spaces trimmed.
pub(super) fn rows_in(buffer: &Buffer, rect: Rect) -> Vec<String> {
    (rect.y..rect.bottom())
        .map(|y| {
            let row: String = (rect.x..rect.right())
                .map(|x| buffer[(x, y)].symbol())
                .collect();
            row.trim_end().to_owned()
        })
        .collect()
}

/// The rect below the canvas, as the frame drew it.
fn footer(app: &App, layout: &crate::ui::Layout) -> Rect {
    crate::ui::overview::view(app, layout.main).footer
}

#[test]
fn run_panel_heights_follow_their_content() {
    let main = |interior: u16| Rect::new(0, 0, 120, interior + 2);
    let footer = |interior, rows| {
        crate::ui::overview::areas(main(interior), true, Some(rows))
            .1
            .height
    };
    assert_eq!(footer(37, 5), INSPECTOR_HEIGHT); // never below milestone 4.7's eight
    assert_eq!(footer(37, 14), 16); // the rows it needs, plus its borders
    assert_eq!(footer(37, 60), 37 - RUN_CANVAS_MIN); // cut, the canvas keeps six rows
    assert_eq!(footer(13, 9), 1); // too short for a panel: the single line
    assert_eq!(
        crate::ui::overview::areas(main(37), true, None).1.height,
        INSPECTOR_HEIGHT,
        "the project overview"
    );
    assert_eq!(
        crate::ui::overview::areas(main(37), false, Some(14))
            .1
            .height,
        1,
        "`i` hides it"
    );
}

/// The Gemini run's `t2` with a forty-line brief, expanded: a body longer than any
/// panel at 120x25.
fn long_task() -> App {
    use crate::app::task_detail::{DetailState, TaskDetailCache, detail_key};
    let mut app = run_view(
        app_of(crate::tree::run_fixtures::gemini_fixture(), false),
        "r1",
    );
    select(&mut app, task_key("r1", "t2"));
    let task = app.runs.runs[0]
        .tasks
        .iter()
        .find(|t| t.id == "t2")
        .unwrap();
    let brief: Vec<String> = (1..=40).map(|n| format!("brief line {n:02}")).collect();
    app.task_detail = Some(TaskDetailCache {
        run_id: "r1".into(),
        task_id: "t2".into(),
        key: detail_key(task),
        state: DetailState::Ready(Box::new(proto::TaskDetailInfo {
            run_id: "r1".into(),
            task_id: "t2".into(),
            brief: brief.join("\n"),
            acceptance: vec!["every event maps".into()],
            worker_summary: None,
            summary_source: None,
        })),
    });
    key(&mut app, KeyCode::Char('b'));
    app
}

/// The panel's interior rows between its title row and its footer row.
fn body_of(panel: &[String]) -> Vec<String> {
    panel[2..panel.len() - 2].to_vec()
}

/// Review focus 3: PgDn scrolls by exactly the body rows the sized panel draws (its
/// interior less the title and the footer rows) and stops at the last body row.
#[test]
fn page_down_follows_the_sized_panel() {
    // The whole body, from a terminal tall enough to draw all of it.
    let mut tall = long_task();
    let (buffer, layout) = frame(&mut tall, 120, 120);
    let all = rows_in(&buffer, footer(&tall, &layout));
    let whole = body_of(&all);
    assert!(whole.len() >= 40, "a forty-row body: {all:#?}");
    assert!(all[all.len() - 1].starts_with('╰') && !all[all.len() - 1].contains("PgDn"));

    let mut app = long_task();
    let (buffer, layout) = frame(&mut app, 120, 25);
    let rect = footer(&app, &layout);
    assert_eq!(layout.main_inner.height, 22);
    assert_eq!(rect.height, 22 - RUN_CANVAS_MIN, "cut: a 16-row panel");
    let interior = rect.height - 2;
    assert_eq!(app.task_panel_interior(), (rect.width - 4, interior));
    let room = usize::from(interior) - 2;
    let drawn = body_of(&rows_in(&buffer, rect));
    assert_eq!(drawn, whole[..room], "the first page");

    let mut first = 0;
    loop {
        key(&mut app, KeyCode::PageDown);
        let (buffer, layout) = frame(&mut app, 120, 25);
        let drawn = body_of(&rows_in(&buffer, footer(&app, &layout)));
        let expected = (first + room).min(whole.len() - room);
        assert_eq!(
            drawn,
            whole[expected..expected + room],
            "from row {expected}"
        );
        if expected == first {
            break;
        }
        first = expected;
    }
    assert_eq!(first, whole.len() - room, "it stops at the last body row");
    let (buffer, layout) = frame(&mut app, 120, 25);
    let panel = rows_in(&buffer, footer(&app, &layout));
    assert!(!panel[panel.len() - 1].contains("PgDn"), "{panel:#?}");
    assert!(panel[0].contains("PgUp"), "{panel:#?}");
}

/// A running run `add-mul-0723`, "Add mul()", two of three tasks merged, approved 14
/// minutes ago; the shell window `1` in its project.
pub(super) fn add_mul(state: RunState) -> (RunsSnapshot, Vec<WindowInfo>) {
    let now = 10_000;
    let mut info = run("add-mul-0723", PROJECT, state);
    info.goal = "Add mul()".into();
    info.created_at = now - 2 * 60;
    if state != RunState::Planning {
        info.created_at = now - 3600;
        info.approved_at = Some(now - 14 * 60);
        info.tasks = vec![
            task("t1", "mul", Size::S, TaskState::Merged),
            task("t2", "docs", Size::S, TaskState::Merged),
            task("t3", "tests", Size::S, TaskState::Working),
        ];
    }
    (
        snapshot(now, vec![info]),
        vec![pty(1, "shell", PROJECT, proto::Status::Idle)],
    )
}

/// The run view's top border row.
fn title_row(app: &mut App) -> String {
    let (buffer, layout) = frame(app, 120, 40);
    rows_in(&buffer, layout.main)[0].clone()
}

#[test]
fn the_run_view_title_names_the_run() {
    let mut app = run_view(app_of(add_mul(RunState::Running), false), "add-mul-0723");
    let top = title_row(&mut app);
    assert!(top.contains(" run · Add mul() · 0723 "), "{top}");
    assert!(
        top.trim_end_matches(['╮', '─'])
            .ends_with(" 2/3 merged · 14m "),
        "{top}"
    );
    assert!(!top.contains("add-mul-0723"), "{top}");

    let mut app = run_view(app_of(add_mul(RunState::Planning), false), "add-mul-0723");
    let top = title_row(&mut app);
    assert!(top.contains(" run · Add mul() · 0723 "), "{top}");
    assert!(top.contains(" planning · 2m "), "{top}");
    assert!(!top.contains("merged"), "{top}");

    // ASCII: the separators fold.
    let mut app = run_view(app_of(add_mul(RunState::Running), true), "add-mul-0723");
    let top = title_row(&mut app);
    assert!(top.contains(" run - Add mul() - 0723 "), "{top}");
    assert!(top.contains(" 2/3 merged - 14m "), "{top}");
    assert!(top.is_ascii(), "{top}");
}

/// The staged run with stage 2 created (a head) and in `state`, `secs` long; the shell
/// window `1` lists its project.
pub(super) fn staged(change: impl FnOnce(&mut proto::StageInfo)) -> App {
    let (mut snap, _) = staged_fixture();
    let two = &mut snap.runs[0].stages[1];
    two.head = Some("3".repeat(40));
    change(two);
    let windows = vec![pty(1, "shell", PROJECT, proto::Status::Idle)];
    run_view(app_of((snap, windows), false), RUN_ID)
}

fn stage_text(app: &App, n: u16) -> String {
    let rows = app.nav_rows();
    let row = rows
        .iter()
        .find(|row| {
            row.key
                == NodeKey::Stage {
                    run: RUN_ID.into(),
                    n,
                }
        })
        .expect("the stage row");
    crate::graph::content_text(row)
}

#[test]
fn a_bisecting_stage_reads_failed() {
    let mut app = staged(|two| two.full.state = FullState::Bisecting);
    assert_eq!(stage_text(&app, 2), "stage 2/2  tier 3 ✗ bisecting");
    let failed = role(Role::Failed, app.palette()).fg;
    for tick in 0..10 {
        app.spinner_frame = tick;
        let (buffer, _) = frame(&mut app, 120, 40);
        let at = audit::find(&buffer, "stage 2/2  tier 3 ✗");
        let &(x, y) = at.first().expect("stage 2's box");
        let glyph = &buffer[(x - 2, y)];
        assert_eq!(glyph.symbol(), "✗", "tick {tick}");
        assert_eq!(glyph.fg, failed.unwrap(), "tick {tick}");
    }
}

#[test]
fn green_and_red_stages_show_their_time() {
    let mut app = staged(|two| {
        two.full.state = FullState::Green;
        two.full.secs = Some(38);
    });
    assert_eq!(stage_text(&app, 2), "stage 2/2  tier 3 ✓ 38s");
    let (buffer, _) = frame(&mut app, 120, 40);
    assert!(!audit::find(&buffer, "stage 2/2  tier 3 ✓ 38s").is_empty());

    let app = staged(|two| {
        two.full.state = FullState::Red;
        two.full.secs = Some(64);
    });
    assert_eq!(stage_text(&app, 2), "stage 2/2  tier 3 ✗ 1m04s");
    let app = staged(|two| two.full.state = FullState::Running);
    assert_eq!(stage_text(&app, 2), "stage 2/2  tier 3 running");
    let app = staged(|two| two.full.state = FullState::None);
    assert_eq!(stage_text(&app, 2), "stage 2/2  tier 3 ◌");
    let app = staged(|two| {
        two.head = None;
        two.full.state = FullState::Green;
    });
    assert_eq!(
        stage_text(&app, 2),
        "stage 2/2  tier 3 ◌",
        "no head: not run"
    );
    let app = staged(|two| two.full.state = FullState::Green);
    assert_eq!(stage_text(&app, 2), "stage 2/2  tier 3 ✓", "no time sent");
}

/// Stage 1's inspection with `failing` as its names, drawn into a `width`-column panel
/// as tall as its content.
fn stage_panel(failing: Vec<String>, width: u16) -> (crate::inspector::Inspection, Vec<String>) {
    let mut app = staged(|_| {});
    app.runs.runs[0].stages[0].full.failing = failing;
    let rows = app.nav_rows();
    let row = rows
        .iter()
        .find(|row| {
            row.key
                == NodeKey::Stage {
                    run: RUN_ID.into(),
                    n: 1,
                }
        })
        .unwrap();
    let inspection = crate::inspector::inspect(row, &app);
    let height = crate::inspector::panel_rows(&inspection, width - 4) + 2;
    let mut terminal =
        ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|f| crate::inspector::render(f, &inspection, f.area()))
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    let rows = rows_in(&buffer, buffer.area);
    (inspection, rows)
}

#[test]
fn stage_failing_names_wrap() {
    let names: Vec<String> = (1..=20).map(|n| format!("suite::case_{n:02}")).collect();
    let (inspection, rows) = stage_panel(names.clone(), 60);
    let tier3 = inspection
        .fields
        .iter()
        .find(|f| f.label == "tier 3")
        .unwrap();
    assert!(!tier3.value.contains("failing"), "{}", tier3.value);
    let failing = inspection
        .fields
        .iter()
        .find(|f| f.label == "failing")
        .unwrap();
    assert!(failing.wrap);
    assert_eq!(failing.value, names.join(", "));
    let text = rows.join("\n");
    for name in &names {
        assert!(text.contains(name.as_str()), "{name}:\n{text}");
    }
    let named: Vec<&String> = rows.iter().filter(|r| r.contains("suite::")).collect();
    assert!(
        named.iter().all(|r| !r.contains('…')),
        "none is cut:\n{text}"
    );
    let rows_with_names = named.len();
    assert!(rows_with_names > 1, "the names wrap:\n{text}");
    assert!(
        rows.iter()
            .any(|r| r.starts_with("│ failing   suite::case_01, ")),
        "{text}"
    );
}

#[test]
fn stage_failing_names_are_sanitised() {
    use crate::safe_text::tests::{first_hostile, hostile_text};
    let (inspection, rows) = stage_panel(vec![hostile_text(), "b::x".into()], 60);
    for field in &inspection.fields {
        assert_eq!(first_hostile(&field.value), None, "{}", field.label);
    }
    for row in &rows {
        assert_eq!(first_hostile(row), None, "{row}");
    }
    assert!(rows.iter().any(|r| r.contains("b::x")), "{rows:#?}");
}

/// `t1` merged, `t2` in review, `t3 docs` S after both.
fn docs_run() -> App {
    let mut info = run("docs-run-0001", PROJECT, RunState::Running);
    let mut t3 = task("t3", "docs", Size::S, TaskState::Pending);
    t3.deps = vec!["t1".into(), "t2".into()];
    info.tasks = vec![
        task("t1", "core", Size::S, TaskState::Merged),
        task("t2", "api", Size::M, TaskState::Review),
        t3,
    ];
    let windows = vec![pty(1, "shell", PROJECT, proto::Status::Idle)];
    run_view(
        app_of((snapshot(10_000, vec![info]), windows), false),
        "docs-run-0001",
    )
}

#[test]
fn deps_read_after() {
    let mut app = docs_run();
    let key = task_key("docs-run-0001", "t3");
    let rows = app.nav_rows();
    let row = rows.iter().find(|row| row.key == key).unwrap();
    assert_eq!(crate::graph::content_text(row), "t3 docs S  after t1, t2");
    let inspection = crate::inspector::inspect(row, &app);
    let detail = inspection
        .sections
        .iter()
        .find(|s| s.title == "DETAIL")
        .unwrap();
    let deps = detail.fields.iter().find(|f| f.label == "deps").unwrap();
    assert!(deps.value.starts_with("after t1 ✓, t2 ◐"), "{}", deps.value);
    drop(rows);
    let (buffer, _) = frame(&mut app, 120, 40);
    assert!(!audit::find(&buffer, "t3 docs S  after t1, t2").is_empty());
}

#[test]
fn selection_has_the_bar_and_related_nodes_are_bold() {
    for ascii in [false, true] {
        let mut app = run_view(app_of(three_task_fixture(), ascii), RUN_ID);
        select(&mut app, task_key(RUN_ID, "t1"));
        let (buffer, layout) = frame(&mut app, 120, 40);
        let p = app.palette();
        let (accent, muted) = (role(Role::Accent, p).fg, role(Role::Muted, p).fg);

        // The selected box's content row starts with the bar, in the accent.
        let &(x, y) = audit::find(&buffer, "t1 spawn").first().expect("t1's box");
        let bar = &buffer[(x - 4, y)];
        assert_eq!(bar.symbol(), if ascii { ">" } else { "▌" });
        assert_eq!(Some(bar.fg), accent);
        assert!(buffer[(x, y)].modifier.contains(Modifier::REVERSED));

        // Its dependency `t0` is lit: a muted border, bold.
        let &(x, y) = audit::find(&buffer, "t0 proto").first().expect("t0's box");
        let corner = &buffer[(x - 4, y - 1)];
        assert_eq!(corner.symbol(), if ascii { "+" } else { "╭" });
        assert_eq!(Some(corner.fg), muted);
        assert!(corner.modifier.contains(Modifier::BOLD));

        // No box wears the accent: only the run view's own frame does.
        assert_eq!(audit::accented_frames(&buffer, p), 1);
        let canvas = crate::ui::overview::view(&app, layout.main).canvas;
        for cy in canvas.y..canvas.bottom() {
            for cx in canvas.x..canvas.right() {
                let cell = &buffer[(cx, cy)];
                if "╭╮╰╯─│├┤+-|".contains(cell.symbol()) && !cell.symbol().is_empty()
                {
                    assert_ne!(Some(cell.fg), accent, "({cx}, {cy}) {}", cell.symbol());
                }
            }
        }
        if ascii {
            assert_eq!(audit::first_non_ascii(&buffer), None);
        }
    }
}

/// Decision 17 through the frame: the panel is the selected task's own rows plus its
/// borders, between milestone 4.7's eight and the interior less six canvas rows.
#[test]
fn the_frame_sizes_the_panel_to_the_selected_task() {
    let mut app = docs_run();
    let key = task_key("docs-run-0001", "t3");
    select(&mut app, key.clone());
    let (_, layout) = frame(&mut app, 120, 40);
    let rows = app.nav_rows();
    let row = rows.iter().find(|row| row.key == key).unwrap();
    let inspection = crate::inspector::inspect(row, &app);
    let rect = footer(&app, &layout);
    let need = crate::inspector::panel_rows(&inspection, rect.width - 4);
    drop(rows);
    assert_eq!(
        rect.height,
        (need + 2).clamp(INSPECTOR_HEIGHT, layout.main_inner.height - RUN_CANVAS_MIN)
    );
}
