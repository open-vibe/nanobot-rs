use crate::bus::MessageBus;
use crate::channels::base::Channel;
use crate::channels::dingtalk::DingTalkChannel;
use crate::channels::discord::DiscordChannel;
use crate::channels::email::EmailChannel;
use crate::channels::feishu::FeishuChannel;
use crate::channels::qq::QQChannel;
use crate::channels::slack::SlackChannel;
use crate::channels::telegram::TelegramChannel;
use crate::channels::whatsapp::WhatsAppChannel;
use crate::config::Config;
use crate::session::SessionManager;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::Mutex;

pub struct ChannelManager {
    bus: Arc<MessageBus>,
    channels: HashMap<String, Arc<dyn Channel>>,
    running: Arc<AtomicBool>,
    dispatch_task: Mutex<Option<tokio::task::JoinHandle<()>>>,
    channel_tasks: Mutex<Vec<tokio::task::JoinHandle<()>>>,
}

impl ChannelManager {
    pub fn new(
        config: &Config,
        bus: Arc<MessageBus>,
        session_manager: Option<Arc<SessionManager>>,
    ) -> Self {
        let mut channels: HashMap<String, Arc<dyn Channel>> = HashMap::new();

        if config.channels.telegram.enabled {
            channels.insert(
                "telegram".to_string(),
                Arc::new(TelegramChannel::new(
                    config.channels.telegram.clone(),
                    bus.clone(),
                    config.providers.groq.api_key.clone(),
                    session_manager.clone(),
                )),
            );
        }
        if config.channels.whatsapp.enabled {
            channels.insert(
                "whatsapp".to_string(),
                Arc::new(WhatsAppChannel::new(
                    config.channels.whatsapp.clone(),
                    bus.clone(),
                )),
            );
        }
        if config.channels.discord.enabled {
            channels.insert(
                "discord".to_string(),
                Arc::new(DiscordChannel::new(
                    config.channels.discord.clone(),
                    bus.clone(),
                )),
            );
        }
        if config.channels.feishu.enabled {
            channels.insert(
                "feishu".to_string(),
                Arc::new(FeishuChannel::new(
                    config.channels.feishu.clone(),
                    bus.clone(),
                )),
            );
        }
        if config.channels.dingtalk.enabled {
            channels.insert(
                "dingtalk".to_string(),
                Arc::new(DingTalkChannel::new(
                    config.channels.dingtalk.clone(),
                    bus.clone(),
                )),
            );
        }
        if config.channels.email.enabled {
            channels.insert(
                "email".to_string(),
                Arc::new(EmailChannel::new(
                    config.channels.email.clone(),
                    bus.clone(),
                )),
            );
        }
        if config.channels.slack.enabled {
            channels.insert(
                "slack".to_string(),
                Arc::new(SlackChannel::new(
                    config.channels.slack.clone(),
                    bus.clone(),
                )),
            );
        }
        if config.channels.qq.enabled {
            channels.insert(
                "qq".to_string(),
                Arc::new(QQChannel::new(config.channels.qq.clone(), bus.clone())),
            );
        }

        Self::from_channels(bus, channels)
    }

    pub(crate) fn from_channels(
        bus: Arc<MessageBus>,
        channels: HashMap<String, Arc<dyn Channel>>,
    ) -> Self {
        Self {
            bus,
            channels,
            running: Arc::new(AtomicBool::new(false)),
            dispatch_task: Mutex::new(None),
            channel_tasks: Mutex::new(Vec::new()),
        }
    }

    pub fn enabled_channels(&self) -> Vec<String> {
        let mut names: Vec<String> = self.channels.keys().cloned().collect();
        names.sort();
        names
    }

    pub async fn start_all(&self) {
        if self.channels.is_empty() {
            return;
        }

        self.running.store(true, Ordering::Relaxed);

        let running = self.running.clone();
        let bus = self.bus.clone();
        let channels_for_dispatch = self.channels.clone();
        let dispatch = tokio::spawn(async move {
            while running.load(Ordering::Relaxed) {
                if let Some(msg) = bus.consume_outbound().await {
                    if let Some(channel) = channels_for_dispatch.get(&msg.channel) {
                        let _ = channel.send(&msg).await;
                    }
                } else {
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                }
            }
        });
        *self.dispatch_task.lock().await = Some(dispatch);

        let mut tasks = self.channel_tasks.lock().await;
        for channel in self.channels.values() {
            let ch = channel.clone();
            let task = tokio::spawn(async move {
                let _ = ch.start().await;
            });
            tasks.push(task);
        }
        drop(tasks);

        while self.running.load(Ordering::Relaxed) {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
    }

    pub async fn stop_all(&self) {
        self.running.store(false, Ordering::Relaxed);
        for channel in self.channels.values() {
            let _ = channel.stop().await;
        }

        if let Some(dispatch) = self.dispatch_task.lock().await.take() {
            dispatch.abort();
        }
        let mut tasks = self.channel_tasks.lock().await;
        for task in tasks.drain(..) {
            task.abort();
        }
    }

    pub fn get_status(&self) -> serde_json::Value {
        let mut map = serde_json::Map::new();
        for (name, channel) in &self.channels {
            map.insert(
                name.clone(),
                serde_json::json!({
                    "enabled": true,
                    "running": channel.is_running(),
                }),
            );
        }
        serde_json::Value::Object(map)
    }

    pub fn get_channel(&self, name: &str) -> Option<Arc<dyn Channel>> {
        self.channels.get(name).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::{MessageBus, OutboundMessage};
    use anyhow::Result;
    use async_trait::async_trait;
    use serde_json::{Map, Value};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use tokio::sync::Mutex as TokioMutex;

    struct MockChannel {
        name: String,
        running: AtomicBool,
        allow_from: Vec<String>,
        bus: Arc<MessageBus>,
        sent: TokioMutex<Vec<OutboundMessage>>,
    }

    impl MockChannel {
        fn new(name: &str, bus: Arc<MessageBus>) -> Self {
            Self {
                name: name.to_string(),
                running: AtomicBool::new(false),
                allow_from: Vec::new(),
                bus,
                sent: TokioMutex::new(Vec::new()),
            }
        }
    }

    #[async_trait]
    impl Channel for MockChannel {
        fn name(&self) -> &str {
            &self.name
        }

        fn is_running(&self) -> bool {
            self.running.load(Ordering::Relaxed)
        }

        fn allow_from(&self) -> &[String] {
            &self.allow_from
        }

        fn bus(&self) -> Arc<MessageBus> {
            self.bus.clone()
        }

        async fn start(&self) -> Result<()> {
            self.running.store(true, Ordering::Relaxed);
            while self.running.load(Ordering::Relaxed) {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
            Ok(())
        }

        async fn stop(&self) -> Result<()> {
            self.running.store(false, Ordering::Relaxed);
            Ok(())
        }

        async fn send(&self, msg: &OutboundMessage) -> Result<()> {
            self.sent.lock().await.push(msg.clone());
            Ok(())
        }

        async fn handle_message(
            &self,
            _sender_id: String,
            _chat_id: String,
            _content: String,
            _media: Vec<String>,
            _metadata: Map<String, Value>,
        ) -> Result<()> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn dispatches_outbound_to_matching_channel() -> Result<()> {
        let bus = Arc::new(MessageBus::new(16));
        let mock = Arc::new(MockChannel::new("mock", bus.clone()));
        let mut channels: HashMap<String, Arc<dyn Channel>> = HashMap::new();
        channels.insert("mock".to_string(), mock.clone());
        let manager = Arc::new(ChannelManager::from_channels(bus.clone(), channels));

        let run_manager = manager.clone();
        let run_handle = tokio::spawn(async move {
            run_manager.start_all().await;
        });

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        bus.publish_outbound(OutboundMessage::new("mock", "chat1", "hello"))
            .await?;

        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                if !mock.sent.lock().await.is_empty() {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await
        .map_err(|_| anyhow::anyhow!("timed out waiting outbound dispatch"))?;

        let sent = mock.sent.lock().await.clone();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].content, "hello");
        assert_eq!(sent[0].chat_id, "chat1");

        manager.stop_all().await;
        let _ = run_handle.await;
        Ok(())
    }
}
