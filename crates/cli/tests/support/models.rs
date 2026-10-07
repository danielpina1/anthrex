//! Milestone 9.8's discovery over the protocol itself (decision 25: no CLI subcommand).

use std::path::Path;
use std::time::Duration;

use proto::{ClientMsg, DaemonMsg, ModelCatalog, Runtime};

use super::run_harness::connect;
use super::runtime;

/// One `ListModels`: each runtime's refresh is bounded by the daemon's
/// `DISCOVERY_TIMEOUT` (the two run at once), plus 10 s for the connection and the
/// blocking pool under load (`docs/timing-budgets.md`, M9.8.6).
pub const MODELS_WAIT: Duration =
    Duration::from_secs(daemon::models::DISCOVERY_TIMEOUT.as_secs() + 10);

/// Sends `ListModels { runtime, refresh }` on a fresh connection and returns the
/// catalogs of its `Models` reply.
pub fn list_models(
    socket: &Path,
    runtime_asked: Option<Runtime>,
    refresh: bool,
) -> Vec<ModelCatalog> {
    let socket = socket.to_path_buf();
    runtime().block_on(async move {
        let mut stream = connect(&socket).await;
        proto::write_frame(
            &mut stream,
            &ClientMsg::ListModels {
                runtime: runtime_asked,
                refresh,
            },
        )
        .await
        .unwrap();
        tokio::time::timeout(MODELS_WAIT, async {
            loop {
                match proto::read_frame::<_, DaemonMsg>(&mut stream).await {
                    Ok(Some(DaemonMsg::Models { catalogs })) => return catalogs,
                    Ok(Some(DaemonMsg::Error { message, .. })) => panic!("error: {message}"),
                    Ok(Some(_)) => {}
                    other => panic!("connection ended: {other:?}"),
                }
            }
        })
        .await
        .expect("ListModels was not answered")
    })
}
