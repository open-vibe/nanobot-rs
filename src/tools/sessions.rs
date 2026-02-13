use crate::bus::OutboundMessage;
use crate::session::SessionManager;
use crate::tools::base::Tool;
use crate::utils::parse_session_key;
use anyhow::{Result, anyhow};
use async_trait::async_trait;
use serde_json::{Map, Value, json};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

pub struct SessionsListTool {
    sessions: Arc<SessionManager>,
}

impl SessionsListTool {
    pub fn new(sessions: Arc<SessionManager>) -> Self {
        Self { sessions }
    }
}

#[async_trait]
impl Tool for SessionsListTool {
    fn name(&self) -> &str {
        "sessions_list"
    }

    fn description(&self) -> &str {
        "List available session keys."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {}
        })
    }

    async fn execute(&self, _params: &Map<String, Value>) -> Result<String> {
        let keys = self.sessions.list_session_keys()?;
        if keys.is_empty() {
            return Ok("No sessions found.".to_string());
        }
        Ok(serde_json::to_string(&json!({ "sessions": keys }))?)
    }
}

pub struct SessionsHistoryTool {
    sessions: Arc<SessionManager>,
}

impl SessionsHistoryTool {
    pub fn new(sessions: Arc<SessionManager>) -> Self {
        Self { sessions }
    }
}

#[async_trait]
impl Tool for SessionsHistoryTool {
    fn name(&self) -> &str {
        "sessions_history"
    }

    fn description(&self) -> &str {
        "Read message history from a given session."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "session": { "type": "string", "description": "Session key like telegram:123456" },
                "limit": { "type": "integer", "minimum": 1, "maximum": 200, "description": "Max messages to return, default 20" }
            },
            "required": ["session"]
        })
    }

    async fn execute(&self, params: &Map<String, Value>) -> Result<String> {
        let session_key = params
            .get("session")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("missing required string field: session"))?;
        let limit = params
            .get("limit")
            .and_then(Value::as_i64)
            .unwrap_or(20)
            .max(1) as usize;
        let session = self.sessions.load_session(session_key)?;
        let total = session.messages.len();
        let start = total.saturating_sub(limit);
        let messages = session.messages[start..].to_vec();
        Ok(serde_json::to_string(&json!({
            "session": session.key,
            "total": total,
            "messages": messages
        }))?)
    }
}

#[derive(Default)]
struct SessionsSendContext {
    origin_channel: String,
    origin_chat_id: String,
}

pub struct SessionsSendTool {
    sender: mpsc::Sender<OutboundMessage>,
    context: Mutex<SessionsSendContext>,
}

impl SessionsSendTool {
    pub fn new(sender: mpsc::Sender<OutboundMessage>) -> Self {
        Self {
            sender,
            context: Mutex::new(SessionsSendContext::default()),
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
impl Tool for SessionsSendTool {
    fn name(&self) -> &str {
        "sessions_send"
    }

    fn description(&self) -> &str {
        "Send a plain message to another existing session (channel:chat_id)."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "session": { "type": "string", "description": "Target session key, e.g. telegram:123456" },
                "content": { "type": "string", "description": "Message content" }
            },
            "required": ["session", "content"]
        })
    }

    async fn execute(&self, params: &Map<String, Value>) -> Result<String> {
        let session = params
            .get("session")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("missing required string field: session"))?;
        let content = params
            .get("content")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("missing required string field: content"))?;

        let (channel, chat_id) = parse_session_key(session)?;
        let mut outbound = OutboundMessage::new(channel, chat_id, content);
        if let Ok(ctx) = self.context.lock() {
            outbound.metadata.insert(
                "forwarded_from".to_string(),
                Value::String(format!("{}:{}", ctx.origin_channel, ctx.origin_chat_id)),
            );
        }
        self.sender
            .send(outbound)
            .await
            .map_err(|err| anyhow!("failed to send session message: {err}"))?;
        Ok(format!("Sent message to session {session}"))
    }
}
