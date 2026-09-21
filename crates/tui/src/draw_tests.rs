use super::*;
use proto::{ClientMsg, DaemonMsg, Runtime, Status, WindowInfo, read_frame, write_frame};
use ratatui::{Terminal, backend::TestBackend};
use tokio::net::{UnixListener, UnixStream};

async fn connection(windows: &[WindowInfo]) -> (tempfile::TempDir, Connection, UnixStream) {
    let dir = tempfile::tempdir_in("/tmp").unwrap();
    let socket = dir.path().join("draw.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    let accept = async {
        let (mut stream, _) = listener.accept().await.unwrap();
        assert!(matches!(
            read_frame::<_, ClientMsg>(&mut stream).await.unwrap(),
            Some(ClientMsg::Hello { .. })
        ));
        write_frame(
            &mut stream,
            &DaemonMsg::Welcome {
                daemon_version: "test".into(),
                windows: windows.to_vec(),
            },
        )
        .await
        .unwrap();
        stream
    };
    let (conn, peer) = tokio::time::timeout(Duration::from_secs(5), async {
        tokio::join!(Connection::connect(&socket), accept)
    })
    .await
    .unwrap();
    (dir, conn.unwrap(), peer)
}

fn shells() -> Vec<WindowInfo> {
    let template = tree::example_windows().remove(0);
    (1..=20)
        .map(|id| WindowInfo {
            id,
            name: format!("shell-{id}"),
            runtime: Runtime::Shell,
            status: Status::Idle,
            model: None,
            subagents: vec![],
            ..template.clone()
        })
        .collect()
}

#[tokio::test]
async fn production_draw_keeps_resize_render_and_click_on_the_same_row() {
    let windows = shells();
    let (_dir, conn, mut peer) = connection(&windows).await;
    let mut app = App::new(windows, "/tmp".into(), UiSettings::default());
    let mut terminal = Terminal::new(TestBackend::new(120, 14)).unwrap();
    draw(&mut terminal, &mut app, &conn).unwrap();
    assert_eq!(
        tokio::time::timeout(
            Duration::from_secs(5),
            read_frame::<_, ClientMsg>(&mut peer)
        )
        .await
        .unwrap()
        .unwrap(),
        Some(ClientMsg::Subscribe {
            window_id: 1,
            cols: 84,
            rows: 11
        })
    );
    app.focus(20);
    draw(&mut terminal, &mut app, &conn).unwrap();
    assert_eq!(app.tree.sidebar.top, 12);

    terminal.backend_mut().resize(120, 10);
    let layout = draw(&mut terminal, &mut app, &conn).unwrap();
    assert_eq!(layout.sidebar_list.height, 5);
    assert_eq!(app.tree.sidebar.top, 16);
    let list = layout.sidebar_list;
    let first_row: String = (list.x..list.right())
        .map(|x| terminal.backend().buffer()[(x, list.y)].symbol())
        .collect();
    let displayed_id: u32 = first_row
        .split("shell-")
        .nth(1)
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(
        app.on_click(list.x, list.y, &layout),
        vec![Effect::Send(ClientMsg::Subscribe {
            window_id: displayed_id,
            cols: 84,
            rows: 7
        })],
        "the first visible row {first_row:?} must be the row clicked after the resize"
    );

    // Repeated draws at this size must not undo wheel scrolling.
    app.on_scroll(true, list.x, list.y, &layout);
    let top = app.tree.sidebar.top;
    draw(&mut terminal, &mut app, &conn).unwrap();
    assert_eq!(app.tree.sidebar.top, top);
    app.modal = Some(app::Modal::Help);
    let layout = draw(&mut terminal, &mut app, &conn).unwrap();
    assert!(
        app.on_click(layout.sidebar_list.x, layout.sidebar_list.y, &layout)
            .is_empty()
    );
    assert_eq!(app.tree.sidebar.top, top);
}
