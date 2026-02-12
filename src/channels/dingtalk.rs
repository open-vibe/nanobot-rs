use crate::bus::{MessageBus, OutboundMessage};
use crate::channels::base::Channel;
use crate::config::DingTalkConfig;
use anyhow::{Result, anyhow};
use async_trait::async_trait;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

#[cfg(feature = "dingtalk-stream")]
use dingtalk_stream_sdk_rust::{
    Client as DingTalkStreamClient, TOPIC_ROBOT,
    down::{MsgContent, RichText, RobotRecvMessage},
    up::{MessageTemplate, RobotSendMessage},
};
#[cfg(feature = "dingtalk-stream")]
use serde_json::Value;
#[cfg(feature = "dingtalk-stream")]
use tokio::sync::Mutex;
#[cfg(feature = "dingtalk-stream")]
use {crate::bus::InboundMessage, crate::channels::base::is_allowed_sender};

pub struct DingTalkChannel {
    config: DingTalkConfig,
    bus: Arc<MessageBus>,
    running: AtomicBool,
    #[cfg(feature = "dingtalk-stream")]
    client: Mutex<Option<Arc<DingTalkStreamClient>>>,
}

impl DingTalkChannel {
    pub fn new(config: DingTalkConfig, bus: Arc<MessageBus>) -> Self {
        Self {
            config,
            bus,
            running: AtomicBool::new(false),
            #[cfg(feature = "dingtalk-stream")]
            client: Mutex::new(None),
        }
    }
}

#[async_trait]
impl Channel for DingTalkChannel {
    fn name(&self) -> &str {
        "dingtalk"
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

        #[cfg(not(feature = "dingtalk-stream"))]
        {
            eprintln!(
                "DingTalk stream support is disabled. Rebuild with --features dingtalk-stream."
            );
            self.running.store(false, Ordering::Relaxed);
            Ok(())
        }

        #[cfg(feature = "dingtalk-stream")]
        {
            if self.config.client_id.is_empty() || self.config.client_secret.is_empty() {
                self.running.store(false, Ordering::Relaxed);
                return Ok(());
            }

            let bus = self.bus.clone();
            let allow_from = self.config.allow_from.clone();
            let client =
                DingTalkStreamClient::new(&self.config.client_id, &self.config.client_secret)?;

            let client = client.register_callback_listener(TOPIC_ROBOT, move |_client, msg| {
                let bus = bus.clone();
                let allow_from = allow_from.clone();
                async move {
                    let RobotRecvMessage {
                        content,
                        sender_staff_id,
                        sender_id,
                        sender_nick,
                        conversation_type,
                        ..
                    } = msg;

                    let mut message_text = match content {
                        MsgContent::Text { content } => content,
                        MsgContent::File { .. } => "[file]".to_string(),
                        MsgContent::Picture { .. } => "[image]".to_string(),
                        MsgContent::Audio { recognition, .. } => {
                            if recognition.trim().is_empty() {
                                "[audio]".to_string()
                            } else {
                                format!("[audio: {recognition}]")
                            }
                        }
                        MsgContent::Video { .. } => "[video]".to_string(),
                        MsgContent::RichText { rich_text } => {
                            let text = rich_text
                                .into_iter()
                                .filter_map(|part| match part {
                                    RichText::Text { text } => Some(text),
                                    _ => None,
                                })
                                .collect::<Vec<_>>()
                                .join(" ");
                            if text.trim().is_empty() {
                                "[rich_text]".to_string()
                            } else {
                                text
                            }
                        }
                        MsgContent::UnknownMsgType { unknown_msg_type } => {
                            format!("[{unknown_msg_type}]")
                        }
                    };
                    message_text = message_text.trim().to_string();
                    if message_text.is_empty() {
                        return Ok::<(), anyhow::Error>(());
                    }

                    let sender = if sender_staff_id.is_empty() {
                        sender_id
                    } else {
                        sender_staff_id
                    };
                    if !is_allowed_sender(&sender, &allow_from) {
                        return Ok(());
                    }

                    let mut inbound =
                        InboundMessage::new("dingtalk", sender.clone(), sender, message_text);
                    inbound
                        .metadata
                        .insert("sender_name".to_string(), Value::String(sender_nick));
                    inbound.metadata.insert(
                        "conversation_type".to_string(),
                        Value::String(conversation_type),
                    );
                    inbound.metadata.insert(
                        "platform".to_string(),
                        Value::String("dingtalk".to_string()),
                    );
                    let _ = bus.publish_inbound(inbound).await;
                    Ok::<(), anyhow::Error>(())
                }
            });

            *self.client.lock().await = Some(client.clone());
            while self.running.load(Ordering::Relaxed) {
                let connect_result = client.clone().connect().await;
                if !self.running.load(Ordering::Relaxed) {
                    break;
                }
                if let Err(err) = connect_result {
                    eprintln!("DingTalk stream error: {err}");
                } else {
                    eprintln!("DingTalk stream disconnected unexpectedly.");
                }
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            }
            self.running.store(false, Ordering::Relaxed);
            Ok(())
        }
    }

    async fn stop(&self) -> Result<()> {
        self.running.store(false, Ordering::Relaxed);
        #[cfg(feature = "dingtalk-stream")]
        if let Some(client) = self.client.lock().await.take() {
            client.exit();
        }
        Ok(())
    }

    async fn send(&self, msg: &OutboundMessage) -> Result<()> {
        #[cfg(not(feature = "dingtalk-stream"))]
        {
            let _ = msg;
            return Err(anyhow!(
                "DingTalk stream support is disabled; build with --features dingtalk-stream"
            ));
        }

        #[cfg(feature = "dingtalk-stream")]
        {
            let client = self
                .client
                .lock()
                .await
                .clone()
                .ok_or_else(|| anyhow!("DingTalk client not connected"))?;
            let message = MessageTemplate::SampleMarkdown {
                title: "Nanobot Reply".to_string(),
                text: msg.content.clone(),
            };
            RobotSendMessage::single(client, msg.chat_id.clone(), message)?
                .send()
                .await?;
            Ok(())
        }
    }
}
