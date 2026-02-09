use crate::bus::{MessageBus, OutboundMessage};
use crate::channels::base::Channel;
use crate::config::QQConfig;
use anyhow::{Result, anyhow};
use async_trait::async_trait;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

#[cfg(feature = "qq-botrs")]
use crate::bus::InboundMessage;
#[cfg(feature = "qq-botrs")]
use crate::channels::base::is_allowed_sender;
#[cfg(feature = "qq-botrs")]
use botrs::models::message::C2CMessageParams;
#[cfg(feature = "qq-botrs")]
use botrs::{C2CMessage, Context as QQContext, EventHandler, Intents, Ready, Token};
#[cfg(feature = "qq-botrs")]
use serde_json::Value;
#[cfg(feature = "qq-botrs")]
use std::collections::VecDeque;
#[cfg(feature = "qq-botrs")]
use tokio::sync::Mutex;

#[cfg(feature = "qq-botrs")]
const QQ_DEDUPE_CAPACITY: usize = 1000;

#[cfg(feature = "qq-botrs")]
struct QQShared {
    bus: Arc<MessageBus>,
    allow_from: Vec<String>,
    context: Mutex<Option<QQContext>>,
    processed_ids: Mutex<VecDeque<String>>,
}

#[cfg(feature = "qq-botrs")]
struct QQEventHandler {
    shared: Arc<QQShared>,
}

#[cfg(feature = "qq-botrs")]
impl QQEventHandler {
    async fn dedupe_message(&self, message_id: &str) -> bool {
        if message_id.is_empty() {
            return false;
        }
        let mut ids = self.shared.processed_ids.lock().await;
        if ids.iter().any(|v| v == message_id) {
            return true;
        }
        ids.push_back(message_id.to_string());
        if ids.len() > QQ_DEDUPE_CAPACITY {
            ids.pop_front();
        }
        false
    }
}

#[cfg(feature = "qq-botrs")]
#[async_trait]
impl EventHandler for QQEventHandler {
    async fn ready(&self, ctx: QQContext, _ready: Ready) {
        *self.shared.context.lock().await = Some(ctx);
    }

    async fn c2c_message_create(&self, ctx: QQContext, message: C2CMessage) {
        *self.shared.context.lock().await = Some(ctx);

        let sender = message
            .author
            .as_ref()
            .and_then(|a| a.user_openid.clone())
            .unwrap_or_default();
        if sender.is_empty() {
            return;
        }
        if !is_allowed_sender(&sender, &self.shared.allow_from) {
            return;
        }

        let content = message.content.unwrap_or_default().trim().to_string();
        if content.is_empty() {
            return;
        }

        let message_id = message.id.unwrap_or_default();
        if self.dedupe_message(&message_id).await {
            return;
        }

        let mut inbound = InboundMessage::new("qq", sender.clone(), sender, content);
        if !message_id.is_empty() {
            inbound
                .metadata
                .insert("message_id".to_string(), Value::String(message_id));
        }
        let _ = self.shared.bus.publish_inbound(inbound).await;
    }
}

pub struct QQChannel {
    config: QQConfig,
    bus: Arc<MessageBus>,
    running: AtomicBool,
    #[cfg(feature = "qq-botrs")]
    shared: Arc<QQShared>,
}

impl QQChannel {
    pub fn new(config: QQConfig, bus: Arc<MessageBus>) -> Self {
        Self {
            #[cfg(feature = "qq-botrs")]
            shared: Arc::new(QQShared {
                bus: bus.clone(),
                allow_from: config.allow_from.clone(),
                context: Mutex::new(None),
                processed_ids: Mutex::new(VecDeque::new()),
            }),
            config,
            bus,
            running: AtomicBool::new(false),
        }
    }
}

#[async_trait]
impl Channel for QQChannel {
    fn name(&self) -> &str {
        "qq"
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

        #[cfg(not(feature = "qq-botrs"))]
        {
            eprintln!("QQ support is disabled. Rebuild with --features qq-botrs.");
            self.running.store(false, Ordering::Relaxed);
            Ok(())
        }

        #[cfg(feature = "qq-botrs")]
        {
            if self.config.app_id.is_empty() || self.config.secret.is_empty() {
                self.running.store(false, Ordering::Relaxed);
                return Ok(());
            }

            let token = Token::new(self.config.app_id.clone(), self.config.secret.clone());
            if let Err(err) = token.validate() {
                self.running.store(false, Ordering::Relaxed);
                return Err(anyhow!("invalid QQ token: {err}"));
            }

            let intents = Intents::default().with_public_messages();
            let handler = QQEventHandler {
                shared: self.shared.clone(),
            };
            let mut client = botrs::Client::new(token, intents, handler, false)
                .map_err(|e| anyhow!("failed to create QQ client: {e}"))?;

            let run_result = client.start().await;
            self.running.store(false, Ordering::Relaxed);
            run_result.map_err(|e| anyhow!("QQ client stopped: {e}"))?;
            Ok(())
        }
    }

    async fn stop(&self) -> Result<()> {
        self.running.store(false, Ordering::Relaxed);
        #[cfg(feature = "qq-botrs")]
        {
            *self.shared.context.lock().await = None;
        }
        Ok(())
    }

    async fn send(&self, msg: &OutboundMessage) -> Result<()> {
        #[cfg(not(feature = "qq-botrs"))]
        {
            let _ = msg;
            return Err(anyhow!(
                "QQ support is disabled; build with --features qq-botrs"
            ));
        }

        #[cfg(feature = "qq-botrs")]
        {
            let ctx = self.shared.context.lock().await.clone();
            let Some(ctx) = ctx else {
                return Ok(());
            };
            let params = C2CMessageParams {
                msg_type: 0,
                content: Some(msg.content.clone()),
                ..Default::default()
            };
            ctx.api
                .post_c2c_message_with_params(&ctx.token, &msg.chat_id, params)
                .await
                .map_err(|e| anyhow!("failed to send QQ C2C message: {e}"))?;
            Ok(())
        }
    }
}
