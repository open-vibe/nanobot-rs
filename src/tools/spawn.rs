use crate::agent::subagent::SubagentManager;
use crate::tools::base::Tool;
use anyhow::{Result, anyhow};
use async_trait::async_trait;
use serde_json::{Map, Value, json};
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct SpawnContext {
    origin_channel: String,
    origin_chat_id: String,
}

pub struct SpawnTool {
    manager: Arc<SubagentManager>,
    context: Mutex<SpawnContext>,
}

impl SpawnTool {
    pub fn new(manager: Arc<SubagentManager>) -> Self {
        Self {
            manager,
            context: Mutex::new(SpawnContext {
                origin_channel: "cli".to_string(),
                origin_chat_id: "direct".to_string(),
            }),
        }
    }

    pub fn set_context(&self, channel: impl Into<String>, chat_id: impl Into<String>) {
        if let Ok(mut guard) = self.context.lock() {
            guard.origin_channel = channel.into();
            guard.origin_chat_id = chat_id.into();
        }
    }
}

#[async_trait]
impl Tool for SpawnTool {
    fn name(&self) -> &str {
        "spawn"
    }

    fn description(&self) -> &str {
        "Spawn a subagent to handle a task in the background. Use this for complex or time-consuming tasks."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "task": { "type": "string", "description": "The task for the subagent to complete" },
                "label": { "type": "string", "description": "Optional short label for the task" }
            },
            "required": ["task"]
        })
    }

    async fn execute(&self, params: &Map<String, Value>) -> Result<String> {
        let task = params
            .get("task")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("missing required string field: task"))?
            .to_string();
        let label = params
            .get("label")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);

        let (origin_channel, origin_chat_id) = {
            let guard = self
                .context
                .lock()
                .map_err(|_| anyhow!("failed to lock spawn context"))?;
            (guard.origin_channel.clone(), guard.origin_chat_id.clone())
        };

        Ok(self
            .manager
            .spawn(task, label, origin_channel, origin_chat_id)
            .await)
    }
}
