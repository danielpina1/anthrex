//! Forwarding one tool call to the daemon (the refreshed M8 brief's decision 41): a
//! fresh socket connection per call, `Hello { client: Mcp }`, one
//! `ClientMsg::Run(RunRequest::Tool(..))`, then the first `RunReply::ToolResult`.
//!
//! Nothing here returns an error to the MCP layer: every failure becomes `(false,
//! <text>)`, which the server turns into an `isError: true` result the agent can read.

use std::time::Duration;

use proto::{
    ClientKind, ClientMsg, DaemonMsg, PROTO_VERSION, RunReply, RunRequest, ToolCall, read_frame,
    write_frame,
};
use tokio::net::UnixStream;
use tokio::net::unix::OwnedReadHalf;

use crate::{McpOptions, TOOL_REPLY_TIMEOUT};

/// How long connecting to the daemon's socket may take.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);

/// Sends `tool` with `args` to the daemon on behalf of `opts`' agent and returns the
/// engine's `(ok, text)`.
pub async fn forward(opts: &McpOptions, tool: &str, args: serde_json::Value) -> (bool, String) {
    let unreachable = |e: &dyn std::fmt::Display| {
        format!(
            "cannot reach the anthrex daemon at {}: {e}",
            opts.socket.display()
        )
    };

    let stream =
        match tokio::time::timeout(CONNECT_TIMEOUT, UnixStream::connect(&opts.socket)).await {
            Ok(Ok(stream)) => stream,
            Ok(Err(e)) => return (false, unreachable(&e)),
            Err(_) => {
                return (
                    false,
                    unreachable(&format!(
                        "connect timed out after {} s",
                        CONNECT_TIMEOUT.as_secs()
                    )),
                );
            }
        };
    let (mut rd, mut wr) = stream.into_split();

    let hello = ClientMsg::Hello {
        proto_version: PROTO_VERSION,
        client: ClientKind::Mcp,
    };
    if let Err(e) = write_frame(&mut wr, &hello).await {
        return (false, unreachable(&e));
    }
    if let Err(e) = handshake(&mut rd).await {
        return (false, unreachable(&e));
    }

    let call = ToolCall {
        run_id: opts.run_id.clone(),
        task_id: opts.task_id.clone(),
        role: opts.role,
        window_id: opts.window_id,
        tool: tool.to_string(),
        args,
    };
    if let Err(e) = write_frame(&mut wr, &ClientMsg::Run(RunRequest::Tool(call))).await {
        return (false, unreachable(&e));
    }

    match tokio::time::timeout(TOOL_REPLY_TIMEOUT, reply(&mut rd, tool)).await {
        Ok(result) => result,
        Err(_) => (
            false,
            format!(
                "the anthrex daemon did not answer {tool} within {} s",
                TOOL_REPLY_TIMEOUT.as_secs()
            ),
        ),
    }
}

/// Waits for `Welcome`, bounded by `proto::HANDSHAKE_TIMEOUT`.
async fn handshake(rd: &mut OwnedReadHalf) -> Result<(), String> {
    match tokio::time::timeout(proto::HANDSHAKE_TIMEOUT, read_frame::<_, DaemonMsg>(rd)).await {
        Err(_) => Err(format!(
            "no handshake within {} s",
            proto::HANDSHAKE_TIMEOUT.as_secs()
        )),
        Ok(Err(e)) => Err(e.to_string()),
        Ok(Ok(None)) => Err("the daemon closed the connection during the handshake".into()),
        Ok(Ok(Some(DaemonMsg::Welcome { .. }))) => Ok(()),
        Ok(Ok(Some(DaemonMsg::Error { message, .. }))) => Err(message),
        Ok(Ok(Some(other))) => Err(format!("unexpected handshake reply: {other:?}")),
    }
}

/// Reads until the `ToolResult`, skipping broadcasts and every other reply. A
/// `DaemonMsg::Error` is the daemon refusing the request outright (for example one that
/// predates the run engine), so it ends the wait instead of sitting out the timeout.
async fn reply(rd: &mut OwnedReadHalf, tool: &str) -> (bool, String) {
    loop {
        match read_frame::<_, DaemonMsg>(rd).await {
            Ok(Some(DaemonMsg::Run(RunReply::ToolResult { ok, text }))) => return (ok, text),
            Ok(Some(DaemonMsg::Error { message, .. })) => return (false, message),
            Ok(Some(_)) => continue,
            Ok(None) => {
                return (
                    false,
                    format!("the anthrex daemon closed the connection before answering {tool}"),
                );
            }
            Err(e) => {
                return (
                    false,
                    format!("the anthrex daemon's reply to {tool} could not be read: {e}"),
                );
            }
        }
    }
}
