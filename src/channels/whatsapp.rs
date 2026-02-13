use crate::bus::{MessageBus, OutboundMessage};
use crate::channels::base::Channel;
use crate::config::WhatsAppConfig;
use anyhow::{Result, anyhow};
use async_trait::async_trait;
use futures_util::{SinkExt, StreamExt};
use serde_json::{Map, Value, json};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::{Mutex, mpsc};
use tokio_tungstenite::{connect_async, tungstenite::Message};

pub struct WhatsAppChannel {
    config: WhatsAppConfig,
    bus: Arc<MessageBus>,
    running: AtomicBool,
    connected: AtomicBool,
    outbound_tx: Mutex<Option<mpsc::UnboundedSender<String>>>,
}

impl WhatsAppChannel {
    pub fn new(config: WhatsAppConfig, bus: Arc<MessageBus>) -> Self {
        Self {
            config,
            bus,
            running: AtomicBool::new(false),
            connected: AtomicBool::new(false),
            outbound_tx: Mutex::new(None),
        }
    }
}

#[async_trait]
impl Channel for WhatsAppChannel {
    fn name(&self) -> &str {
        "whatsapp"
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
        self.running.store(true, Ordering::Relaxed);
        while self.running.load(Ordering::Relaxed) {
            let connection = connect_async(&self.config.bridge_url).await;
            let Ok((ws, _)) = connection else {
                self.connected.store(false, Ordering::Relaxed);
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                continue;
            };
            let (mut write, mut read) = ws.split();
            let (tx, mut rx) = mpsc::unbounded_channel::<String>();
            *self.outbound_tx.lock().await = Some(tx);

            let writer = tokio::spawn(async move {
                while let Some(payload) = rx.recv().await {
                    if write.send(Message::Text(payload)).await.is_err() {
                        break;
                    }
                }
            });

            if !self.config.bridge_token.is_empty() {
                let auth_payload = json!({
                    "type": "auth",
                    "token": self.config.bridge_token
                })
                .to_string();
                if let Some(tx) = self.outbound_tx.lock().await.clone() {
                    let _ = tx.send(auth_payload);
                }
            }

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
                let Ok(text) = msg.into_text() else {
                    continue;
                };
                let Ok(data) = serde_json::from_str::<Value>(&text) else {
                    continue;
                };
                let msg_type = data.get("type").and_then(Value::as_str).unwrap_or_default();
                match msg_type {
                    "message" => {
                        let pn = data
                            .get("pn")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string();
                        let sender = data
                            .get("sender")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string();
                        let mut content = data
                            .get("content")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string();
                        let user_id = if pn.is_empty() { &sender } else { &pn };
                        let sender_id = user_id.split('@').next().unwrap_or(user_id).to_string();
                        if content == "[Voice Message]" {
                            content =
                                "[Voice Message: Transcription not available for WhatsApp yet]"
                                    .to_string();
                        }
                        let mut metadata = Map::new();
                        metadata.insert(
                            "message_id".to_string(),
                            data.get("id").cloned().unwrap_or(Value::Null),
                        );
                        metadata.insert(
                            "timestamp".to_string(),
                            data.get("timestamp").cloned().unwrap_or(Value::Null),
                        );
                        metadata.insert("pn".to_string(), Value::String(pn));
                        metadata.insert(
                            "is_group".to_string(),
                            data.get("isGroup").cloned().unwrap_or(Value::Bool(false)),
                        );
                        self.handle_message(sender_id, sender, content, Vec::new(), metadata)
                            .await?;
                    }
                    "status" => {
                        let status = data
                            .get("status")
                            .and_then(Value::as_str)
                            .unwrap_or_default();
                        let is_connected = status == "connected";
                        self.connected.store(is_connected, Ordering::Relaxed);
                        if !status.is_empty() {
                            eprintln!("WhatsApp status: {status}");
                        }
                    }
                    "qr" => {
                        eprintln!("WhatsApp QR received. Scan the QR code in bridge terminal.");
                    }
                    "error" => {
                        let err = data
                            .get("error")
                            .and_then(Value::as_str)
                            .unwrap_or("unknown bridge error");
                        eprintln!("WhatsApp bridge error: {err}");
                    }
                    "sent" => {}
                    _ => {}
                }
            }

            writer.abort();
            self.connected.store(false, Ordering::Relaxed);
            *self.outbound_tx.lock().await = None;
            if self.running.load(Ordering::Relaxed) {
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            }
        }
        Ok(())
    }

    async fn stop(&self) -> Result<()> {
        self.running.store(false, Ordering::Relaxed);
        self.connected.store(false, Ordering::Relaxed);
        *self.outbound_tx.lock().await = None;
        Ok(())
    }

    async fn send(&self, msg: &OutboundMessage) -> Result<()> {
        if !self.connected.load(Ordering::Relaxed) {
            return Err(anyhow!("WhatsApp bridge not connected"));
        }
        let payload = json!({
            "type": "send",
            "to": msg.chat_id,
            "text": msg.content
        })
        .to_string();
        let tx = self
            .outbound_tx
            .lock()
            .await
            .clone()
            .ok_or_else(|| anyhow!("WhatsApp bridge not connected"))?;
        tx.send(payload)
            .map_err(|err| anyhow!("failed to send bridge payload: {err}"))?;
        Ok(())
    }
}
