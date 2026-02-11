use crate::agent::context::ContextBuilder;
use crate::agent::subagent::SubagentManager;
use crate::bus::{InboundMessage, MessageBus, OutboundMessage};
use crate::cron::CronService;
use crate::memory::MemoryStore;
use crate::providers::base::LLMProvider;
use crate::session::SessionManager;
use crate::tools::cron::CronTool;
use crate::tools::filesystem::{EditFileTool, ListDirTool, ReadFileTool, WriteFileTool};
use crate::tools::message::MessageTool;
use crate::tools::registry::ToolRegistry;
use crate::tools::shell::ExecTool;
use crate::tools::spawn::SpawnTool;
use crate::tools::web::{WebFetchTool, WebSearchTool};
use anyhow::Result;
use serde_json::json;
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
    pub fn new(
        bus: Arc<MessageBus>,
        provider: Arc<dyn LLMProvider>,
        workspace: PathBuf,
        model: Option<String>,
        max_iterations: u32,
        brave_api_key: Option<String>,
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
        tools.register(Arc::new(WebSearchTool::new(brave_api_key, 5)));
        tools.register(Arc::new(WebFetchTool::new(50_000)));

        let message_tool = Arc::new(MessageTool::new(bus.outbound_sender()));
        tools.register(message_tool.clone());

        let subagents = Arc::new(SubagentManager::new(
            provider.clone(),
            workspace.clone(),
            bus.clone(),
            model_name.clone(),
            None,
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

            let response = match self.process_message(msg.clone()).await {
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

    async fn process_message(&self, msg: InboundMessage) -> Result<OutboundMessage> {
        if msg.channel == "system" {
            return self.process_system_message(msg).await;
        }

        let mut session = self.sessions.get_or_create(&msg.session_key());
        if let Err(err) = self.persist_explicit_memory_request(&msg.content) {
            eprintln!("Warning: failed to persist explicit memory request: {err}");
        }
        self.message_tool
            .set_context(msg.channel.clone(), msg.chat_id.clone());
        self.spawn_tool
            .set_context(msg.channel.clone(), msg.chat_id.clone());
        if let Some(cron_tool) = &self.cron_tool {
            cron_tool.set_context(msg.channel.clone(), msg.chat_id.clone());
        }

        let history = session.get_history(50);
        let mut messages = self.context.build_messages(
            &history,
            &msg.content,
            None,
            Some(&msg.channel),
            Some(&msg.chat_id),
        );

        let mut final_content: Option<String> = None;
        for _ in 0..self.max_iterations {
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
            } else {
                final_content = response.content;
                break;
            }
        }

        let answer = final_content.unwrap_or_else(|| {
            "I've completed processing but have no response to give.".to_string()
        });

        session.add_message("user", &msg.content);
        session.add_message("assistant", &answer);
        self.sessions.save(&session)?;

        let mut outbound = OutboundMessage::new(msg.channel, msg.chat_id, answer);
        outbound.metadata = msg.metadata;
        Ok(outbound)
    }

    fn persist_explicit_memory_request(&self, content: &str) -> Result<()> {
        let Some(fact) = MemoryStore::extract_explicit_memory(content) else {
            return Ok(());
        };
        let memory = MemoryStore::new(self.workspace.clone())?;
        let _ = memory.remember_fact(&fact)?;
        Ok(())
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
        let mut messages = self.context.build_messages(
            &session.get_history(50),
            &msg.content,
            None,
            Some(&origin_channel),
            Some(&origin_chat_id),
        );

        let mut final_content: Option<String> = None;
        for _ in 0..self.max_iterations {
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
            } else {
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
        let response = self.process_message(msg).await?;
        Ok(response.content)
    }

    pub fn workspace(&self) -> &PathBuf {
        &self.workspace
    }

    pub async fn running_subagents(&self) -> usize {
        self.subagents.get_running_count().await
    }
}
