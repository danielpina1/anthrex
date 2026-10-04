//! The anthrex MCP server (decisions 4 and 5): headless worker and reviewer agents
//! report to the run engine through it, and milestone 9's orchestrator and sub-planners
//! read and plan through it. `anthrex mcp` runs [`serve_stdio`]; each tool
//! call is forwarded to the daemon over its socket by [`forward`].
//!
//! Stdout carries JSON-RPC and nothing else (pitfall 17): nothing in this crate
//! prints, and it installs no tracing subscriber.

pub mod forward;
pub mod tools;
pub mod tools_design;
pub mod tools_orch;
pub mod tools_scout;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, Implementation,
    ListToolsResult, PaginatedRequestParams, ProtocolVersion, ServerCapabilities, ServerConfig,
};
use rmcp::service::{RequestContext, RoleServer};
use rmcp::{ErrorData, ServerHandler, ServiceExt};

pub use forward::{forward, notify_ready};
pub use tools::tools_for;

/// Who this server speaks for: `anthrex mcp --role [--run] [--task] [--scout] [--epic]
/// [--chain] [--lane] --window --socket`. `run_id` is empty for a repository-level scout
/// (M8b decision 15); `epic` is a sub-planner's own (milestone 9 decision 15); `chain`
/// is a chained orchestrator's (milestone 9.3, KG §3.4); `lane` is a racer's
/// (milestone 9.5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpOptions {
    pub role: proto::AgentRole,
    pub run_id: String,
    pub task_id: Option<String>,
    pub scout_id: Option<String>,
    pub epic: Option<String>,
    pub chain: Option<String>,
    pub lane: Option<proto::RaceLane>,
    pub window_id: u32,
    pub socket: PathBuf,
}

/// How long one tool call waits for the engine's answer.
pub const TOOL_REPLY_TIMEOUT: Duration = Duration::from_secs(100);

/// The server name `initialize` reports; Claude names the tools `mcp__anthrex__*`.
pub const SERVER_NAME: &str = "anthrex";

/// Serves MCP over `reader`/`writer` until the client closes its end.
pub async fn serve_on<R, W>(opts: McpOptions, reader: R, writer: W) -> anyhow::Result<()>
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
    W: tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let server = Server {
        opts: Arc::new(opts),
        ready: Arc::default(),
    };
    let notice = server.ready.clone();
    let served = server.serve((reader, writer)).await?.waiting().await;
    // Decision 38: a client that closed at once still has its notice sent.
    let pending = crate::lock_ready(&notice).take();
    if let Some(handle) = pending {
        let _ = handle.await;
    }
    served?;
    Ok(())
}

/// Serves MCP on this process's stdin and stdout.
pub async fn serve_stdio(opts: McpOptions) -> anyhow::Result<()> {
    let (stdin, stdout) = rmcp::transport::stdio();
    serve_on(opts, stdin, stdout).await
}

#[derive(Clone)]
struct Server {
    opts: Arc<McpOptions>,
    /// Milestone 9.5 decision 38: the orchestrator's one `McpReady`, sent after its
    /// first `tools/list` is answered; `Some` once started.
    ready: Arc<std::sync::Mutex<Option<tokio::task::JoinHandle<()>>>>,
}

fn lock_ready<T>(m: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl ServerHandler for Server {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new(SERVER_NAME, env!("CARGO_PKG_VERSION")))
            .with_protocol_version(ProtocolVersion::V_2025_06_18)
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        let mut ready = lock_ready(&self.ready);
        if self.opts.role == proto::AgentRole::Orchestrator && ready.is_none() {
            let opts = self.opts.clone();
            // Beside the answer: the daemon still pastes only into a quiet window that
            // has sent a signal (`driver/wake_first_turn.rs`).
            *ready = Some(tokio::spawn(async move { notify_ready(&opts).await }));
        }
        Ok(ListToolsResult::with_all_items(tools_for(self.opts.role)))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let tool = request.name.as_ref();
        let (ok, text) = if tools::allowed(self.opts.role, tool) {
            let args = serde_json::Value::Object(request.arguments.unwrap_or_default());
            forward(&self.opts, tool, args).await
        } else {
            (
                false,
                format!(
                    "tool {tool} is not available to the {} role",
                    tools::role_name(self.opts.role)
                ),
            )
        };
        let content = vec![ContentBlock::text(text)];
        let result = if ok {
            CallToolResult::success(content)
        } else {
            CallToolResult::error(content)
        };
        Ok(result.into())
    }
}
