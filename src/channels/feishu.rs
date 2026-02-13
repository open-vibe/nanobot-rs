use crate::bus::{MessageBus, OutboundMessage};
use crate::channels::base::Channel;
use crate::config::FeishuConfig;
use anyhow::{Result, anyhow};
use async_trait::async_trait;
use regex::Regex;
use reqwest::Client;
use serde_json::{Value, json};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::Mutex;

#[cfg(feature = "feishu-websocket")]
use crate::bus::InboundMessage;
#[cfg(feature = "feishu-websocket")]
use crate::channels::base::is_allowed_sender;
#[cfg(feature = "feishu-websocket")]
use open_lark::client::ws_client::LarkWsClient;
#[cfg(feature = "feishu-websocket")]
use open_lark::prelude::{AppType, EventDispatcherHandler, LarkClient, P2ImMessageReceiveV1};
#[cfg(feature = "feishu-websocket")]
use std::collections::{HashSet, VecDeque};

#[cfg(feature = "feishu-websocket")]
const MAX_DEDUP_IDS: usize = 1000;

#[cfg(feature = "feishu-websocket")]
#[derive(Default)]
struct DedupState {
    order: VecDeque<String>,
    seen: HashSet<String>,
}

pub struct FeishuChannel {
    config: FeishuConfig,
    bus: Arc<MessageBus>,
    running: Arc<AtomicBool>,
    http: Client,
    tenant_access_token: Mutex<Option<String>>,
    #[cfg(feature = "feishu-websocket")]
    ws_thread: Mutex<Option<std::thread::JoinHandle<()>>>,
    #[cfg(feature = "feishu-websocket")]
    dedup: Arc<Mutex<DedupState>>,
}

impl FeishuChannel {
    pub fn new(config: FeishuConfig, bus: Arc<MessageBus>) -> Self {
        Self {
            config,
            bus,
            running: Arc::new(AtomicBool::new(false)),
            http: Client::new(),
            tenant_access_token: Mutex::new(None),
            #[cfg(feature = "feishu-websocket")]
            ws_thread: Mutex::new(None),
            #[cfg(feature = "feishu-websocket")]
            dedup: Arc::new(Mutex::new(DedupState::default())),
        }
    }

    async fn get_tenant_access_token(&self) -> Result<String> {
        if let Some(token) = self.tenant_access_token.lock().await.clone() {
            return Ok(token);
        }
        if self.config.app_id.is_empty() || self.config.app_secret.is_empty() {
            return Err(anyhow!("feishu app id/secret not configured"));
        }

        let response = self
            .http
            .post("https://open.feishu.cn/open-apis/auth/v3/tenant_access_token/internal")
            .json(&json!({
                "app_id": self.config.app_id,
                "app_secret": self.config.app_secret,
            }))
            .send()
            .await?;
        let payload: Value = response.json().await?;
        let token = payload
            .get("tenant_access_token")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("failed to get feishu tenant_access_token: {payload}"))?
            .to_string();
        *self.tenant_access_token.lock().await = Some(token.clone());
        Ok(token)
    }

    fn parse_md_table(table_text: &str) -> Option<Value> {
        let lines = table_text
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>();
        if lines.len() < 3 {
            return None;
        }
        let split_row = |line: &str| {
            line.trim_matches('|')
                .split('|')
                .map(|c| c.trim().to_string())
                .collect::<Vec<_>>()
        };
        let headers = split_row(lines[0]);
        let rows = lines
            .iter()
            .skip(2)
            .map(|line| split_row(line))
            .collect::<Vec<_>>();
        let columns = headers
            .iter()
            .enumerate()
            .map(|(i, header)| {
                json!({
                    "tag": "column",
                    "name": format!("c{i}"),
                    "display_name": header,
                    "width": "auto"
                })
            })
            .collect::<Vec<_>>();
        let row_values = rows
            .iter()
            .map(|row| {
                let mut map = serde_json::Map::new();
                for (i, _) in headers.iter().enumerate() {
                    map.insert(
                        format!("c{i}"),
                        Value::String(row.get(i).cloned().unwrap_or_default()),
                    );
                }
                Value::Object(map)
            })
            .collect::<Vec<_>>();
        Some(json!({
            "tag": "table",
            "page_size": row_values.len() + 1,
            "columns": columns,
            "rows": row_values,
        }))
    }

    fn build_card_elements(&self, content: &str) -> Vec<Value> {
        let table_re = Regex::new(
            r"(?m)((?:^[ \t]*\|.+\|[ \t]*\n)(?:^[ \t]*\|[-:\s|]+\|[ \t]*\n)(?:^[ \t]*\|.+\|[ \t]*\n?)+)",
        )
        .expect("valid feishu table regex");
        let mut elements = Vec::new();
        let mut last_end = 0usize;
        for m in table_re.find_iter(content) {
            let before = &content[last_end..m.start()];
            if !before.trim().is_empty() {
                elements.extend(Self::split_headings(before));
            }
            let raw_table = m.as_str();
            if let Some(parsed) = Self::parse_md_table(raw_table) {
                elements.push(parsed);
            } else {
                elements.push(json!({"tag":"markdown","content": raw_table}));
            }
            last_end = m.end();
        }
        let remaining = &content[last_end..];
        if !remaining.trim().is_empty() {
            elements.extend(Self::split_headings(remaining));
        }
        if elements.is_empty() {
            elements.push(json!({"tag":"markdown","content": content}));
        }
        elements
    }

    fn split_headings(content: &str) -> Vec<Value> {
        let heading_re = Regex::new(r"(?m)^(#{1,6})\s+(.+)$").expect("valid heading regex");
        let code_block_re = Regex::new(r"(?ms)(```[\s\S]*?```)").expect("valid code block regex");

        let mut protected = content.to_string();
        let mut code_blocks = Vec::new();
        for cap in code_block_re.captures_iter(content) {
            if let Some(m) = cap.get(1) {
                code_blocks.push(m.as_str().to_string());
            }
        }
        for (idx, block) in code_blocks.iter().enumerate() {
            let token = format!("\u{0000}CODE{idx}\u{0000}");
            protected = protected.replacen(block, &token, 1);
        }

        let mut elements = Vec::new();
        let mut last_end = 0usize;
        for cap in heading_re.captures_iter(&protected) {
            let Some(m) = cap.get(0) else {
                continue;
            };
            let before = protected[last_end..m.start()].trim();
            if !before.is_empty() {
                elements.push(json!({"tag":"markdown","content": before}));
            }
            let text = cap.get(2).map(|v| v.as_str().trim()).unwrap_or_default();
            elements.push(json!({
                "tag":"div",
                "text": {
                    "tag":"lark_md",
                    "content": format!("**{text}**"),
                }
            }));
            last_end = m.end();
        }
        let remaining = protected[last_end..].trim();
        if !remaining.is_empty() {
            elements.push(json!({"tag":"markdown","content": remaining}));
        }

        for (idx, block) in code_blocks.iter().enumerate() {
            let token = format!("\u{0000}CODE{idx}\u{0000}");
            for element in &mut elements {
                if element.get("tag").and_then(Value::as_str) == Some("markdown")
                    && let Some(content) = element.get_mut("content")
                    && let Some(text) = content.as_str()
                {
                    *content = Value::String(text.replace(&token, block));
                }
            }
        }

        if elements.is_empty() {
            vec![json!({"tag":"markdown","content": content})]
        } else {
            elements
        }
    }

    #[cfg(feature = "feishu-websocket")]
    fn build_event_handler(
        bus: Arc<MessageBus>,
        allow_from: Vec<String>,
        dedup: Arc<Mutex<DedupState>>,
        verification_token: String,
        encrypt_key: String,
    ) -> Result<EventDispatcherHandler> {
        let bus_outer = bus.clone();
        let allow_from_outer = allow_from.clone();
        let dedup_outer = dedup.clone();

        let builder = EventDispatcherHandler::builder().register_p2_im_message_receive_v1(
            move |event: P2ImMessageReceiveV1| {
                let bus = bus_outer.clone();
                let allow_from = allow_from_outer.clone();
                let dedup = dedup_outer.clone();
                tokio::spawn(async move {
                    let message = event.event.message;
                    let sender = event.event.sender;
                    if sender.sender_type == "bot" {
                        return;
                    }

                    let message_id = message.message_id.clone();
                    {
                        let mut state = dedup.lock().await;
                        if state.seen.contains(&message_id) {
                            return;
                        }
                        state.seen.insert(message_id.clone());
                        state.order.push_back(message_id.clone());
                        while state.order.len() > MAX_DEDUP_IDS {
                            if let Some(old) = state.order.pop_front() {
                                state.seen.remove(&old);
                            }
                        }
                    }

                    let sender_id = sender.sender_id.open_id;
                    if !is_allowed_sender(&sender_id, &allow_from) {
                        return;
                    }

                    let msg_type = message.message_type.clone();
                    let content = if msg_type == "text" {
                        serde_json::from_str::<Value>(&message.content)
                            .ok()
                            .and_then(|v| {
                                v.get("text").and_then(Value::as_str).map(ToOwned::to_owned)
                            })
                            .unwrap_or_else(|| message.content.clone())
                    } else {
                        match msg_type.as_str() {
                            "image" => "[image]".to_string(),
                            "audio" => "[audio]".to_string(),
                            "file" => "[file]".to_string(),
                            "sticker" => "[sticker]".to_string(),
                            _ => format!("[{}]", msg_type),
                        }
                    };
                    if content.trim().is_empty() {
                        return;
                    }

                    let chat_id = if message.chat_type == "group" {
                        message.chat_id
                    } else {
                        sender_id.clone()
                    };

                    let mut inbound = InboundMessage::new("feishu", sender_id, chat_id, content);
                    inbound
                        .metadata
                        .insert("message_id".to_string(), Value::String(message_id));
                    inbound
                        .metadata
                        .insert("chat_type".to_string(), Value::String(message.chat_type));
                    inbound
                        .metadata
                        .insert("msg_type".to_string(), Value::String(msg_type));
                    let _ = bus.publish_inbound(inbound).await;
                });
            },
        );

        let mut handler = builder
            .map_err(|e| anyhow!("failed to register feishu event handler: {e}"))?
            .build();
        if !verification_token.is_empty() {
            handler.set_verification_token(verification_token);
        }
        if !encrypt_key.is_empty() {
            handler.set_event_encrypt_key(encrypt_key);
        }
        Ok(handler)
    }

    #[cfg(feature = "feishu-websocket")]
    fn spawn_ws_thread(&self) -> std::thread::JoinHandle<()> {
        let app_id = self.config.app_id.clone();
        let app_secret = self.config.app_secret.clone();
        let allow_from = self.config.allow_from.clone();
        let verification_token = self.config.verification_token.clone();
        let encrypt_key = self.config.encrypt_key.clone();
        let running = self.running.clone();
        let bus = self.bus.clone();
        let dedup = self.dedup.clone();

        std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build();
            let Ok(runtime) = runtime else {
                eprintln!("Feishu: failed to create runtime for websocket receiver");
                return;
            };

            runtime.block_on(async move {
                while running.load(Ordering::Relaxed) {
                    let client = Arc::new(
                        LarkClient::builder(&app_id, &app_secret)
                            .with_app_type(AppType::SelfBuild)
                            .with_enable_token_cache(true)
                            .build(),
                    );
                    let ws_config = Arc::new(client.config.clone());
                    let handler = Self::build_event_handler(
                        bus.clone(),
                        allow_from.clone(),
                        dedup.clone(),
                        verification_token.clone(),
                        encrypt_key.clone(),
                    );
                    let Ok(handler) = handler else {
                        eprintln!("Feishu: failed to build event handler");
                        return;
                    };

                    let _ = LarkWsClient::open(ws_config, handler).await;
                    if running.load(Ordering::Relaxed) {
                        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                    }
                }
            });
        })
    }
}

#[async_trait]
impl Channel for FeishuChannel {
    fn name(&self) -> &str {
        "feishu"
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
        #[cfg(not(feature = "feishu-websocket"))]
        {
            eprintln!("Feishu receive loop is disabled. Rebuild with --features feishu-websocket.");
        }
        #[cfg(feature = "feishu-websocket")]
        {
            if !self.config.app_id.is_empty() && !self.config.app_secret.is_empty() {
                let ws_thread = self.spawn_ws_thread();
                *self.ws_thread.lock().await = Some(ws_thread);
            }
        }

        while self.running.load(Ordering::Relaxed) {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
        Ok(())
    }

    async fn stop(&self) -> Result<()> {
        self.running.store(false, Ordering::Relaxed);
        #[cfg(feature = "feishu-websocket")]
        if let Some(handle) = self.ws_thread.lock().await.take() {
            drop(handle);
        }
        Ok(())
    }

    async fn send(&self, msg: &OutboundMessage) -> Result<()> {
        let token = self.get_tenant_access_token().await?;
        let receive_id_type = if msg.chat_id.starts_with("oc_") {
            "chat_id"
        } else {
            "open_id"
        };
        let url = format!(
            "https://open.feishu.cn/open-apis/im/v1/messages?receive_id_type={receive_id_type}"
        );
        let elements = self.build_card_elements(&msg.content);
        let card = json!({
            "config": {"wide_screen_mode": true},
            "elements": elements,
        });
        let resp = self
            .http
            .post(url)
            .bearer_auth(token)
            .json(&json!({
                "receive_id": msg.chat_id,
                "msg_type": "interactive",
                "content": card.to_string(),
            }))
            .send()
            .await?;
        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(anyhow!("failed to send feishu message: {body}"));
        }
        Ok(())
    }
}
