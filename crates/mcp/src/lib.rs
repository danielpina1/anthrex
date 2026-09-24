//! The anthrex MCP server (decisions 4 and 5): headless worker and reviewer agents
//! report to the run engine through it. `anthrex mcp` runs [`serve_stdio`]; each tool
//! call is forwarded to the daemon over its socket by [`forward`].
//!
//! Stdout carries JSON-RPC and nothing else (pitfall 17): nothing in this crate
//! prints, and it installs no tracing subscriber.

pub mod forward;
pub mod tools;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, Implementation,
    ListToolsResult, PaginatedRequestParams, ProtocolVersion, ServerCapabilities, ServerConfig,
};
use rmcp::service::{RequestContext, RoleServer};
use rmcp::{ErrorData, ServerHandler, ServiceExt};

pub use forward::forward;
pub use tools::tools_for;

/// Who this server speaks for: `anthrex mcp --role --run [--task] --window --socket`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpOptions {
    pub role: proto::AgentRole,
    pub run_id: String,
    pub task_id: Option<String>,
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
    };
    server.serve((reader, writer)).await?.waiting().await?;
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
