use crate::bus::{MessageBus, OutboundMessage};
use crate::channels::base::Channel;
use crate::config::DiscordConfig;
use crate::pairing::{issue_pairing, pairing_prompt};
use anyhow::Result;
use async_trait::async_trait;
use futures_util::{SinkExt, StreamExt};
use reqwest::Client;
use serde_json::{Map, Value, json};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::{Mutex, mpsc};
use tokio::task::JoinHandle;
use tokio_tungstenite::{connect_async, tungstenite::Message};

const DISCORD_API_BASE: &str = "https://discord.com/api/v10";
const MAX_ATTACHMENT_BYTES: u64 = 20 * 1024 * 1024;

pub struct DiscordChannel {
    config: DiscordConfig,
    bus: Arc<MessageBus>,
    running: AtomicBool,
    seq: Arc<Mutex<Option<i64>>>,
    http: Client,
    typing_tasks: Mutex<HashMap<String, JoinHandle<()>>>,
}

impl DiscordChannel {
    pub fn new(config: DiscordConfig, bus: Arc<MessageBus>) -> Self {
        Self {
            config,
            bus,
            running: AtomicBool::new(false),
            seq: Arc::new(Mutex::new(None)),
            http: Client::new(),
            typing_tasks: Mutex::new(HashMap::new()),
        }
    }

    async fn handle_message_create(&self, payload: &Value) -> Result<()> {
        let author = payload.get("author").cloned().unwrap_or_else(|| json!({}));
        if author.get("bot").and_then(Value::as_bool).unwrap_or(false) {
            return Ok(());
        }

        let sender_id = author
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let channel_id = payload
            .get("channel_id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        if sender_id.is_empty() || channel_id.is_empty() {
            return Ok(());
        }

        if !self.is_allowed(&sender_id) {
            if let Ok(issue) = issue_pairing(self.name(), &sender_id, &channel_id) {
                let prompt = pairing_prompt(&issue);
                let _ = self
                    .bus
                    .publish_outbound(OutboundMessage::new(
                        self.name(),
                        channel_id.clone(),
                        prompt,
                    ))
                    .await;
            }
            return Ok(());
        }

        let mut content_parts = Vec::new();
        if let Some(content) = payload.get("content").and_then(Value::as_str) {
            if !content.is_empty() {
                content_parts.push(content.to_string());
            }
        }
        let mut media_paths = Vec::new();

        let media_dir = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".nanobot")
            .join("media");
        tokio::fs::create_dir_all(&media_dir).await.ok();

        if let Some(attachments) = payload.get("attachments").and_then(Value::as_array) {
            for attachment in attachments {
                let url = attachment
                    .get("url")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let filename = attachment
                    .get("filename")
                    .and_then(Value::as_str)
                    .unwrap_or("attachment");
                let size = attachment.get("size").and_then(Value::as_u64).unwrap_or(0);
                if url.is_empty() {
                    continue;
                }
                if size > MAX_ATTACHMENT_BYTES {
                    content_parts.push(format!("[attachment: {filename} - too large]"));
                    continue;
                }
                let id = attachment
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or("file");
                let safe_name = filename.replace('/', "_");
                let file_path = media_dir.join(format!("{id}_{safe_name}"));
                match self.http.get(url).send().await {
                    Ok(resp) if resp.status().is_success() => {
                        let bytes = resp.bytes().await.unwrap_or_default();
                        tokio::fs::write(&file_path, &bytes).await.ok();
                        media_paths.push(file_path.display().to_string());
                        content_parts.push(format!("[attachment: {}]", file_path.display()));
                    }
                    _ => {
                        content_parts.push(format!("[attachment: {filename} - download failed]"));
                    }
                }
            }
        }

        let mut metadata = Map::new();
        metadata.insert(
            "message_id".to_string(),
            payload.get("id").cloned().unwrap_or(Value::Null),
        );
        metadata.insert(
            "guild_id".to_string(),
            payload.get("guild_id").cloned().unwrap_or(Value::Null),
        );
        metadata.insert(
            "reply_to".to_string(),
            payload
                .get("referenced_message")
                .and_then(|v| v.get("id"))
                .cloned()
                .unwrap_or(Value::Null),
        );

        self.handle_message(
            sender_id,
            channel_id.clone(),
            if content_parts.is_empty() {
                "[empty message]".to_string()
            } else {
                content_parts.join("\n")
            },
            media_paths,
            metadata,
        )
        .await?;

        self.start_typing(channel_id).await;
        Ok(())
    }

    async fn start_typing(&self, channel_id: String) {
        self.stop_typing(&channel_id).await;
        let channel_for_task = channel_id.clone();
        let token = self.config.token.clone();
        let http = self.http.clone();
        let task = tokio::spawn(async move {
            let url = format!("{DISCORD_API_BASE}/channels/{channel_for_task}/typing");
            loop {
                let _ = http
                    .post(&url)
                    .header("Authorization", format!("Bot {token}"))
                    .send()
                    .await;
                tokio::time::sleep(std::time::Duration::from_secs(8)).await;
            }
        });
        self.typing_tasks.lock().await.insert(channel_id, task);
    }

    async fn stop_typing(&self, channel_id: &str) {
        if let Some(task) = self.typing_tasks.lock().await.remove(channel_id) {
            task.abort();
        }
    }
}

#[async_trait]
impl Channel for DiscordChannel {
    fn name(&self) -> &str {
        "discord"
    }

    fn is_running(&self) -> bool {
        self.running.load(Ordering::Relaxed)
    }

    fn allow_from(&self) -> &[String] {
        &self.config.allow_from
    }

    fn bus(&self) -> Arc<MessageBus> {
        self.bus.clone()
    }

    async fn start(&self) -> Result<()> {
        if self.config.token.is_empty() {
            return Ok(());
        }
        self.running.store(true, Ordering::Relaxed);

        while self.running.load(Ordering::Relaxed) {
            let connection = connect_async(&self.config.gateway_url).await;
            let Ok((ws, _)) = connection else {
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                continue;
            };

            let (mut write, mut read) = ws.split();
            let (tx, mut rx) = mpsc::unbounded_channel::<Message>();
            let writer_task = tokio::spawn(async move {
                while let Some(msg) = rx.recv().await {
                    if write.send(msg).await.is_err() {
                        break;
                    }
                }
            });

            let mut heartbeat_task: Option<tokio::task::JoinHandle<()>> = None;
            while self.running.load(Ordering::Relaxed) {
                let Some(msg) = read.next().await else {
                    break;
                };
                let Ok(msg) = msg else {
                    break;
                };
                if !msg.is_text() {
                    continue;
                }
                let Ok(payload) =
                    serde_json::from_str::<Value>(&msg.into_text().unwrap_or_default())
                else {
                    continue;
                };

                if let Some(seq) = payload.get("s").and_then(Value::as_i64) {
                    *self.seq.lock().await = Some(seq);
                }

                let op = payload.get("op").and_then(Value::as_i64).unwrap_or(-1);
                let event_type = payload.get("t").and_then(Value::as_str).unwrap_or_default();

                match op {
                    10 => {
                        let interval_ms = payload
                            .get("d")
                            .and_then(|v| v.get("heartbeat_interval"))
                            .and_then(Value::as_u64)
                            .unwrap_or(45_000);

                        let identify = json!({
                            "op": 2,
                            "d": {
                                "token": self.config.token,
                                "intents": self.config.intents,
                                "properties": { "os": "nanobot-rs", "browser": "nanobot-rs", "device": "nanobot-rs" }
                            }
                        });
                        let _ = tx.send(Message::Text(identify.to_string()));

                        if let Some(task) = heartbeat_task.take() {
                            task.abort();
                        }
                        let seq = self.seq.clone();
                        let tx_for_heartbeat = tx.clone();
                        heartbeat_task = Some(tokio::spawn(async move {
                            let mut interval = tokio::time::interval(
                                std::time::Duration::from_millis(interval_ms),
                            );
                            loop {
                                interval.tick().await;
                                let seq_val = *seq.lock().await;
                                let heartbeat = json!({"op": 1, "d": seq_val});
                                if tx_for_heartbeat
                                    .send(Message::Text(heartbeat.to_string()))
                                    .is_err()
                                {
                                    break;
                                }
                            }
                        }));
                    }
                    0 if event_type == "MESSAGE_CREATE" => {
                        if let Some(data) = payload.get("d") {
                            let _ = self.handle_message_create(data).await;
                        }
                    }
                    7 | 9 => {
                        break;
                    }
                    _ => {}
                }
            }

            if let Some(task) = heartbeat_task.take() {
                task.abort();
            }
            writer_task.abort();
            let mut typing = self.typing_tasks.lock().await;
            for (_, task) in typing.drain() {
                task.abort();
            }

            if self.running.load(Ordering::Relaxed) {
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            }
        }

        Ok(())
    }

    async fn stop(&self) -> Result<()> {
        self.running.store(false, Ordering::Relaxed);
        let mut typing = self.typing_tasks.lock().await;
        for (_, task) in typing.drain() {
            task.abort();
        }
        Ok(())
    }

    async fn send(&self, msg: &OutboundMessage) -> Result<()> {
        let url = format!("{DISCORD_API_BASE}/channels/{}/messages", msg.chat_id);
        let mut payload = json!({ "content": msg.content });
        if let Some(reply_to) = &msg.reply_to {
            payload["message_reference"] = json!({ "message_id": reply_to });
            payload["allowed_mentions"] = json!({ "replied_user": false });
        }

        let headers = [("Authorization", format!("Bot {}", self.config.token))];
        for _ in 0..3 {
            let response = self
                .http
                .post(&url)
                .headers(headers.iter().fold(
                    reqwest::header::HeaderMap::new(),
                    |mut map, (k, v)| {
                        map.insert(
                            reqwest::header::HeaderName::from_bytes(k.as_bytes()).unwrap(),
                            reqwest::header::HeaderValue::from_str(v).unwrap(),
                        );
                        map
                    },
                ))
                .json(&payload)
                .send()
                .await?;

            if response.status().as_u16() == 429 {
                let data: Value = response.json().await.unwrap_or_else(|_| json!({}));
                let retry_after = data
                    .get("retry_after")
                    .and_then(Value::as_f64)
                    .unwrap_or(1.0);
                tokio::time::sleep(std::time::Duration::from_secs_f64(retry_after)).await;
                continue;
            }
            if response.status().is_success() {
                self.stop_typing(&msg.chat_id).await;
                return Ok(());
            }
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
        self.stop_typing(&msg.chat_id).await;
        Ok(())
    }
}
