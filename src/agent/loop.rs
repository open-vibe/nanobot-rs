use crate::agent::context::ContextBuilder;
use crate::agent::subagent::SubagentManager;
use crate::agent::turn_guard::TurnGuard;
use crate::bus::{InboundMessage, MessageBus, OutboundMessage};
use crate::config::WebSearchConfig;
use crate::cron::CronService;
use crate::memory::MemoryStore;
use crate::providers::base::LLMProvider;
use crate::session::SessionManager;
use crate::tools::cron::CronTool;
use crate::tools::filesystem::{EditFileTool, ListDirTool, ReadFileTool, WriteFileTool};
use crate::tools::http::HttpRequestTool;
use crate::tools::message::MessageTool;
use crate::tools::registry::ToolRegistry;
use crate::tools::shell::ExecTool;
use crate::tools::spawn::SpawnTool;
use crate::tools::web::{WebFetchTool, WebSearchTool};
use anyhow::{Context, Result};
use chrono::Local;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::time::{Duration, timeout};

pub struct AgentLoop {
    bus: Arc<MessageBus>,
    provider: Arc<dyn LLMProvider>,
    workspace: PathBuf,
    model: String,
    max_iterations: u32,
    memory_window: usize,
    context: ContextBuilder,
    sessions: Arc<SessionManager>,
    tools: ToolRegistry,
    message_tool: Arc<MessageTool>,
    spawn_tool: Arc<SpawnTool>,
    cron_tool: Option<Arc<CronTool>>,
    subagents: Arc<SubagentManager>,
    running: AtomicBool,
}

impl AgentLoop {
    fn available_tools_text(&self) -> String {
        let mut tool_names = self.tools.tool_names();
        tool_names.sort();
        if tool_names.is_empty() {
            "(none)".to_string()
        } else {
            tool_names.join(", ")
        }
    }

    fn runtime_facts_message(&self) -> serde_json::Value {
        let tools_text = self.available_tools_text();

        json!({
            "role": "system",
            "content": format!(
                "Runtime facts (authoritative): active model is '{model}'; available tools are: {tools}. \
        If a user asks for external actions (network/file/command/scheduling), do not claim tools are unavailable; call the matching tool directly. \
        Focus on the current user message only; do not summarize prior tasks unless explicitly requested.",
                model = self.model,
                tools = tools_text
            )
        })
    }

    fn build_turn_messages(
        &self,
        history: &[Value],
        current_message: &str,
        channel: &str,
        chat_id: &str,
        media: Option<&[String]>,
    ) -> Vec<Value> {
        let mut messages = self.context.build_messages(
            history,
            current_message,
            None,
            Some(channel),
            Some(chat_id),
            media,
        );
        messages.insert(1, self.runtime_facts_message());
        messages
    }

    fn extract_json_object(text: &str) -> Option<Value> {
        let trimmed = text.trim();
        if let Ok(value) = serde_json::from_str::<Value>(trimmed)
            && value.is_object()
        {
            return Some(value);
        }
        if trimmed.starts_with("```") {
            let mut lines = trimmed.lines();
            let _ = lines.next();
            let body = lines.collect::<Vec<_>>().join("\n");
            let stripped = body.rsplit_once("```").map(|(v, _)| v).unwrap_or(&body);
            if let Ok(value) = serde_json::from_str::<Value>(stripped.trim())
                && value.is_object()
            {
                return Some(value);
            }
        }
        None
    }

    pub fn new(
        bus: Arc<MessageBus>,
        provider: Arc<dyn LLMProvider>,
        workspace: PathBuf,
        model: Option<String>,
        max_iterations: u32,
        memory_window: usize,
        web_search: WebSearchConfig,
        exec_timeout_s: u64,
        restrict_to_workspace: bool,
        cron_service: Option<Arc<CronService>>,
        session_manager: Option<Arc<SessionManager>>,
    ) -> Result<Self> {
        let context = ContextBuilder::new(workspace.clone())?;
        let sessions = session_manager.unwrap_or(Arc::new(SessionManager::new()?));
        let mut tools = ToolRegistry::new();
        let model_name = model.unwrap_or_else(|| provider.default_model().to_string());

        let allowed_dir = if restrict_to_workspace {
            Some(workspace.clone())
        } else {
            None
        };

        tools.register(Arc::new(ReadFileTool::new(allowed_dir.clone())));
        tools.register(Arc::new(WriteFileTool::new(allowed_dir.clone())));
        tools.register(Arc::new(EditFileTool::new(allowed_dir.clone())));
        tools.register(Arc::new(ListDirTool::new(allowed_dir.clone())));
        tools.register(Arc::new(ExecTool::new(
            exec_timeout_s,
            Some(workspace.clone()),
            None,
            None,
            restrict_to_workspace,
        )));
        tools.register(Arc::new(WebSearchTool::from_config(web_search.clone())));
        tools.register(Arc::new(WebFetchTool::new(50_000)));
        tools.register(Arc::new(HttpRequestTool::new(30, 50_000)));

        let message_tool = Arc::new(MessageTool::new(bus.outbound_sender()));
        tools.register(message_tool.clone());

        let subagents = Arc::new(SubagentManager::new(
            provider.clone(),
            workspace.clone(),
            bus.clone(),
            model_name.clone(),
            web_search,
            exec_timeout_s,
            restrict_to_workspace,
        ));
        let spawn_tool = Arc::new(SpawnTool::new(subagents.clone()));
        tools.register(spawn_tool.clone());

        let cron_tool = if let Some(cron_service) = cron_service {
            let tool = Arc::new(CronTool::new(cron_service));
            tools.register(tool.clone());
            Some(tool)
        } else {
            None
        };

        Ok(Self {
            bus,
            provider: provider.clone(),
            workspace,
            model: model_name,
            max_iterations,
            memory_window,
            context,
            sessions,
            tools,
            message_tool,
            spawn_tool,
            cron_tool,
            subagents,
            running: AtomicBool::new(false),
        })
    }

    pub async fn run(&self) -> Result<()> {
        self.running.store(true, Ordering::Relaxed);
        while self.running.load(Ordering::Relaxed) {
            let message = timeout(Duration::from_secs(1), self.bus.consume_inbound()).await;
            let Some(msg) = (match message {
                Ok(v) => v,
                Err(_) => continue,
            }) else {
                continue;
            };

            let response = match self.process_message(msg.clone(), None).await {
                Ok(resp) => resp,
                Err(err) => {
                    let mut out = OutboundMessage::new(
                        msg.channel.clone(),
                        msg.chat_id.clone(),
                        format!("Sorry, I encountered an error: {err}"),
                    );
                    out.metadata = msg.metadata.clone();
                    out
                }
            };
            let _ = self.bus.publish_outbound(response).await;
        }
        Ok(())
    }

    pub fn stop(&self) {
        self.running.store(false, Ordering::Relaxed);
    }

    async fn process_message(
        &self,
        msg: InboundMessage,
        session_key: Option<&str>,
    ) -> Result<OutboundMessage> {
        if msg.channel == "system" {
            return self.process_system_message(msg).await;
        }

        let mut session = self
            .sessions
            .get_or_create(session_key.unwrap_or(&msg.session_key()));
        if session.messages.len() > self.memory_window {
            if let Err(err) = self.consolidate_memory(&mut session).await {
                eprintln!("Warning: memory consolidation failed: {err}");
            }
        }
        self.message_tool
            .set_context(msg.channel.clone(), msg.chat_id.clone());
        self.spawn_tool
            .set_context(msg.channel.clone(), msg.chat_id.clone());
        if let Some(cron_tool) = &self.cron_tool {
            cron_tool.set_context(msg.channel.clone(), msg.chat_id.clone());
        }

        let media = if msg.media.is_empty() {
            None
        } else {
            Some(msg.media.as_slice())
        };
        // Deterministic anti-contamination: only current turn is sent to the model.
        let history = session.get_history(0);
        let mut messages =
            self.build_turn_messages(&history, &msg.content, &msg.channel, &msg.chat_id, media);

        let mut final_content: Option<String> = None;
        let mut retried_with_fresh_context = false;
        let mut tools_used: Vec<String> = Vec::new();
        let turn_guard = TurnGuard::new(
            self.provider.as_ref(),
            &self.model,
            self.available_tools_text(),
            self.max_iterations,
        );
        for iteration in 1..=self.max_iterations {
            let tool_defs = self.tools.get_definitions();
            let response = self
                .provider
                .chat(&messages, Some(&tool_defs), Some(&self.model), 4096, 0.7)
                .await?;

            if response.has_tool_calls() {
                let tool_call_dicts = response
                    .tool_calls
                    .iter()
                    .map(|tc| {
                        json!({
                            "id": tc.id,
                            "type": "function",
                            "function": {
                                "name": tc.name,
                                "arguments": serde_json::to_string(&tc.arguments).unwrap_or_else(|_| "{}".to_string()),
                            }
                        })
                    })
                    .collect::<Vec<_>>();
                self.context.add_assistant_message(
                    &mut messages,
                    response.content.as_deref(),
                    Some(tool_call_dicts),
                    response.reasoning_content.as_deref(),
                );

                for tool_call in response.tool_calls {
                    tools_used.push(tool_call.name.clone());
                    let result = self
                        .tools
                        .execute(&tool_call.name, &tool_call.arguments)
                        .await;
                    self.context.add_tool_result(
                        &mut messages,
                        &tool_call.id,
                        &tool_call.name,
                        &result,
                    );
                }
                messages.push(json!({
                    "role": "user",
                    "content": "Reflect on the results and decide next steps."
                }));
            } else {
                if turn_guard
                    .should_retry_after_false_no_tools_claim(response.content.as_deref(), iteration)
                    .await
                {
                    if !retried_with_fresh_context {
                        messages = self.build_turn_messages(
                            &[],
                            &msg.content,
                            &msg.channel,
                            &msg.chat_id,
                            media,
                        );
                        messages.push(turn_guard.correction_message());
                        retried_with_fresh_context = true;
                        continue;
                    }
                    final_content = Some(turn_guard.tools_available_response());
                    break;
                }
                final_content = response.content;
                break;
            }
        }

        let answer = final_content.unwrap_or_else(|| {
            "I've completed processing but have no response to give.".to_string()
        });

        session.add_message("user", &msg.content);
        session.add_message_with_tools("assistant", &answer, Some(&tools_used));
        self.sessions.save(&session)?;

        let mut outbound = OutboundMessage::new(msg.channel, msg.chat_id, answer);
        outbound.metadata = msg.metadata;
        Ok(outbound)
    }

    async fn process_system_message(&self, msg: InboundMessage) -> Result<OutboundMessage> {
        let (origin_channel, origin_chat_id) = msg
            .chat_id
            .split_once(':')
            .map(|(c, id)| (c.to_string(), id.to_string()))
            .unwrap_or_else(|| ("cli".to_string(), msg.chat_id.clone()));

        self.message_tool
            .set_context(origin_channel.clone(), origin_chat_id.clone());
        self.spawn_tool
            .set_context(origin_channel.clone(), origin_chat_id.clone());
        if let Some(cron_tool) = &self.cron_tool {
            cron_tool.set_context(origin_channel.clone(), origin_chat_id.clone());
        }

        let session_key = format!("{origin_channel}:{origin_chat_id}");
        let mut session = self.sessions.get_or_create(&session_key);
        // Deterministic anti-contamination: only current turn is sent to the model.
        let history = session.get_history(0);
        let mut messages = self.build_turn_messages(
            &history,
            &msg.content,
            &origin_channel,
            &origin_chat_id,
            None,
        );

        let mut final_content: Option<String> = None;
        let mut retried_with_fresh_context = false;
        let turn_guard = TurnGuard::new(
            self.provider.as_ref(),
            &self.model,
            self.available_tools_text(),
            self.max_iterations,
        );
        for iteration in 1..=self.max_iterations {
            let tool_defs = self.tools.get_definitions();
            let response = self
                .provider
                .chat(&messages, Some(&tool_defs), Some(&self.model), 4096, 0.7)
                .await?;

            if response.has_tool_calls() {
                let tool_call_dicts = response
                    .tool_calls
                    .iter()
                    .map(|tc| {
                        json!({
                            "id": tc.id,
                            "type": "function",
                            "function": {
                                "name": tc.name,
                                "arguments": serde_json::to_string(&tc.arguments).unwrap_or_else(|_| "{}".to_string()),
                            }
                        })
                    })
                    .collect::<Vec<_>>();
                self.context.add_assistant_message(
                    &mut messages,
                    response.content.as_deref(),
                    Some(tool_call_dicts),
                    response.reasoning_content.as_deref(),
                );

                for tool_call in response.tool_calls {
                    let result = self
                        .tools
                        .execute(&tool_call.name, &tool_call.arguments)
                        .await;
                    self.context.add_tool_result(
                        &mut messages,
                        &tool_call.id,
                        &tool_call.name,
                        &result,
                    );
                }
                messages.push(json!({
                    "role": "user",
                    "content": "Reflect on the results and decide next steps."
                }));
            } else {
                if turn_guard
                    .should_retry_after_false_no_tools_claim(response.content.as_deref(), iteration)
                    .await
                {
                    if !retried_with_fresh_context {
                        messages = self.build_turn_messages(
                            &[],
                            &msg.content,
                            &origin_channel,
                            &origin_chat_id,
                            None,
                        );
                        messages.push(turn_guard.correction_message());
                        retried_with_fresh_context = true;
                        continue;
                    }
                    final_content = Some(turn_guard.tools_available_response());
                    break;
                }
                final_content = response.content;
                break;
            }
        }

        let answer = final_content.unwrap_or_else(|| "Background task completed.".to_string());
        session.add_message(
            "user",
            &format!("[System: {}] {}", msg.sender_id, msg.content),
        );
        session.add_message("assistant", &answer);
        self.sessions.save(&session)?;

        Ok(OutboundMessage::new(origin_channel, origin_chat_id, answer))
    }

    async fn consolidate_memory(&self, session: &mut crate::session::Session) -> Result<()> {
        let memory = MemoryStore::new(self.workspace.clone())?;
        let keep_count = usize::min(10, usize::max(2, self.memory_window / 2));
        if session.messages.len() <= keep_count {
            return Ok(());
        }

        let split_idx = session.messages.len() - keep_count;
        let old_messages = &session.messages[..split_idx];
        let mut lines = Vec::new();
        for msg in old_messages {
            let Some(content) = msg.get("content").and_then(Value::as_str) else {
                continue;
            };
            if content.trim().is_empty() {
                continue;
            }
            let timestamp = msg
                .get("timestamp")
                .and_then(Value::as_str)
                .unwrap_or("?")
                .chars()
                .take(16)
                .collect::<String>();
            let role = msg
                .get("role")
                .and_then(Value::as_str)
                .unwrap_or("user")
                .to_ascii_uppercase();
            let tools_suffix = msg
                .get("tools_used")
                .and_then(Value::as_array)
                .filter(|tools| !tools.is_empty())
                .map(|tools| {
                    let list = tools
                        .iter()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join(", ");
                    if list.is_empty() {
                        String::new()
                    } else {
                        format!(" [tools: {list}]")
                    }
                })
                .unwrap_or_default();
            lines.push(format!(
                "[{timestamp}] {role}{tools_suffix}: {content}",
                content = content.trim()
            ));
        }

        if lines.is_empty() {
            session.messages = session.messages[split_idx..].to_vec();
            self.sessions.save(session)?;
            return Ok(());
        }

        let current_memory = memory.read_long_term();
        let now = Local::now().format("%Y-%m-%d %H:%M").to_string();
        let prompt = format!(
            "You are a memory consolidation agent. Process this conversation and return a JSON object with exactly two keys:\n\n\
1. \"history_entry\": A paragraph (2-5 sentences) summarizing the key events/decisions/topics. Start with a timestamp like [{now}]. Include enough detail to be useful when found by grep search later.\n\n\
2. \"memory_update\": The updated long-term memory content. Add any new facts: user preferences, personal info, habits, project context, technical decisions, tools/services used. If nothing new, return the existing content unchanged.\n\n\
## Current Long-term Memory\n{current_memory}\n\n\
## Conversation to Process\n{conversation}\n\n\
Respond with ONLY valid JSON, no markdown fences.",
            current_memory = if current_memory.trim().is_empty() {
                "(empty)"
            } else {
                current_memory.trim()
            },
            conversation = lines.join("\n")
        );

        let response = self
            .provider
            .chat(
                &[
                    json!({
                        "role": "system",
                        "content": "You are a memory consolidation agent. Respond only with valid JSON."
                    }),
                    json!({
                        "role": "user",
                        "content": prompt
                    }),
                ],
                None,
                Some(&self.model),
                1200,
                0.0,
            )
            .await?;

        let parsed = response
            .content
            .as_deref()
            .and_then(Self::extract_json_object)
            .context("memory consolidation returned non-JSON content")?;

        if let Some(entry) = parsed.get("history_entry").and_then(Value::as_str)
            && !entry.trim().is_empty()
        {
            memory.append_history(entry)?;
        }
        if let Some(update) = parsed.get("memory_update").and_then(Value::as_str)
            && update.trim() != current_memory.trim()
        {
            memory.write_long_term(update)?;
        }

        session.messages = session.messages[split_idx..].to_vec();
        self.sessions.save(session)?;
        Ok(())
    }

    pub async fn process_direct(
        &self,
        content: &str,
        session_key: Option<&str>,
        channel: Option<&str>,
        chat_id: Option<&str>,
    ) -> Result<String> {
        let session_key = session_key.unwrap_or("cli:direct");
        let (default_channel, default_chat_id) = session_key
            .split_once(':')
            .map(|(c, id)| (c.to_string(), id.to_string()))
            .unwrap_or_else(|| ("cli".to_string(), "direct".to_string()));
        let channel = channel.unwrap_or(&default_channel);
        let chat_id = chat_id.unwrap_or(&default_chat_id);

        let msg = InboundMessage::new(channel, "user", chat_id, content);
        let response = self.process_message(msg, Some(session_key)).await?;
        Ok(response.content)
    }

    pub fn workspace(&self) -> &PathBuf {
        &self.workspace
    }

    pub async fn running_subagents(&self) -> usize {
        self.subagents.get_running_count().await
    }
}
