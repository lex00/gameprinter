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
    PaginatedRequestParams, ProgressNotificationParam, ServerCapabilities, ServerConfig,
    ServerRequest, Tool,
};
use rmcp::service::{PeerRequestOptions, RequestContext};
use rmcp::{ErrorData, RoleServer, ServerHandler, ServiceExt};
use serde_json::{Value, json};

use crate::{Config, Reply, elicit, tools};

/// How often a long tool call reports progress.
pub const PROGRESS_EVERY: std::time::Duration = std::time::Duration::from_secs(1);

/// What the server tells a client when it connects.
pub const INSTRUCTIONS: &str = "In Game Printer, a model is a game's rules and numbers as plain text (model.toml), read when the game loads and never compiled, and a template is a game's folder holding a model and nothing from any ROM, ready to copy. What a person makes here plays in a browser with nothing installed; it is never a new ROM. Porting a ROM the person owns is what gp is for, and you start it when they ask (roms add, roms scan, game new): gp reads their ROM on their own machine the way an emulator does, to play it, record clips and read its code; the model is the game's rules and numbers written as text and checked against clips of that ROM; no ROM bytes, art lifted from the ROM or built game go in any repo, commit or release; and the port reads its art and levels from the player's own ROM when it plays, so it plays only for someone who has the ROM. That is not recreating someone else's game. A new game or theme of the person's own is a copy of a template with new art. Some games also have custom code (Rust hooks) for what the model text can't say. Every ported game's folder in the games checkout is a template, and so is each folder under the repo's templates/ (Moss Hop and the games made here). Find a template with registry_search and copy it for the person with template_fork (its reply says whether the copy is ready and, if not, the next step), or start a game from nothing with gp_run running game new NAME with empty true (one stage, a player that walks and jumps, plain pictures, no sound, ready to build; every tool below then names it as a copy), read the copy with surface_describe, change it with model_edit and contract_edit (model paths, docs/formats/model-path.md), replace its art with assets_generate and assets_place, edit a sprite or a level tile with assets_edit (assets_history reads what the person decided on its card; assets_export hands a sprite to their pixel editor), change its music and effects with sound_edit after sound_describe (sound_banks lists what they can play through, sound_preview plays a run with them), try it with run_preview, hold it to its contract with validate, measure and correct a ported game's feel with model_refine (asking the person model_questions), and build it for the browser with build_publish. When the person wants their own registry and page (a theme maker hosting their theme, say), host_your_own walks the road step by step: their repo, the registry in it, their namespace, their theme, the check from outside, the public page with their registry as r= (games still come from the page's own registry; theirs adds themes), and optionally a page of their own; run each step through gp_run and ask before every one that writes to the outside world. When the person points at the screen on their page and says what's wrong, it comes as their turn in the talk of the mechanic open there, with point (the recording, the frame, the box and what is under it): gp mechanic wait GAME (through gp_run) waits for it and gp mechanic show GAME MECHANIC reads it; answer in plain words with gp mechanic say GAME MECHANIC. When the turn asks what something is (point.ask is explain), run the gp element explain call it names (point.explain) through gp_run and answer from its output, saying what it could not determine. Show them what you talk about with screen_show, and find the moment their words name in a recording (the second jump) with moments. To read a ROM's code as it runs, watch_start starts a watch session and watch_events follows it; watch_watch and watch_hook run it again watching addresses or hooking instructions, watch_detail gives a window's instructions, watch_explore searches again, watch_point says what is under a box of the screen and view_now what the person's watch page shows. Every edit is checked and written in canonical form; a refused edit writes nothing. When the loop's runner is on, the loop does the work, and your job is the setup, not the work: run gp loop health through gp_run at the start and after anything changes, and fix each PROBLEM it lists (a gp serve on an old build or down: restart it on the published gp; a stuck row or card: set it from its worker's report; a bug in the loop or the tools: fix it and land it through gp tool gate); put what the person reports into the quest log as a row (gp mechanic queue) instead of doing it yourself, tell them in one line what is on their page, and never do a row's work, raise a row's card or write their games checkout yourself. Without a runner, call next at the start of every conversation and after each step you take, tell the person where things stand and offer its next step in plain words without being asked, start what it marks for the tools yourself, and stop and ask before anything in waits_for_yes, which costs money or makes something public, until the person says yes to that item. While the person plays, call next with wait_s so you hear when they hand you something. The named tools are the short road for common steps; gp_commands lists every other gp command (porting steps such as roms scan, game new, corpus score and progress among them) and gp_run runs any of them. gp_run runs reads and writes to the person's games, copies and home at once, but a change to a file already in their games checkout is put back and held until their yes on its card, and anything that publishes, lands a change, spends money (paid clip agents too) or downloads a lot runs only with the person's yes. While their page is open, gp_run's refusal puts a card with YES and NO on it (ask_id): tell them what it is, wait with next and wait_s until waits_for_yes shows it answered, then call gp_run again with ask_id; a yes runs that exact call once. With no page open, a client that shows forms (MCP elicitation) gets the question from gp_run itself, in the card's own words: the person answers it there, and their answer stands whatever confirmed_by_person says; a client that shows none needs confirmed_by_person, set only after the person said yes to that exact action here. template_fork asks the same way for the path of the person's own ROM when a copy needs one that isn't in their ROM folder.";

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
/// text, for clients that read only text; when the reply has a next step
/// (crate::next_step), a last text item gives it in one line.
pub fn tool_result(reply: &Reply) -> CallToolResult {
    let text = serde_json::to_string_pretty(&reply.json).expect("JSON serializes");
    let mut content = vec![json!({ "type": "text", "text": text })];
    if let Some(l) = crate::next_step::line(&reply.json) {
        content.push(json!({ "type": "text", "text": l }));
    }
    serde_json::from_value(json!({
        "content": content,
        "structuredContent": reply.json,
        "isError": reply.is_error,
    }))
    .expect("a tool result fits the SDK's type")
}

#[derive(Clone)]
pub struct Server {
    config: Arc<Config>,
    /// The version it says it is: the gp serving it, not this crate.
    version: Arc<str>,
}

impl Server {
    pub fn new(config: Config, version: &str) -> Self {
        Self {
            config: Arc::new(config),
            version: version.into(),
        }
    }
}

impl ServerHandler for Server {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("game-printer", &*self.version))
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
        let config = self.config.clone();
        // A client that shows forms gets the tool's questions to the person
        // as forms (crate::elicit).
        let asker = form_asker(&context);
        // Tools block (child processes, the interpreter), so they run off
        // the runtime's thread.
        let tool = name.clone();
        // Every reply ends with the next step (crate::next_step).
        let mut handle = tokio::task::spawn_blocking(move || {
            elicit::with(asker, || {
                let mut r = tools::call(&config, &tool, args.clone())?;
                crate::next_step::attach(&config, &tool, &args, &mut r);
                Some(r)
            })
        });
        // While a tool runs, a client that sent a progress token hears from
        // it every PROGRESS_EVERY: how long it has run.
        let token = context.meta.get_progress_token();
        let started = std::time::Instant::now();
        let mut ticks = 0u32;
        let joined = loop {
            match tokio::time::timeout(PROGRESS_EVERY, &mut handle).await {
                Ok(done) => break done,
                Err(_) => {
                    let Some(token) = token.clone() else { continue };
                    ticks += 1;
                    let message = format!("{name} running, {} s", started.elapsed().as_secs());
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

/// How long a form waits for the person before the question goes back to
/// the agent as before.
pub const FORM_WAITS: std::time::Duration = std::time::Duration::from_secs(15 * 60);

/// Whether a client's start (its initialize params, as JSON) says it shows
/// forms: an `elicitation` capability, with form mode or no mode named, on
/// a protocol before 2026-07-28 (which forbids a server's own requests;
/// its forms come back in a tool's result, which this server doesn't send).
pub fn shows_forms(client: &Value) -> bool {
    let e = &client["capabilities"]["elicitation"];
    let before = client["protocolVersion"]
        .as_str()
        .is_some_and(|v| v < "2026-07-28");
    before && e.is_object() && (e["form"].is_object() || e["url"].is_null())
}

/// The way a tool call asks the person with a form (`elicitation/create`
/// to the client), when the client said it shows them: run on the tool's
/// blocking thread, it waits on the runtime for the answer.
fn form_asker(context: &RequestContext<RoleServer>) -> Option<Arc<elicit::Asker>> {
    let info = context.peer.peer_info()?;
    if !shows_forms(&serde_json::to_value(&*info).ok()?) {
        return None;
    }
    let peer = context.peer.clone();
    let rt = tokio::runtime::Handle::current();
    Some(Arc::new(move |message: &str, schema: Value| {
        let form = json!({
            "method": "elicitation/create",
            "params": {"mode": "form", "message": message, "requestedSchema": schema},
        });
        let req: ServerRequest = match serde_json::from_value(form) {
            Ok(r) => r,
            Err(e) => return elicit::Answer::NoForm(format!("the form isn't the SDK's: {e}")),
        };
        let mut opts = PeerRequestOptions::no_options();
        opts.timeout = Some(FORM_WAITS);
        let got = rt.block_on(async {
            peer.send_request_with_option(req, opts)
                .await?
                .await_response()
                .await
        });
        match got {
            Ok(r) => answer_of(&serde_json::to_value(r).unwrap_or(Value::Null)),
            Err(e) => elicit::Answer::NoForm(format!("the form didn't come back: {e}")),
        }
    }))
}

/// The person's answer from the client's reply to a form (an
/// `ElicitResult` as JSON).
pub fn answer_of(r: &Value) -> elicit::Answer {
    match r["action"].as_str() {
        Some("accept") => elicit::Answer::Sent(r["content"].clone()),
        Some("decline") => elicit::Answer::Declined,
        Some("cancel") => elicit::Answer::Closed,
        _ => elicit::Answer::NoForm(format!("the client's reply to the form isn't one: {r}")),
    }
}

/// Serves the tools on stdin and stdout until the client closes stdin,
/// saying it is `version` (the gp serving them).
pub fn serve_stdio(config: Config, version: &str) -> Result<(), String> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("starting the runtime: {e}"))?;
    rt.block_on(async {
        let running = Server::new(config, version)
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

#[cfg(test)]
mod form_tests {
    use super::*;

    /// A client shows forms when it says so, in form mode or no mode, on a
    /// protocol that lets the server ask; its answer reads as the person's.
    #[test]
    fn a_client_that_says_it_shows_forms_gets_them() {
        let shows = |e: Value, v: &str| {
            let start = json!({"protocolVersion": v, "capabilities": {"elicitation": e}});
            shows_forms(&start)
        };
        let old = "2025-11-25";
        assert!(shows(json!({}), old));
        assert!(shows(json!({"form": {}}), old));
        assert!(!shows(json!({"url": {}}), old));
        assert!(!shows(Value::Null, old));
        assert!(!shows(json!({}), "2026-07-28"));
        let sent = json!({"action": "accept", "content": {"yes": true}});
        let yes = elicit::Answer::Sent(json!({"yes": true}));
        assert_eq!(answer_of(&sent), yes);
        let no = json!({"action": "decline"});
        assert_eq!(answer_of(&no), elicit::Answer::Declined);
        let closed = json!({"action": "cancel"});
        assert_eq!(answer_of(&closed), elicit::Answer::Closed);
        assert!(matches!(answer_of(&json!({})), elicit::Answer::NoForm(_)));
    }
}
