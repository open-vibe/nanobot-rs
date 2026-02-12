use crate::bus::{MessageBus, OutboundMessage};
use crate::channels::base::Channel;
use crate::config::TelegramConfig;
use crate::providers::transcription::GroqTranscriptionProvider;
use crate::session::SessionManager;
use anyhow::Result;
use async_trait::async_trait;
use html_escape::encode_text;
use regex::Regex;
use reqwest::{Client, Proxy};
use serde_json::{Map, Value, json};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

fn markdown_to_telegram_html(text: &str) -> String {
    if text.is_empty() {
        return String::new();
    }
    let mut content = text.to_string();

    let code_block_re =
        Regex::new(r"(?s)```[\w]*\n?([\s\S]*?)```").expect("valid code block regex");
    let inline_code_re = Regex::new(r"`([^`]+)`").expect("valid inline code regex");
    let header_re = Regex::new(r"(?m)^#{1,6}\s+(.+)$").expect("valid header regex");
    let quote_re = Regex::new(r"(?m)^>\s*(.*)$").expect("valid quote regex");
    let link_re = Regex::new(r"\[([^\]]+)\]\(([^)]+)\)").expect("valid link regex");
    let bold_star_re = Regex::new(r"\*\*(.+?)\*\*").expect("valid bold regex");
    let bold_underscore_re = Regex::new(r"__(.+?)__").expect("valid bold underscore regex");
    let italic_re =
        Regex::new(r"(?m)(^|[^A-Za-z0-9])_([^_]+)_([^A-Za-z0-9]|$)").expect("valid italic regex");
    let strike_re = Regex::new(r"~~(.+?)~~").expect("valid strike regex");
    let bullet_re = Regex::new(r"(?m)^[-*]\s+").expect("valid bullet regex");

    let mut code_blocks = Vec::new();
    content = code_block_re
        .replace_all(&content, |caps: &regex::Captures<'_>| {
            let idx = code_blocks.len();
            code_blocks.push(caps[1].to_string());
            format!("\u{0001}CB{idx}\u{0002}")
        })
        .to_string();

    let mut inline_codes = Vec::new();
    content = inline_code_re
        .replace_all(&content, |caps: &regex::Captures<'_>| {
            let idx = inline_codes.len();
            inline_codes.push(caps[1].to_string());
            format!("\u{0001}IC{idx}\u{0002}")
        })
        .to_string();

    content = header_re.replace_all(&content, "$1").to_string();
    content = quote_re.replace_all(&content, "$1").to_string();
    content = encode_text(&content).to_string();
    content = link_re
        .replace_all(&content, r#"<a href="$2">$1</a>"#)
        .to_string();
    content = bold_star_re.replace_all(&content, "<b>$1</b>").to_string();
    content = bold_underscore_re
        .replace_all(&content, "<b>$1</b>")
        .to_string();
    content = italic_re.replace_all(&content, "$1<i>$2</i>$3").to_string();
    content = strike_re.replace_all(&content, "<s>$1</s>").to_string();
    content = bullet_re.replace_all(&content, "• ").to_string();

    for (idx, value) in inline_codes.iter().enumerate() {
        let token = format!("\u{0001}IC{idx}\u{0002}");
        let escaped = encode_text(value);
        content = content.replace(&token, &format!("<code>{escaped}</code>"));
    }
    for (idx, value) in code_blocks.iter().enumerate() {
        let token = format!("\u{0001}CB{idx}\u{0002}");
        let escaped = encode_text(value);
        content = content.replace(&token, &format!("<pre><code>{escaped}</code></pre>"));
    }

    content
}

pub struct TelegramChannel {
    config: TelegramConfig,
    bus: Arc<MessageBus>,
    running: AtomicBool,
    client: Client,
    offset: Mutex<i64>,
    groq_api_key: String,
    session_manager: Option<Arc<SessionManager>>,
    typing_tasks: Mutex<HashMap<String, JoinHandle<()>>>,
}

#[cfg(test)]
mod tests {
    use super::markdown_to_telegram_html;

    #[test]
    fn markdown_converter_preserves_code_blocks_and_escapes_html() {
        let input = "```rust\nlet x = 1 < 2;\n```\ntext";
        let out = markdown_to_telegram_html(input);
        assert!(out.contains("<pre><code>let x = 1 &lt; 2;\n</code></pre>"));
        assert!(out.contains("text"));
    }

    #[test]
    fn markdown_converter_formats_links_and_styles() {
        let input = "[site](https://example.com) **b** _i_ ~~s~~";
        let out = markdown_to_telegram_html(input);
        assert!(out.contains(r#"<a href="https://example.com">site</a>"#));
        assert!(out.contains("<b>b</b>"));
        assert!(out.contains("<i>i</i>"));
        assert!(out.contains("<s>s</s>"));
    }
}

impl TelegramChannel {
    fn build_http_client(proxy: Option<&str>) -> Client {
        let base_builder = || {
            Client::builder()
                .connect_timeout(std::time::Duration::from_secs(30))
                .timeout(std::time::Duration::from_secs(60))
                .pool_max_idle_per_host(16)
        };

        if let Some(proxy_url) = proxy {
            match Proxy::all(proxy_url) {
                Ok(proxy) => base_builder().proxy(proxy).build().unwrap_or_else(|err| {
                    eprintln!("Telegram HTTP client build with proxy failed ({proxy_url}): {err}");
                    base_builder().build().unwrap_or_else(|_| Client::new())
                }),
                Err(err) => {
                    eprintln!("Telegram proxy URL is invalid ({proxy_url}): {err}");
                    base_builder().build().unwrap_or_else(|_| Client::new())
                }
            }
        } else {
            base_builder().build().unwrap_or_else(|_| Client::new())
        }
    }

    pub fn new(
        config: TelegramConfig,
        bus: Arc<MessageBus>,
        groq_api_key: String,
        session_manager: Option<Arc<SessionManager>>,
    ) -> Self {
        let client = Self::build_http_client(config.proxy.as_deref());
        Self {
            config,
            bus,
            running: AtomicBool::new(false),
            client,
            offset: Mutex::new(0),
            groq_api_key,
            session_manager,
            typing_tasks: Mutex::new(HashMap::new()),
        }
    }

    fn api_url(&self, method: &str) -> String {
        format!(
            "https://api.telegram.org/bot{}/{}",
            self.config.token, method
        )
    }

    fn file_url(&self, file_path: &str) -> String {
        format!(
            "https://api.telegram.org/file/bot{}/{}",
            self.config.token, file_path
        )
    }

    async fn download_media(
        &self,
        file_id: &str,
        media_type: &str,
        mime_type: Option<&str>,
    ) -> Option<PathBuf> {
        let response = self
            .client
            .get(self.api_url("getFile"))
            .query(&[("file_id", file_id)])
            .send()
            .await
            .ok()?;
        let body: Value = response.json().await.ok()?;
        let file_path = body
            .get("result")
            .and_then(|v| v.get("file_path"))
            .and_then(Value::as_str)?;

        let bytes = self
            .client
            .get(self.file_url(file_path))
            .send()
            .await
            .ok()?
            .bytes()
            .await
            .ok()?;

        let ext = self.get_extension(media_type, mime_type);
        let media_dir = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".nanobot")
            .join("media");
        tokio::fs::create_dir_all(&media_dir).await.ok()?;
        let save_path = media_dir.join(format!("{}{}", &file_id[..file_id.len().min(16)], ext));
        tokio::fs::write(&save_path, &bytes).await.ok()?;
        Some(save_path)
    }

    async fn send_text_message(&self, chat_id: &str, text: &str, parse_mode: Option<&str>) {
        let mut payload = json!({
            "chat_id": chat_id,
            "text": text
        });
        if let Some(parse_mode) = parse_mode {
            payload["parse_mode"] = Value::String(parse_mode.to_string());
        }
        let _ = self
            .client
            .post(self.api_url("sendMessage"))
            .json(&payload)
            .send()
            .await;
    }

    async fn start_typing(&self, chat_id: &str) {
        self.stop_typing(chat_id).await;
        let api_url = self.api_url("sendChatAction");
        let chat_id_owned = chat_id.to_string();
        let client = self.client.clone();
        let task = tokio::spawn(async move {
            loop {
                let _ = client
                    .post(&api_url)
                    .json(&json!({
                        "chat_id": chat_id_owned,
                        "action": "typing"
                    }))
                    .send()
                    .await;
                tokio::time::sleep(std::time::Duration::from_secs(4)).await;
            }
        });
        self.typing_tasks
            .lock()
            .await
            .insert(chat_id.to_string(), task);
    }

    async fn stop_typing(&self, chat_id: &str) {
        if let Some(task) = self.typing_tasks.lock().await.remove(chat_id) {
            task.abort();
        }
    }

    fn get_extension(&self, media_type: &str, mime_type: Option<&str>) -> &'static str {
        if let Some(mime_type) = mime_type {
            match mime_type {
                "image/jpeg" => return ".jpg",
                "image/png" => return ".png",
                "image/gif" => return ".gif",
                "audio/ogg" => return ".ogg",
                "audio/mpeg" => return ".mp3",
                "audio/mp4" => return ".m4a",
                _ => {}
            }
        }
        match media_type {
            "image" => ".jpg",
            "voice" => ".ogg",
            "audio" => ".mp3",
            _ => "",
        }
    }

    async fn handle_update(&self, update: &Value) -> Result<()> {
        let Some(message) = update.get("message") else {
            return Ok(());
        };
        let Some(user) = message.get("from") else {
            return Ok(());
        };
        let user_id = user.get("id").and_then(Value::as_i64).unwrap_or_default();
        if user_id == 0 {
            return Ok(());
        }
        let username = user.get("username").and_then(Value::as_str);
        let sender_id = if let Some(username) = username {
            format!("{user_id}|{username}")
        } else {
            user_id.to_string()
        };

        let chat_id = message
            .get("chat")
            .and_then(|v| v.get("id"))
            .and_then(Value::as_i64)
            .unwrap_or_default()
            .to_string();
        if chat_id == "0" {
            return Ok(());
        }

        let mut content_parts = Vec::new();
        let mut media_paths = Vec::new();
        if let Some(text) = message.get("text").and_then(Value::as_str) {
            if text.starts_with('/') {
                let command = text
                    .trim_start_matches('/')
                    .split_whitespace()
                    .next()
                    .unwrap_or("")
                    .split('@')
                    .next()
                    .unwrap_or("");
                match command {
                    "start" => {
                        let first_name = user
                            .get("first_name")
                            .and_then(Value::as_str)
                            .unwrap_or("there");
                        self.send_text_message(
                            &chat_id,
                            &format!(
                                "Hi {first_name}! I'm nanobot.\n\nSend me a message and I'll respond!\nType /help to see available commands."
                            ),
                            None,
                        )
                        .await;
                    }
                    "help" => {
                        self.send_text_message(
                            &chat_id,
                            "nanobot commands:\n/start - Start the bot\n/reset - Reset conversation history\n/help - Show this help message",
                            None,
                        )
                        .await;
                    }
                    "reset" => {
                        if let Some(session_manager) = &self.session_manager {
                            let session_key = format!("{}:{}", self.name(), chat_id);
                            let _ = session_manager.delete(&session_key);
                            self.send_text_message(
                                &chat_id,
                                "Conversation history cleared. Let's start fresh!",
                                None,
                            )
                            .await;
                        } else {
                            self.send_text_message(
                                &chat_id,
                                "Session management is not available.",
                                None,
                            )
                            .await;
                        }
                    }
                    _ => {}
                }
                return Ok(());
            }
            content_parts.push(text.to_string());
        }
        if let Some(caption) = message.get("caption").and_then(Value::as_str) {
            content_parts.push(caption.to_string());
        }

        let mut media_file_id = None::<String>;
        let mut media_type = None::<String>;
        let mut mime_type = None::<String>;

        if let Some(photos) = message.get("photo").and_then(Value::as_array) {
            if let Some(last) = photos.last() {
                media_file_id = last
                    .get("file_id")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned);
                media_type = Some("image".to_string());
            }
        } else if let Some(voice) = message.get("voice") {
            media_file_id = voice
                .get("file_id")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned);
            media_type = Some("voice".to_string());
            mime_type = voice
                .get("mime_type")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned);
        } else if let Some(audio) = message.get("audio") {
            media_file_id = audio
                .get("file_id")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned);
            media_type = Some("audio".to_string());
            mime_type = audio
                .get("mime_type")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned);
        } else if let Some(doc) = message.get("document") {
            media_file_id = doc
                .get("file_id")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned);
            media_type = Some("file".to_string());
            mime_type = doc
                .get("mime_type")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned);
        }

        if let (Some(file_id), Some(kind)) = (media_file_id.as_deref(), media_type.as_deref()) {
            if let Some(path) = self
                .download_media(file_id, kind, mime_type.as_deref())
                .await
            {
                media_paths.push(path.display().to_string());
                if kind == "voice" || kind == "audio" {
                    let transcriber =
                        GroqTranscriptionProvider::new(Some(self.groq_api_key.clone()));
                    let transcription = transcriber.transcribe(&path).await.unwrap_or_default();
                    if !transcription.is_empty() {
                        content_parts.push(format!("[transcription: {transcription}]"));
                    } else {
                        content_parts.push(format!("[{kind}: {}]", path.display()));
                    }
                } else {
                    content_parts.push(format!("[{kind}: {}]", path.display()));
                }
            } else {
                content_parts.push(format!("[{kind}: download failed]"));
            }
        }

        let mut metadata = Map::new();
        metadata.insert(
            "message_id".to_string(),
            message.get("message_id").cloned().unwrap_or(Value::Null),
        );
        metadata.insert("user_id".to_string(), Value::Number(user_id.into()));
        metadata.insert(
            "username".to_string(),
            username
                .map(|v| Value::String(v.to_string()))
                .unwrap_or(Value::Null),
        );
        metadata.insert(
            "first_name".to_string(),
            user.get("first_name").cloned().unwrap_or(Value::Null),
        );
        metadata.insert(
            "is_group".to_string(),
            Value::Bool(
                message
                    .get("chat")
                    .and_then(|v| v.get("type"))
                    .and_then(Value::as_str)
                    .map(|t| t != "private")
                    .unwrap_or(false),
            ),
        );

        self.start_typing(&chat_id).await;

        self.handle_message(
            sender_id,
            chat_id,
            if content_parts.is_empty() {
                "[empty message]".to_string()
            } else {
                content_parts.join("\n")
            },
            media_paths,
            metadata,
        )
        .await
    }
}

#[async_trait]
impl Channel for TelegramChannel {
    fn name(&self) -> &str {
        "telegram"
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
            let offset = *self.offset.lock().await;
            let response = self
                .client
                .post(self.api_url("getUpdates"))
                .json(&json!({
                    "offset": if offset > 0 { Value::Number(offset.into()) } else { Value::Null },
                    "timeout": 20,
                    "allowed_updates": ["message"]
                }))
                .send()
                .await;

            let Ok(response) = response else {
                if let Err(err) = response {
                    eprintln!("Telegram polling request error: {err}");
                }
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                continue;
            };
            let body: Value = match response.json().await {
                Ok(body) => body,
                Err(err) => {
                    eprintln!("Telegram polling decode error: {err}");
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    continue;
                }
            };
            if !body.get("ok").and_then(Value::as_bool).unwrap_or(false) {
                if let Some(desc) = body.get("description").and_then(Value::as_str) {
                    eprintln!("Telegram polling returned not ok: {desc}");
                }
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                continue;
            }

            if let Some(results) = body.get("result").and_then(Value::as_array) {
                for update in results {
                    if let Some(update_id) = update.get("update_id").and_then(Value::as_i64) {
                        *self.offset.lock().await = update_id + 1;
                    }
                    if let Err(err) = self.handle_update(update).await {
                        eprintln!("Telegram update handling error: {err}");
                    }
                }
            }
        }
        Ok(())
    }

    async fn stop(&self) -> Result<()> {
        self.running.store(false, Ordering::Relaxed);
        let mut typing_tasks = self.typing_tasks.lock().await;
        for (_, task) in typing_tasks.drain() {
            task.abort();
        }
        Ok(())
    }

    async fn send(&self, msg: &OutboundMessage) -> Result<()> {
        self.stop_typing(&msg.chat_id).await;
        let html = markdown_to_telegram_html(&msg.content);
        let first_try = self
            .client
            .post(self.api_url("sendMessage"))
            .json(&json!({
                "chat_id": msg.chat_id,
                "text": html,
                "parse_mode": "HTML"
            }))
            .send()
            .await?;

        if first_try.status().is_success() {
            return Ok(());
        }

        self.send_text_message(&msg.chat_id, &msg.content, None)
            .await;
        Ok(())
    }
}
