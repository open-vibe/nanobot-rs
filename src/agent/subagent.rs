use crate::bus::{InboundMessage, MessageBus};
use crate::providers::base::LLMProvider;
use crate::tools::filesystem::{ListDirTool, ReadFileTool, WriteFileTool};
use crate::tools::registry::ToolRegistry;
use crate::tools::shell::ExecTool;
use crate::tools::web::{WebFetchTool, WebSearchTool};
use serde_json::json;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;
use uuid::Uuid;

pub struct SubagentManager {
    provider: Arc<dyn LLMProvider>,
    workspace: PathBuf,
    bus: Arc<MessageBus>,
    model: String,
    brave_api_key: Option<String>,
    exec_timeout_s: u64,
    restrict_to_workspace: bool,
    running_tasks: Arc<Mutex<HashMap<String, tokio::task::JoinHandle<()>>>>,
}

impl SubagentManager {
    pub fn new(
        provider: Arc<dyn LLMProvider>,
        workspace: PathBuf,
        bus: Arc<MessageBus>,
        model: String,
        brave_api_key: Option<String>,
        exec_timeout_s: u64,
        restrict_to_workspace: bool,
    ) -> Self {
        Self {
            provider,
            workspace,
            bus,
            model,
            brave_api_key,
            exec_timeout_s,
            restrict_to_workspace,
            running_tasks: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub async fn spawn(
        &self,
        task: String,
        label: Option<String>,
        origin_channel: String,
        origin_chat_id: String,
    ) -> String {
        let task_id = Uuid::new_v4().simple().to_string()[..8].to_string();
        let display_label = label.unwrap_or_else(|| {
            if task.len() > 30 {
                format!("{}...", &task[..30])
            } else {
                task.clone()
            }
        });

        let provider = self.provider.clone();
        let workspace = self.workspace.clone();
        let model = self.model.clone();
        let brave_api_key = self.brave_api_key.clone();
        let exec_timeout_s = self.exec_timeout_s;
        let restrict_to_workspace = self.restrict_to_workspace;
        let bus = self.bus.clone();
        let task_id_for_cleanup = task_id.clone();
        let task_id_for_run = task_id.clone();
        let running_map = self.running_tasks.clone();
        let task_for_run = task.clone();
        let label_for_run = display_label.clone();

        let handle = tokio::spawn(async move {
            let result = run_subagent(
                provider,
                workspace,
                model,
                brave_api_key,
                exec_timeout_s,
                restrict_to_workspace,
                task_id_for_run.clone(),
                task_for_run.clone(),
                label_for_run.clone(),
            )
            .await;

            let (status, content) = match result {
                Ok(summary) => ("ok", summary),
                Err(err) => ("error", format!("Error: {err}")),
            };

            let status_text = if status == "ok" {
                "completed successfully"
            } else {
                "failed"
            };

            let announce = format!(
                "[Subagent '{label_for_run}' {status_text}]\n\nTask: {task_for_run}\n\nResult:\n{content}\n\nSummarize this naturally for the user. Keep it brief (1-2 sentences). Do not mention technical details like \"subagent\" or task IDs."
            );

            let _ = bus
                .publish_inbound(InboundMessage::new(
                    "system",
                    "subagent",
                    format!("{origin_channel}:{origin_chat_id}"),
                    announce,
                ))
                .await;

            running_map.lock().await.remove(&task_id_for_cleanup);
        });

        self.running_tasks
            .lock()
            .await
            .insert(task_id.clone(), handle);
        format!(
            "Subagent [{display_label}] started (id: {task_id}). I'll notify you when it completes."
        )
    }

    pub async fn get_running_count(&self) -> usize {
        self.running_tasks.lock().await.len()
    }
}

async fn run_subagent(
    provider: Arc<dyn LLMProvider>,
    workspace: PathBuf,
    model: String,
    brave_api_key: Option<String>,
    exec_timeout_s: u64,
    restrict_to_workspace: bool,
    _task_id: String,
    task: String,
    _label: String,
) -> anyhow::Result<String> {
    let mut tools = ToolRegistry::new();
    let allowed_dir = if restrict_to_workspace {
        Some(workspace.clone())
    } else {
        None
    };
    tools.register(Arc::new(ReadFileTool::new(allowed_dir.clone())));
    tools.register(Arc::new(WriteFileTool::new(allowed_dir.clone())));
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

    let system_prompt = format!(
        "# Subagent\n\nYou are a subagent spawned by the main agent to complete a specific task.\n\n## Your Task\n{task}\n\n## Rules\n1. Stay focused - complete only the assigned task, nothing else\n2. Your final response will be reported back to the main agent\n3. Do not initiate conversations or take on side tasks\n4. Be concise but informative in your findings\n\n## What You Can Do\n- Read and write files in the workspace\n- Execute shell commands\n- Search the web and fetch web pages\n\n## What You Cannot Do\n- Send messages directly to users\n- Spawn other subagents\n\n## Workspace\n{}\n",
        workspace.display()
    );

    let mut messages = vec![
        json!({"role":"system","content":system_prompt}),
        json!({"role":"user","content":task}),
    ];

    let mut final_result = None;
    for _ in 0..15 {
        let tool_defs = tools.get_definitions();
        let response = provider
            .chat(&messages, Some(&tool_defs), Some(&model), 4096, 0.7)
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
            messages.push(json!({
                "role":"assistant",
                "content": response.content.unwrap_or_default(),
                "tool_calls": tool_call_dicts,
            }));
            for tc in response.tool_calls {
                let result = tools.execute(&tc.name, &tc.arguments).await;
                messages.push(json!({
                    "role":"tool",
                    "tool_call_id": tc.id,
                    "name": tc.name,
                    "content": result,
                }));
            }
        } else {
            final_result = response.content;
            break;
        }
    }

    Ok(final_result
        .unwrap_or_else(|| "Task completed but no final response was generated.".to_string()))
}
