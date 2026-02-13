use crate::bus::OutboundMessage;
use crate::bus::{InboundMessage, MessageBus};
use crate::pairing::{issue_pairing, pairing_prompt};
use anyhow::Result;
use async_trait::async_trait;
use serde_json::{Map, Value};
use std::sync::Arc;

#[async_trait]
pub trait Channel: Send + Sync {
    fn name(&self) -> &str;
    fn is_running(&self) -> bool;
    fn allow_from(&self) -> &[String];
    fn bus(&self) -> Arc<MessageBus>;

    async fn start(&self) -> Result<()>;
    async fn stop(&self) -> Result<()>;
    async fn send(&self, msg: &crate::bus::OutboundMessage) -> Result<()>;

    fn is_allowed(&self, sender_id: &str) -> bool {
        is_allowed_sender(sender_id, self.allow_from())
    }

    async fn handle_message(
        &self,
        sender_id: String,
        chat_id: String,
        content: String,
        media: Vec<String>,
        metadata: Map<String, Value>,
    ) -> Result<()> {
        if !self.is_allowed(&sender_id) {
            if let Ok(issue) = issue_pairing(self.name(), &sender_id, &chat_id) {
                let prompt = pairing_prompt(&issue);
                let _ = self
                    .bus()
                    .publish_outbound(OutboundMessage::new(self.name(), chat_id.clone(), prompt))
                    .await;
            }
            return Ok(());
        }
        let mut msg = InboundMessage::new(self.name(), sender_id, chat_id, content);
        msg.media = media;
        msg.metadata = metadata;
        self.bus().publish_inbound(msg).await?;
        Ok(())
    }
}

pub fn is_allowed_sender(sender_id: &str, allow_from: &[String]) -> bool {
    if allow_from.is_empty() {
        return true;
    }
    if allow_from.iter().any(|allowed| allowed == sender_id) {
        return true;
    }
    if sender_id.contains('|') {
        for part in sender_id.split('|') {
            if allow_from.iter().any(|allowed| allowed == part) {
                return true;
            }
        }
    }
    false
}
