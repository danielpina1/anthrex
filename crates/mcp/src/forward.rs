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
    match handshake(&mut rd).await {
        Ok(()) => {}
        Err(Handshake::Refused(message)) => {
            // Review I1: the daemon's text is written for a human at a terminal (the
            // mismatch reply says to run `anthrex daemon stop`), and an agent reading
            // it may act on it. It goes to stderr; the agent gets a fixed text.
            eprintln!("anthrex mcp: the daemon refused the handshake: {message}");
            return (false, VERSION_MISMATCH.to_string());
        }
        Err(Handshake::Failed(e)) => return (false, unreachable(&e)),
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

/// What the agent reads when the daemon refuses the handshake. The daemon only refuses
/// a `Hello` over a protocol version mismatch, and its own text names a command.
pub const VERSION_MISMATCH: &str =
    "the anthrex daemon speaks a different protocol version; the user must restart it";

enum Handshake {
    /// The daemon answered `Hello` with a `DaemonMsg::Error`; its message.
    Refused(String),
    /// Anything else that kept the handshake from completing.
    Failed(String),
}

/// Waits for `Welcome`, bounded by `proto::HANDSHAKE_TIMEOUT`.
async fn handshake(rd: &mut OwnedReadHalf) -> Result<(), Handshake> {
    match tokio::time::timeout(proto::HANDSHAKE_TIMEOUT, read_frame::<_, DaemonMsg>(rd)).await {
        Err(_) => Err(Handshake::Failed(format!(
            "no handshake within {} s",
            proto::HANDSHAKE_TIMEOUT.as_secs()
        ))),
        Ok(Err(e)) => Err(Handshake::Failed(e.to_string())),
        Ok(Ok(None)) => Err(Handshake::Failed(
            "the daemon closed the connection during the handshake".into(),
        )),
        Ok(Ok(Some(DaemonMsg::Welcome { .. }))) => Ok(()),
        Ok(Ok(Some(DaemonMsg::Error { message, .. }))) => Err(Handshake::Refused(message)),
        Ok(Ok(Some(_))) => Err(Handshake::Failed("unexpected handshake reply".into())),
    }
}

/// Reads until the `ToolResult`, skipping broadcasts and every other reply. A
/// `DaemonMsg::Error` labelled `proto::run_wire::request::TOOL` is the daemon refusing
/// this call outright, so it ends the wait instead of sitting out the timeout; an
/// `Error` with any other label is not about this call and is skipped (review M1).
async fn reply(rd: &mut OwnedReadHalf, tool: &str) -> (bool, String) {
    loop {
        match read_frame::<_, DaemonMsg>(rd).await {
            Ok(Some(DaemonMsg::Run(RunReply::ToolResult { ok, text }))) => return (ok, text),
            Ok(Some(DaemonMsg::Error { request, message }))
                if request == proto::run_wire::request::TOOL =>
            {
                return (false, message);
            }
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
