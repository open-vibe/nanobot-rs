use crate::cron::{CronSchedule, CronService};
use crate::tools::base::Tool;
use anyhow::{Result, anyhow};
use async_trait::async_trait;
use serde_json::{Map, Value, json};
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct CronContext {
    channel: String,
    chat_id: String,
}

pub struct CronTool {
    cron: Arc<CronService>,
    context: Mutex<CronContext>,
}

impl CronTool {
    pub fn new(cron: Arc<CronService>) -> Self {
        Self {
            cron,
            context: Mutex::new(CronContext::default()),
        }
    }

    pub fn set_context(&self, channel: impl Into<String>, chat_id: impl Into<String>) {
        if let Ok(mut guard) = self.context.lock() {
            guard.channel = channel.into();
            guard.chat_id = chat_id.into();
        }
    }
}

#[async_trait]
impl Tool for CronTool {
    fn name(&self) -> &str {
        "cron"
    }

    fn description(&self) -> &str {
        "Schedule reminders and recurring tasks. Actions: add, list, remove."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": { "type": "string", "enum": ["add", "list", "remove"] },
                "message": { "type": "string" },
                "every_seconds": { "type": "integer" },
                "cron_expr": { "type": "string" },
                "job_id": { "type": "string" }
            },
            "required": ["action"]
        })
    }

    async fn execute(&self, params: &Map<String, Value>) -> Result<String> {
        let action = params
            .get("action")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("missing required string field: action"))?;

        match action {
            "add" => self.add_job(params).await,
            "list" => self.list_jobs().await,
            "remove" => self.remove_job(params).await,
            _ => Ok(format!("Unknown action: {action}")),
        }
    }
}

impl CronTool {
    async fn add_job(&self, params: &Map<String, Value>) -> Result<String> {
        let message = params
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        if message.is_empty() {
            return Ok("Error: message is required for add".to_string());
        }

        let (channel, chat_id) = {
            let guard = self
                .context
                .lock()
                .map_err(|_| anyhow!("failed to lock cron context"))?;
            (guard.channel.clone(), guard.chat_id.clone())
        };
        if channel.is_empty() || chat_id.is_empty() {
            return Ok("Error: no session context (channel/chat_id)".to_string());
        }

        let every_seconds = params.get("every_seconds").and_then(Value::as_i64);
        let cron_expr = params.get("cron_expr").and_then(Value::as_str);
        let schedule = if let Some(seconds) = every_seconds {
            CronSchedule {
                kind: "every".to_string(),
                every_ms: Some(seconds * 1000),
                ..Default::default()
            }
        } else if let Some(expr) = cron_expr {
            CronSchedule {
                kind: "cron".to_string(),
                expr: Some(expr.to_string()),
                ..Default::default()
            }
        } else {
            return Ok("Error: either every_seconds or cron_expr is required".to_string());
        };

        let job = self
            .cron
            .add_job(
                message.chars().take(30).collect::<String>(),
                schedule,
                message,
                true,
                Some(channel),
                Some(chat_id),
                false,
            )
            .await?;
        Ok(format!("Created job '{}' (id: {})", job.name, job.id))
    }

    async fn list_jobs(&self) -> Result<String> {
        let jobs = self.cron.list_jobs(false).await;
        if jobs.is_empty() {
            return Ok("No scheduled jobs.".to_string());
        }
        let lines = jobs
            .iter()
            .map(|j| format!("- {} (id: {}, {})", j.name, j.id, j.schedule.kind))
            .collect::<Vec<_>>();
        Ok(format!("Scheduled jobs:\n{}", lines.join("\n")))
    }

    async fn remove_job(&self, params: &Map<String, Value>) -> Result<String> {
        let Some(job_id) = params.get("job_id").and_then(Value::as_str) else {
            return Ok("Error: job_id is required for remove".to_string());
        };
        if self.cron.remove_job(job_id).await? {
            Ok(format!("Removed job {job_id}"))
        } else {
            Ok(format!("Job {job_id} not found"))
        }
    }
}
