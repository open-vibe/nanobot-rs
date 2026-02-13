use crate::VERSION;
use crate::agent::AgentLoop;
use crate::config::{load_config, providers_status};
use crate::health::collect_health;
use crate::pairing::list_pending;
use crate::providers::base::LLMProvider;
use crate::providers::litellm::LiteLLMProvider;
use crate::session::SessionManager;
use crate::utils::get_data_path;
use anyhow::Result;
use chrono::Local;
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;
use std::sync::mpsc;
use tiny_http::{Header, Method, Request, Response, Server, StatusCode};

const INDEX_HTML: &str = include_str!("index.html");
const APP_CSS: &str = include_str!("app.css");
const APP_JS: &str = include_str!("app.js");

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChatPayload {
    message: String,
    session: Option<String>,
    channel: Option<String>,
    chat_id: Option<String>,
}

struct ChatRequest {
    message: String,
    session: Option<String>,
    channel: Option<String>,
    chat_id: Option<String>,
    reply_tx: mpsc::Sender<Result<String>>,
}

struct ChatWorker {
    tx: mpsc::Sender<ChatRequest>,
}

impl ChatWorker {
    fn new() -> Self {
        let (tx, rx) = mpsc::channel::<ChatRequest>();
        std::thread::spawn(move || {
            let config = load_config(None).unwrap_or_default();
            let model = config.agents.defaults.model.clone();
            let normalized_model = model.strip_prefix("litellm/").unwrap_or(&model);
            let is_bedrock = normalized_model.starts_with("bedrock/");
            let api_key = config.get_api_key(Some(&model));
            if api_key.is_none() && !is_bedrock {
                let err = "No API key configured. Set providers.*.apiKey in ~/.nanobot/config.json."
                    .to_string();
                while let Ok(req) = rx.recv() {
                    let _ = req.reply_tx.send(Err(anyhow::anyhow!(err.clone())));
                }
                return;
            }

            let runtime = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(err) => {
                    while let Ok(req) = rx.recv() {
                        let _ = req
                            .reply_tx
                            .send(Err(anyhow::anyhow!("failed to initialize runtime: {err}")));
                    }
                    return;
                }
            };

            let bus = Arc::new(crate::bus::MessageBus::new(1024));
            let provider = build_provider(
                &config,
                &model,
                api_key.unwrap_or_else(|| "dummy".to_string()),
            );
            let session_manager = match SessionManager::new() {
                Ok(m) => Arc::new(m),
                Err(err) => {
                    while let Ok(req) = rx.recv() {
                        let _ = req
                            .reply_tx
                            .send(Err(anyhow::anyhow!("failed to init session manager: {err}")));
                    }
                    return;
                }
            };
            let agent = match AgentLoop::new(
                bus,
                provider,
                config.workspace_path(),
                Some(model),
                config.agents.defaults.max_tool_iterations,
                config.agents.defaults.memory_window,
                config.tools.web.search.clone(),
                config.tools.exec.timeout,
                config.tools.restrict_to_workspace,
                None,
                Some(session_manager),
            ) {
                Ok(agent) => Arc::new(agent),
                Err(err) => {
                    while let Ok(req) = rx.recv() {
                        let _ = req
                            .reply_tx
                            .send(Err(anyhow::anyhow!("failed to init agent loop: {err}")));
                    }
                    return;
                }
            };

            while let Ok(req) = rx.recv() {
                let session_key = req.session.as_deref().or(Some("webui:default"));
                let answer = runtime.block_on(agent.process_direct(
                    &req.message,
                    session_key,
                    req.channel.as_deref(),
                    req.chat_id.as_deref(),
                ));
                let _ = req.reply_tx.send(answer);
            }
        });
        Self { tx }
    }

    fn chat(
        &self,
        message: String,
        session: Option<String>,
        channel: Option<String>,
        chat_id: Option<String>,
    ) -> Result<String> {
        let (reply_tx, reply_rx) = mpsc::channel();
        self.tx
            .send(ChatRequest {
                message,
                session,
                channel,
                chat_id,
                reply_tx,
            })
            .map_err(|err| anyhow::anyhow!("chat worker unavailable: {err}"))?;
        reply_rx
            .recv()
            .map_err(|err| anyhow::anyhow!("chat worker response error: {err}"))?
    }
}

struct WebUiContext {
    chat: ChatWorker,
}

fn build_provider(
    config: &crate::config::Config,
    model: &str,
    api_key: String,
) -> Arc<dyn LLMProvider> {
    let api_base = config.get_api_base(Some(model));
    let extra_headers = config
        .get_provider(Some(model))
        .and_then(|p| p.extra_headers.clone());
    let provider_name = config.get_provider_name(Some(model));
    Arc::new(LiteLLMProvider::new(
        api_key,
        api_base,
        model.to_string(),
        extra_headers,
        provider_name.as_deref(),
    ))
}

fn content_type_header(value: &str) -> Option<Header> {
    Header::from_bytes(b"Content-Type".as_slice(), value.as_bytes()).ok()
}

fn respond(req: Request, status: u16, content_type: &str, body: String) {
    let mut response = Response::from_string(body).with_status_code(StatusCode(status));
    if let Some(header) = content_type_header(content_type) {
        response.add_header(header);
    }
    let _ = req.respond(response);
}

fn read_cron_jobs() -> Vec<Value> {
    let path = match get_data_path() {
        Ok(p) => p.join("cron").join("jobs.json"),
        Err(_) => return Vec::new(),
    };
    let raw = match std::fs::read_to_string(path) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    let value: Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    value
        .get("jobs")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

fn list_sessions() -> Vec<String> {
    SessionManager::new()
        .and_then(|m| m.list_session_keys())
        .unwrap_or_default()
}

fn enabled_channels(config: &crate::config::Config) -> Vec<&'static str> {
    let mut out = Vec::new();
    if config.channels.telegram.enabled {
        out.push("telegram");
    }
    if config.channels.discord.enabled {
        out.push("discord");
    }
    if config.channels.whatsapp.enabled {
        out.push("whatsapp");
    }
    if config.channels.feishu.enabled {
        out.push("feishu");
    }
    if config.channels.mochat.enabled {
        out.push("mochat");
    }
    if config.channels.dingtalk.enabled {
        out.push("dingtalk");
    }
    if config.channels.email.enabled {
        out.push("email");
    }
    if config.channels.slack.enabled {
        out.push("slack");
    }
    if config.channels.qq.enabled {
        out.push("qq");
    }
    out
}

fn snapshot() -> Value {
    let config = load_config(None).unwrap_or_default();
    let health = collect_health(&config).ok();
    let cron_jobs = read_cron_jobs();
    let sessions = list_sessions();
    let pairing_pending = list_pending().unwrap_or_default();
    json!({
        "version": VERSION,
        "generatedAt": Local::now().to_rfc3339(),
        "model": config.agents.defaults.model,
        "providers": providers_status(&config),
        "channelsEnabled": enabled_channels(&config),
        "cronJobs": cron_jobs,
        "sessions": sessions,
        "pairingPending": pairing_pending,
        "health": health,
    })
}

fn read_request_body(req: &mut Request) -> String {
    let mut buf = String::new();
    let _ = req.as_reader().read_to_string(&mut buf);
    buf
}

fn handle_request(mut req: Request, ctx: &WebUiContext) {
    let url = req.url().to_string();
    let method = req.method().clone();

    match (method, url.as_str()) {
        (Method::Get, "/") => respond(req, 200, "text/html; charset=utf-8", INDEX_HTML.to_string()),
        (Method::Get, "/app.css") => respond(req, 200, "text/css; charset=utf-8", APP_CSS.to_string()),
        (Method::Get, "/app.js") => respond(
            req,
            200,
            "application/javascript; charset=utf-8",
            APP_JS.to_string(),
        ),
        (Method::Get, "/api/state") => {
            let body =
                serde_json::to_string_pretty(&snapshot()).unwrap_or_else(|_| "{}".to_string());
            respond(req, 200, "application/json; charset=utf-8", body);
        }
        (Method::Post, "/api/chat") => {
            let raw = read_request_body(&mut req);
            let payload: ChatPayload = match serde_json::from_str(&raw) {
                Ok(v) => v,
                Err(err) => {
                    respond(
                        req,
                        400,
                        "application/json; charset=utf-8",
                        json!({
                            "ok": false,
                            "error": format!("invalid JSON body: {err}")
                        })
                        .to_string(),
                    );
                    return;
                }
            };
            if payload.message.trim().is_empty() {
                respond(
                    req,
                    400,
                    "application/json; charset=utf-8",
                    json!({
                        "ok": false,
                        "error": "message cannot be empty"
                    })
                    .to_string(),
                );
                return;
            }
            match ctx.chat.chat(
                payload.message,
                payload.session,
                payload.channel,
                payload.chat_id,
            ) {
                Ok(answer) => {
                    respond(
                        req,
                        200,
                        "application/json; charset=utf-8",
                        json!({ "ok": true, "response": answer }).to_string(),
                    );
                }
                Err(err) => {
                    respond(
                        req,
                        500,
                        "application/json; charset=utf-8",
                        json!({
                            "ok": false,
                            "error": err.to_string()
                        })
                        .to_string(),
                    );
                }
            }
        }
        (Method::Get, "/api/chat") => {
            respond(
                req,
                405,
                "application/json; charset=utf-8",
                json!({"ok": false, "error": "use POST /api/chat"}).to_string(),
            );
        }
        (_, "/api/chat") | (_, "/api/state") | (_, "/app.css") | (_, "/app.js") | (_, "/") => {
            respond(
                req,
                405,
                "text/plain; charset=utf-8",
                "Method Not Allowed".to_string(),
            );
        }
        _ => respond(
            req,
            404,
            "text/plain; charset=utf-8",
            "Not Found".to_string(),
        ),
    }
}

pub fn run_webui_server(host: &str, port: u16) -> Result<()> {
    let addr = format!("{host}:{port}");
    let server = Server::http(&addr).map_err(|err| anyhow::anyhow!(err.to_string()))?;
    let ctx = WebUiContext {
        chat: ChatWorker::new(),
    };
    println!("WebUI running at http://{addr}");
    for req in server.incoming_requests() {
        handle_request(req, &ctx);
    }
    Ok(())
}
