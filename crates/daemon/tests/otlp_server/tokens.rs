//! Milestone 9 decisions 14a and 14b over real TCP: only points carrying their run's
//! token are metered, the connection cap grows with live orchestrators, and a full
//! receiver closes a connection that never presented a token first.

use super::*;
use daemon::metering::server::OTLP_EVICT_GRACE;

/// Milestone 9 decision 14a: only points carrying their run's token are metered. A
/// missing or wrong token is still answered `200`, and nothing reaches the sink.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn points_without_the_runs_token_are_dropped() {
    let mut rx = start().await;
    let addr = rx.server.addr;
    for token in [None, Some("ffffffffffffffffffffffffffffffff"), Some("")] {
        let (_, answer) = send(
            addr,
            &request_with("/v1/metrics", "application/json", FIXTURE, token),
        )
        .await;
        assert_eq!(answer, Some((200, "{}".to_string())), "{token:?}");
    }
    let (_, answer) = send(addr, &request("/v1/metrics", "application/json", FIXTURE)).await;
    assert_eq!(answer, Some((200, "{}".to_string())));
    assert_eq!(
        next_total(&mut rx.sink).await,
        ("r-fix".into(), fixture_usage()),
        "only the tokened export was metered"
    );
    assert!(rx.sink.try_recv().is_err());
    rx.shutdown.cancel();
}

/// Milestone 9 decision 14b: the cap is `OTLP_BASE_CONNECTIONS` plus the live
/// orchestrators, read again when the sink's live generation changes.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn connection_cap_grows_with_live_orchestrators() {
    let rx = start().await;
    let addr = rx.server.addr;
    let mut held = hold_every_slot(addr).await;
    let (_, answer) = send(addr, &request("/v1/metrics", "application/json", b"{}")).await;
    assert_eq!(answer, None, "the base cap is full");
    rx.control.set_orchestrators(2);
    held.extend(hold_slots(addr, 2).await);
    let (_, answer) = send(addr, &request("/v1/metrics", "application/json", b"{}")).await;
    assert_eq!(answer, None, "the grown cap is full too");
    assert_eq!(held.len(), OTLP_BASE_CONNECTIONS + 2);
    rx.shutdown.cancel();
}

/// Milestone 9 decision 14b: with every slot taken by connections that never presented
/// a token, a tokened client still gets in: one untokened connection is closed first.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn untokened_connections_are_closed_first_when_full() {
    let mut rx = start().await;
    let addr = rx.server.addr;
    let mut idle = Vec::new();
    for _ in 0..OTLP_BASE_CONNECTIONS {
        idle.push(TcpStream::connect(addr).await.unwrap());
    }
    // Every idle connection holds a slot, and is past the eviction grace, before the
    // tokened one arrives.
    tokio::time::sleep(OTLP_EVICT_GRACE + OTLP_SLOT_WAIT / 10).await;
    let started = Instant::now();
    let (_, answer) = send(addr, &request("/v1/metrics", "application/json", FIXTURE)).await;
    assert_eq!(answer, Some((200, "{}".to_string())));
    assert!(
        started.elapsed() < OTLP_READ_TIMEOUT,
        "{:?}",
        started.elapsed()
    );
    assert_eq!(next_total(&mut rx.sink).await.1, fixture_usage());
    // Exactly one idle connection was closed to make room.
    let mut closed = 0;
    for stream in &mut idle {
        let mut byte = [0u8; 1];
        let read = tokio::time::timeout(Duration::from_millis(50), stream.read(&mut byte)).await;
        if matches!(read, Ok(Ok(0)) | Ok(Err(_))) {
            closed += 1;
        }
    }
    assert_eq!(closed, 1);
    rx.shutdown.cancel();
}

/// M9.10 review: a connection younger than `OTLP_EVICT_GRACE` is never closed to make
/// room, so a real orchestrator's new connection cannot be closed before its first POST
/// marks it tokened. With every slot held by fresh idle connections, a newcomer waits
/// the slot wait and is closed, and every idle connection stays open.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_young_untokened_connection_is_not_closed_to_make_room() {
    const { assert!(OTLP_SLOT_WAIT.as_millis() < OTLP_EVICT_GRACE.as_millis()) };
    let rx = start().await;
    let addr = rx.server.addr;
    let mut idle = Vec::new();
    for _ in 0..OTLP_BASE_CONNECTIONS {
        idle.push(TcpStream::connect(addr).await.unwrap());
    }
    let (_, answer) = send(addr, &request("/v1/metrics", "application/json", FIXTURE)).await;
    assert_eq!(
        answer, None,
        "no slot, and none of the young ones was closed"
    );
    for stream in &mut idle {
        let mut byte = [0u8; 1];
        let read = tokio::time::timeout(Duration::from_millis(50), stream.read(&mut byte)).await;
        assert!(read.is_err(), "an idle connection was closed: {read:?}");
    }
    rx.shutdown.cancel();
}
