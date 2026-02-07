use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::{Mutex, mpsc};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InboundMessage {
    pub channel: String,
    pub sender_id: String,
    pub chat_id: String,
    pub content: String,
    pub timestamp: DateTime<Local>,
    pub media: Vec<String>,
    pub metadata: Map<String, Value>,
}

impl InboundMessage {
    pub fn new(
        channel: impl Into<String>,
        sender_id: impl Into<String>,
        chat_id: impl Into<String>,
        content: impl Into<String>,
    ) -> Self {
        Self {
            channel: channel.into(),
            sender_id: sender_id.into(),
            chat_id: chat_id.into(),
            content: content.into(),
            timestamp: Local::now(),
            media: Vec::new(),
            metadata: Map::new(),
        }
    }

    pub fn session_key(&self) -> String {
        format!("{}:{}", self.channel, self.chat_id)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutboundMessage {
    pub channel: String,
    pub chat_id: String,
    pub content: String,
    pub reply_to: Option<String>,
    pub media: Vec<String>,
    pub metadata: Map<String, Value>,
}

impl OutboundMessage {
    pub fn new(
        channel: impl Into<String>,
        chat_id: impl Into<String>,
        content: impl Into<String>,
    ) -> Self {
        Self {
            channel: channel.into(),
            chat_id: chat_id.into(),
            content: content.into(),
            reply_to: None,
            media: Vec::new(),
            metadata: Map::new(),
        }
    }
}

pub struct MessageBus {
    inbound_tx: mpsc::Sender<InboundMessage>,
    inbound_rx: Mutex<mpsc::Receiver<InboundMessage>>,
    outbound_tx: mpsc::Sender<OutboundMessage>,
    outbound_rx: Mutex<mpsc::Receiver<OutboundMessage>>,
    inbound_size: AtomicUsize,
    outbound_size: AtomicUsize,
}

impl MessageBus {
    pub fn new(capacity: usize) -> Self {
        let (inbound_tx, inbound_rx) = mpsc::channel(capacity);
        let (outbound_tx, outbound_rx) = mpsc::channel(capacity);
        Self {
            inbound_tx,
            inbound_rx: Mutex::new(inbound_rx),
            outbound_tx,
            outbound_rx: Mutex::new(outbound_rx),
            inbound_size: AtomicUsize::new(0),
            outbound_size: AtomicUsize::new(0),
        }
    }

    pub fn inbound_sender(&self) -> mpsc::Sender<InboundMessage> {
        self.inbound_tx.clone()
    }

    pub fn outbound_sender(&self) -> mpsc::Sender<OutboundMessage> {
        self.outbound_tx.clone()
    }

    pub async fn publish_inbound(&self, msg: InboundMessage) -> anyhow::Result<()> {
        self.inbound_size.fetch_add(1, Ordering::Relaxed);
        if let Err(err) = self.inbound_tx.send(msg).await {
            self.inbound_size.fetch_sub(1, Ordering::Relaxed);
            return Err(anyhow::anyhow!("failed to publish inbound message: {err}"));
        }
        Ok(())
    }

    pub async fn consume_inbound(&self) -> Option<InboundMessage> {
        let mut rx = self.inbound_rx.lock().await;
        let msg = rx.recv().await;
        if msg.is_some() {
            self.inbound_size.fetch_sub(1, Ordering::Relaxed);
        }
        msg
    }

    pub async fn publish_outbound(&self, msg: OutboundMessage) -> anyhow::Result<()> {
        self.outbound_size.fetch_add(1, Ordering::Relaxed);
        if let Err(err) = self.outbound_tx.send(msg).await {
            self.outbound_size.fetch_sub(1, Ordering::Relaxed);
            return Err(anyhow::anyhow!("failed to publish outbound message: {err}"));
        }
        Ok(())
    }

    pub async fn consume_outbound(&self) -> Option<OutboundMessage> {
        let mut rx = self.outbound_rx.lock().await;
        let msg = rx.recv().await;
        if msg.is_some() {
            self.outbound_size.fetch_sub(1, Ordering::Relaxed);
        }
        msg
    }

    pub fn inbound_size(&self) -> usize {
        self.inbound_size.load(Ordering::Relaxed)
    }

    pub fn outbound_size(&self) -> usize {
        self.outbound_size.load(Ordering::Relaxed)
    }
}
