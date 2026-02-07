use crate::bus::OutboundMessage;
use crate::tools::base::Tool;
use anyhow::{Result, anyhow};
use async_trait::async_trait;
use serde_json::{Map, Value, json};
use std::sync::Mutex;
use tokio::sync::mpsc;

#[derive(Default)]
struct MessageContext {
    channel: String,
    chat_id: String,
}

pub struct MessageTool {
    sender: mpsc::Sender<OutboundMessage>,
    context: Mutex<MessageContext>,
}

impl MessageTool {
    pub fn new(sender: mpsc::Sender<OutboundMessage>) -> Self {
        Self {
            sender,
            context: Mutex::new(MessageContext::default()),
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
impl Tool for MessageTool {
    fn name(&self) -> &str {
        "message"
    }

    fn description(&self) -> &str {
        "Send a message to the user. Use this when you need to communicate a progress update to a chat channel."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "content": { "type": "string", "description": "The message content to send" },
                "channel": { "type": "string", "description": "Optional target channel" },
                "chat_id": { "type": "string", "description": "Optional target chat/user ID" }
            },
            "required": ["content"]
        })
    }

    async fn execute(&self, params: &Map<String, Value>) -> Result<String> {
        let content = params
            .get("content")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("missing required string field: content"))?;

        let explicit_channel = params
            .get("channel")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);
        let explicit_chat_id = params
            .get("chat_id")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);

        let (channel, chat_id) =
            if let (Some(channel), Some(chat_id)) = (explicit_channel, explicit_chat_id) {
                (channel, chat_id)
            } else {
                let guard = self
                    .context
                    .lock()
                    .map_err(|_| anyhow!("failed to lock message tool context"))?;
                (guard.channel.clone(), guard.chat_id.clone())
            };

        if channel.is_empty() || chat_id.is_empty() {
            return Ok("Error: No target channel/chat specified".to_string());
        }

        let msg = OutboundMessage::new(channel.clone(), chat_id.clone(), content);
        self.sender
            .send(msg)
            .await
            .map_err(|err| anyhow!("Error sending message: {err}"))?;

        Ok(format!("Message sent to {channel}:{chat_id}"))
    }
}
