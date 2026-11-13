//! The MCP server: the tools of [`crate::tools`] over stdio, with the
//! official Rust SDK (`rmcp`). Requests are newline-delimited JSON-RPC on
//! stdin and stdout; nothing else may write to stdout while it runs.
//!
//! The SDK's types are built from JSON where they're `non_exhaustive`, so
//! the tool list stays plain data and an SDK update that adds fields
//! doesn't touch this file.

use std::sync::Arc;

use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, Implementation, ListToolsResult,
    PaginatedRequestParams, ProgressNotificationParam, ServerCapabilities, ServerConfig, Tool,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData, RoleServer, ServerHandler, ServiceExt};
use serde_json::{Value, json};

use crate::{Config, Reply, capture, tools};

/// How often a long tool call reports progress.
pub const PROGRESS_EVERY: std::time::Duration = std::time::Duration::from_secs(1);

/// What the server tells a client when it connects.
pub const INSTRUCTIONS: &str = "In Game Printer, a model is a game's rules and numbers as plain text (model.toml), read when the game loads and never compiled, and a template is a game's folder holding a model and nothing from any ROM, ready to copy. What a person makes here plays in a browser with nothing installed; it is never a new ROM. Some games also have custom code (Rust hooks) for what the model text can't say. Every ported game's folder in the games checkout is a template (Donkey Kong is one), and so is each folder under the repo's templates/ (Moss Hop and the games made here). Find a template with registry_search and copy it for the person with template_fork (its reply says whether the copy is ready and, if not, the next step), read the copy with surface_describe, change it with model_edit and contract_edit (model paths, docs/formats/model-path.md), replace its art with assets_generate and assets_place, try it with run_preview, hold it to its contract with validate, measure and correct a ported game's feel with model_refine (asking the person model_questions), and build it for the browser with build_publish. When the person is watching a capture run, pass its name as run to any tool so the call shows in the run's log, read the run with capture_events and send what the person chose with capture_input. When the person points at the screen and says what's wrong, the run has requests: read them with requests and answer each in plain words. Every edit is checked and written in canonical form; a refused edit writes nothing. Call next at the start of every conversation and after each step you take, tell the person where things stand and offer its next step in plain words without being asked, start what it marks for the tools yourself, and stop and ask before anything in waits_for_yes, which costs money or makes something public, until the person says yes to that item. While the person plays, call next with wait_s so you hear when they hand you something.";

/// The tools as MCP tool definitions.
pub fn tool_list() -> Vec<Tool> {
    tools::list()
        .into_iter()
        .map(|t| {
            serde_json::from_value(json!({
                "name": t.name,
                "title": t.title,
                "description": t.description,
                "inputSchema": t.input_schema,
                "annotations": {
                    "title": t.title,
                    "readOnlyHint": t.read_only,
                    "destructiveHint": t.destructive,
                    "idempotentHint": t.idempotent,
                    "openWorldHint": false,
                },
            }))
            .expect("a tool definition fits the SDK's type")
        })
        .collect()
}

/// A reply as an MCP tool result: the JSON as structured content and as
/// text, for clients that read only text.
pub fn tool_result(reply: &Reply) -> CallToolResult {
    let text = serde_json::to_string_pretty(&reply.json).expect("JSON serializes");
    serde_json::from_value(json!({
        "content": [{ "type": "text", "text": text }],
        "structuredContent": reply.json,
        "isError": reply.is_error,
    }))
    .expect("a tool result fits the SDK's type")
}

#[derive(Clone)]
pub struct Server {
    config: Arc<Config>,
}

impl Server {
    pub fn new(config: Config) -> Self {
        Self {
            config: Arc::new(config),
        }
    }
}

impl ServerHandler for Server {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new(
                "game-printer",
                env!("CARGO_PKG_VERSION"),
            ))
            .with_instructions(INSTRUCTIONS)
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        Ok(serde_json::from_value(json!({ "tools": tool_list() })).expect("a tool list"))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let name = request.name.to_string();
        let args = Value::Object(request.arguments.unwrap_or_default());
        let run = args
            .get("run")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .filter(|_| !name.starts_with("capture_"));
        let config = self.config.clone();
        // Tools block (child processes, the interpreter), so they run off
        // the runtime's thread.
        let tool = name.clone();
        let mut handle = tokio::task::spawn_blocking(move || tools::call(&config, &tool, args));
        // While a tool runs, a client that sent a progress token hears from
        // it every PROGRESS_EVERY: the run's newest stage when the call
        // names a capture run, else how long it has run.
        let token = context.meta.get_progress_token();
        let started = std::time::Instant::now();
        let mut seq = run
            .as_deref()
            .map_or(0, |r| capture::last_seq(&self.config, r));
        let mut ticks = 0u32;
        let joined = loop {
            match tokio::time::timeout(PROGRESS_EVERY, &mut handle).await {
                Ok(done) => break done,
                Err(_) => {
                    let Some(token) = token.clone() else { continue };
                    ticks += 1;
                    let mut message = format!("{name} running, {} s", started.elapsed().as_secs());
                    if let Some((s, line)) = run
                        .as_deref()
                        .and_then(|r| capture::progress_line(&self.config, r, seq))
                    {
                        seq = s;
                        message = line;
                    }
                    let _ = context
                        .peer
                        .notify_progress(
                            ProgressNotificationParam::new(token, f64::from(ticks))
                                .with_message(message),
                        )
                        .await;
                }
            }
        };
        let reply = joined
            .map_err(|e| ErrorData::internal_error(format!("the tool panicked: {e}"), None))?;
        match reply {
            Some(r) => Ok(tool_result(&r).into()),
            None => Err(ErrorData::invalid_params(
                format!("there's no tool named `{}`", request.name),
                None,
            )),
        }
    }
}

/// Serves the tools on stdin and stdout until the client closes stdin.
pub fn serve_stdio(config: Config) -> Result<(), String> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("starting the runtime: {e}"))?;
    rt.block_on(async {
        let running = Server::new(config)
            .serve(rmcp::transport::stdio())
            .await
            .map_err(|e| format!("starting the MCP session: {e}"))?;
        running
            .waiting()
            .await
            .map_err(|e| format!("the MCP session: {e}"))?;
        Ok(())
    })
}
