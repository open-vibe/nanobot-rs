use crate::bus::{MessageBus, OutboundMessage};
use crate::channels::base::Channel;
use crate::config::SlackConfig;
use crate::pairing::{issue_pairing, pairing_prompt};
use anyhow::{Result, anyhow};
use async_trait::async_trait;
use futures_util::{SinkExt, StreamExt};
use serde_json::{Map, Value, json};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::Mutex;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message as WsMessage;

pub struct SlackChannel {
    config: SlackConfig,
    bus: Arc<MessageBus>,
    running: AtomicBool,
    client: reqwest::Client,
    bot_user_id: Mutex<Option<String>>,
}

impl SlackChannel {
    pub fn new(config: SlackConfig, bus: Arc<MessageBus>) -> Self {
        Self {
            config,
            bus,
            running: AtomicBool::new(false),
            client: reqwest::Client::new(),
            bot_user_id: Mutex::new(None),
        }
    }

    async fn auth_test_user_id(&self) -> Option<String> {
        let response = self
            .client
            .post("https://slack.com/api/auth.test")
            .bearer_auth(&self.config.bot_token)
            .send()
            .await
            .ok()?;
        let payload: Value = response.json().await.ok()?;
        if !payload.get("ok").and_then(Value::as_bool).unwrap_or(false) {
            return None;
        }
        payload
            .get("user_id")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
    }

    async fn open_socket_url(&self) -> Option<String> {
        let response = self
            .client
            .post("https://slack.com/api/apps.connections.open")
            .bearer_auth(&self.config.app_token)
            .send()
            .await
            .ok()?;
        let payload: Value = response.json().await.ok()?;
        if !payload.get("ok").and_then(Value::as_bool).unwrap_or(false) {
            return None;
        }
        payload
            .get("url")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
    }

    async fn post_slack_api(&self, path: &str, body: Value) -> Result<Value> {
        let url = format!("https://slack.com/api/{path}");
        let response = self
            .client
            .post(url)
            .bearer_auth(&self.config.bot_token)
            .json(&body)
            .send()
            .await?;
        let payload: Value = response.json().await?;
        if !payload.get("ok").and_then(Value::as_bool).unwrap_or(false) {
            let err = payload
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            return Err(anyhow!("slack api {path} failed: {err}"));
        }
        Ok(payload)
    }

    fn is_allowed(&self, sender_id: &str, chat_id: &str, channel_type: &str) -> bool {
        if channel_type == "im" {
            if !self.config.dm.enabled {
                return false;
            }
            if self.config.dm.policy == "allowlist" {
                return self.config.dm.allow_from.iter().any(|v| v == sender_id);
            }
            return true;
        }

        if self.config.group_policy == "allowlist" {
            return self.config.group_allow_from.iter().any(|v| v == chat_id);
        }
        true
    }

    async fn should_respond_in_channel(&self, event_type: &str, text: &str, chat_id: &str) -> bool {
        match self.config.group_policy.as_str() {
            "open" => true,
            "mention" => {
                if event_type == "app_mention" {
                    true
                } else if let Some(bot_id) = self.bot_user_id.lock().await.clone() {
                    text.contains(&format!("<@{bot_id}>"))
                } else {
                    false
                }
            }
            "allowlist" => self.config.group_allow_from.iter().any(|v| v == chat_id),
            _ => false,
        }
    }

    async fn strip_bot_mention(&self, text: &str) -> String {
        if let Some(bot_id) = self.bot_user_id.lock().await.clone() {
            text.replace(&format!("<@{bot_id}>"), "").trim().to_string()
        } else {
            text.trim().to_string()
        }
    }

    async fn handle_event_payload(&self, payload: &Value) -> Result<()> {
        let event = payload.get("event").cloned().unwrap_or_else(|| json!({}));
        let event_type = event.get("type").and_then(Value::as_str).unwrap_or("");
        if event_type != "message" && event_type != "app_mention" {
            return Ok(());
        }

        if event.get("subtype").is_some() {
            return Ok(());
        }

        let sender_id = event
            .get("user")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let chat_id = event
            .get("channel")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        if sender_id.is_empty() || chat_id.is_empty() {
            return Ok(());
        }

        if let Some(bot_id) = self.bot_user_id.lock().await.clone() {
            if sender_id == bot_id {
                return Ok(());
            }
            let text = event.get("text").and_then(Value::as_str).unwrap_or("");
            if event_type == "message" && text.contains(&format!("<@{bot_id}>")) {
                // Slack can emit both message and app_mention for same mention message.
                return Ok(());
            }
        }

        let channel_type = event
            .get("channel_type")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        if !self.is_allowed(&sender_id, &chat_id, &channel_type) {
            if let Ok(issue) = issue_pairing(self.name(), &sender_id, &chat_id) {
                let prompt = pairing_prompt(&issue);
                let _ = self
                    .post_slack_api(
                        "chat.postMessage",
                        json!({
                            "channel": chat_id,
                            "text": prompt,
                        }),
                    )
                    .await;
            }
            return Ok(());
        }

        let raw_text = event.get("text").and_then(Value::as_str).unwrap_or("");
        if channel_type != "im"
            && !self
                .should_respond_in_channel(event_type, raw_text, &chat_id)
                .await
        {
            return Ok(());
        }

        let text = self.strip_bot_mention(raw_text).await;
        if text.is_empty() {
            return Ok(());
        }

        let thread_ts = event
            .get("thread_ts")
            .and_then(Value::as_str)
            .or_else(|| event.get("ts").and_then(Value::as_str))
            .unwrap_or("")
            .to_string();

        if let Some(ts) = event.get("ts").and_then(Value::as_str) {
            let _ = self
                .post_slack_api(
                    "reactions.add",
                    json!({
                        "channel": chat_id,
                        "name": "eyes",
                        "timestamp": ts,
                    }),
                )
                .await;
        }

        let mut slack_meta = Map::new();
        if !thread_ts.is_empty() {
            slack_meta.insert("thread_ts".to_string(), Value::String(thread_ts));
        }
        if !channel_type.is_empty() {
            slack_meta.insert("channel_type".to_string(), Value::String(channel_type));
        }
        let mut metadata = Map::new();
        metadata.insert("slack".to_string(), Value::Object(slack_meta));

        self.handle_message(sender_id, chat_id, text, Vec::new(), metadata)
            .await
    }
}

#[async_trait]
impl Channel for SlackChannel {
    fn name(&self) -> &str {
        "slack"
    }

    fn is_running(&self) -> bool {
        self.running.load(Ordering::Relaxed)
    }

    fn allow_from(&self) -> &[String] {
        &self.config.dm.allow_from
    }

    fn bus(&self) -> Arc<MessageBus> {
        self.bus.clone()
    }

    async fn start(&self) -> Result<()> {
        if self.config.bot_token.is_empty() || self.config.app_token.is_empty() {
            eprintln!("Slack bot/app token not configured");
            return Ok(());
        }
        if self.config.mode != "socket" {
            eprintln!("Unsupported Slack mode: {}", self.config.mode);
            return Ok(());
        }

        self.running.store(true, Ordering::Relaxed);
        let bot_user_id = self.auth_test_user_id().await;
        *self.bot_user_id.lock().await = bot_user_id;

        while self.running.load(Ordering::Relaxed) {
            let Some(socket_url) = self.open_socket_url().await else {
                tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                continue;
            };

            let Ok((ws_stream, _)) = connect_async(socket_url).await else {
                tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                continue;
            };

            let (mut write, mut read) = ws_stream.split();
            while self.running.load(Ordering::Relaxed) {
                let Some(next_message) = read.next().await else {
                    break;
                };
                let Ok(next_message) = next_message else {
                    break;
                };

                match next_message {
                    WsMessage::Text(text) => {
                        let payload: Value =
                            serde_json::from_str(&text).unwrap_or_else(|_| json!({}));
                        if let Some(envelope_id) =
                            payload.get("envelope_id").and_then(Value::as_str)
                        {
                            let ack = json!({ "envelope_id": envelope_id }).to_string();
                            let _ = write.send(WsMessage::Text(ack.into())).await;
                        }
                        if payload.get("type").and_then(Value::as_str) != Some("events_api") {
                            continue;
                        }
                        let _ = self.handle_event_payload(&payload).await;
                    }
                    WsMessage::Ping(data) => {
                        let _ = write.send(WsMessage::Pong(data)).await;
                    }
                    WsMessage::Close(_) => break,
                    _ => {}
                }
            }
        }

        self.running.store(false, Ordering::Relaxed);
        Ok(())
    }

    async fn stop(&self) -> Result<()> {
        self.running.store(false, Ordering::Relaxed);
        Ok(())
    }

    async fn send(&self, msg: &OutboundMessage) -> Result<()> {
        let slack_meta = msg.metadata.get("slack").and_then(Value::as_object);
        let thread_ts = slack_meta
            .and_then(|m| m.get("thread_ts"))
            .and_then(Value::as_str);
        let channel_type = slack_meta
            .and_then(|m| m.get("channel_type"))
            .and_then(Value::as_str)
            .unwrap_or("");
        let use_thread = thread_ts.is_some() && channel_type != "im";

        let mut body = json!({
            "channel": msg.chat_id,
            "text": msg.content,
        });
        if use_thread {
            body["thread_ts"] = Value::String(thread_ts.unwrap_or_default().to_string());
        }

        let _ = self.post_slack_api("chat.postMessage", body).await?;
        Ok(())
    }
}
