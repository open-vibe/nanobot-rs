use crate::bus::{MessageBus, OutboundMessage};
use crate::channels::base::Channel;
use crate::config::EmailConfig;
use anyhow::{Context, Result, anyhow};
use async_trait::async_trait;
use html_escape::decode_html_entities;
use imap::{ClientBuilder, ConnectionMode};
use lettre::message::header::{InReplyTo, References};
use lettre::transport::smtp::authentication::Credentials;
use lettre::{Message, SmtpTransport, Transport};
use mailparse::{DispositionType, MailAddr, MailHeaderMap, ParsedMail, addrparse, parse_mail};
use regex::Regex;
use serde_json::{Map, Value};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

const MAX_PROCESSED_UIDS: usize = 100_000;

#[derive(Debug, Clone)]
struct InboundEmail {
    sender: String,
    subject: String,
    message_id: String,
    date_value: String,
    content: String,
    uid: String,
}

pub struct EmailChannel {
    config: EmailConfig,
    bus: Arc<MessageBus>,
    running: AtomicBool,
    last_subject_by_chat: Mutex<HashMap<String, String>>,
    last_message_id_by_chat: Mutex<HashMap<String, String>>,
    processed_uids: Mutex<HashSet<String>>,
}

impl EmailChannel {
    pub fn new(config: EmailConfig, bus: Arc<MessageBus>) -> Self {
        Self {
            config,
            bus,
            running: AtomicBool::new(false),
            last_subject_by_chat: Mutex::new(HashMap::new()),
            last_message_id_by_chat: Mutex::new(HashMap::new()),
            processed_uids: Mutex::new(HashSet::new()),
        }
    }

    fn validate_config(&self) -> Result<()> {
        let mut missing = Vec::new();
        if self.config.imap_host.trim().is_empty() {
            missing.push("imapHost");
        }
        if self.config.imap_username.trim().is_empty() {
            missing.push("imapUsername");
        }
        if self.config.imap_password.trim().is_empty() {
            missing.push("imapPassword");
        }
        if self.config.smtp_host.trim().is_empty() {
            missing.push("smtpHost");
        }
        if self.config.smtp_username.trim().is_empty() {
            missing.push("smtpUsername");
        }
        if self.config.smtp_password.trim().is_empty() {
            missing.push("smtpPassword");
        }

        if missing.is_empty() {
            Ok(())
        } else {
            Err(anyhow!(
                "email channel not configured, missing: {}",
                missing.join(", ")
            ))
        }
    }

    fn reply_subject(&self, base_subject: &str) -> String {
        let subject = if base_subject.trim().is_empty() {
            "nanobot reply"
        } else {
            base_subject.trim()
        };
        if subject.to_lowercase().starts_with("re:") {
            return subject.to_string();
        }
        let prefix = if self.config.subject_prefix.is_empty() {
            "Re: "
        } else {
            &self.config.subject_prefix
        };
        format!("{prefix}{subject}")
    }

    fn html_to_text(raw_html: &str) -> String {
        let br_re = Regex::new(r"(?i)<\s*br\s*/?>").expect("valid html br regex");
        let p_end_re = Regex::new(r"(?i)<\s*/\s*p\s*>").expect("valid html p regex");
        let tag_re = Regex::new(r"(?is)<[^>]+>").expect("valid html tag regex");
        let text = br_re.replace_all(raw_html, "\n");
        let text = p_end_re.replace_all(&text, "\n");
        let text = tag_re.replace_all(&text, "");
        decode_html_entities(&text).to_string()
    }

    fn extract_sender(from_header: &str) -> String {
        if from_header.trim().is_empty() {
            return String::new();
        }

        if let Ok(parsed) = addrparse(from_header) {
            for addr in parsed.into_inner() {
                match addr {
                    MailAddr::Single(s) => return s.addr.trim().to_lowercase(),
                    MailAddr::Group(g) => {
                        if let Some(first) = g.addrs.first() {
                            return first.addr.trim().to_lowercase();
                        }
                    }
                }
            }
        }

        let raw = from_header.trim();
        if let (Some(start), Some(end)) = (raw.find('<'), raw.rfind('>'))
            && start < end
        {
            return raw[start + 1..end].trim().to_lowercase();
        }
        raw.to_lowercase()
    }

    fn extract_text_body(parsed: &ParsedMail<'_>) -> String {
        if !parsed.subparts.is_empty() {
            let mut plain_parts = Vec::new();
            let mut html_parts = Vec::new();
            for part in &parsed.subparts {
                if part.get_content_disposition().disposition == DispositionType::Attachment {
                    continue;
                }
                let payload = part.get_body().unwrap_or_default();
                let mime = part.ctype.mimetype.to_lowercase();
                if mime == "text/plain" {
                    plain_parts.push(payload);
                } else if mime == "text/html" {
                    html_parts.push(payload);
                }
            }
            if !plain_parts.is_empty() {
                return plain_parts.join("\n\n").trim().to_string();
            }
            if !html_parts.is_empty() {
                return Self::html_to_text(&html_parts.join("\n\n"))
                    .trim()
                    .to_string();
            }
            return String::new();
        }

        let payload = parsed.get_body().unwrap_or_default();
        if parsed.ctype.mimetype.eq_ignore_ascii_case("text/html") {
            Self::html_to_text(&payload).trim().to_string()
        } else {
            payload.trim().to_string()
        }
    }

    fn fetch_new_messages(&self) -> Result<Vec<InboundEmail>> {
        self.fetch_messages("UNSEEN", self.config.mark_seen, true, 0)
    }

    fn fetch_messages(
        &self,
        query: &str,
        mark_seen: bool,
        dedupe: bool,
        limit: usize,
    ) -> Result<Vec<InboundEmail>> {
        let mut client = ClientBuilder::new(self.config.imap_host.as_str(), self.config.imap_port);
        client = if self.config.imap_use_ssl {
            client.mode(ConnectionMode::Tls)
        } else {
            client.mode(ConnectionMode::Plaintext)
        };
        let imap_client = client
            .connect()
            .context("failed to connect to IMAP server")?;
        let mut session = imap_client
            .login(
                self.config.imap_username.as_str(),
                self.config.imap_password.as_str(),
            )
            .map_err(|(err, _)| anyhow!("failed to login IMAP: {err}"))?;

        let result = (|| -> Result<Vec<InboundEmail>> {
            let mailbox = if self.config.imap_mailbox.trim().is_empty() {
                "INBOX"
            } else {
                self.config.imap_mailbox.as_str()
            };
            session
                .select(mailbox)
                .with_context(|| format!("failed to select mailbox {mailbox}"))?;

            let mut ids: Vec<u32> = session.search(query)?.into_iter().collect();
            ids.sort_unstable();
            if limit > 0 && ids.len() > limit {
                ids = ids[ids.len() - limit..].to_vec();
            }

            let mut messages = Vec::new();
            for seq_id in ids {
                let fetches = session.fetch(seq_id.to_string(), "(BODY.PEEK[] UID)")?;
                for fetch in fetches.iter() {
                    let Some(raw_bytes) = fetch.body() else {
                        continue;
                    };
                    let uid = fetch.uid.map(|u| u.to_string()).unwrap_or_default();

                    if dedupe && !uid.is_empty() {
                        let processed = self.processed_uids.lock().expect("poisoned mutex");
                        if processed.contains(&uid) {
                            continue;
                        }
                    }

                    let parsed = parse_mail(raw_bytes).context("failed to parse email body")?;
                    let sender = Self::extract_sender(
                        &parsed.headers.get_first_value("From").unwrap_or_default(),
                    );
                    if sender.is_empty() {
                        continue;
                    }

                    let subject = parsed
                        .headers
                        .get_first_value("Subject")
                        .unwrap_or_default();
                    let date_value = parsed.headers.get_first_value("Date").unwrap_or_default();
                    let message_id = parsed
                        .headers
                        .get_first_value("Message-ID")
                        .unwrap_or_default()
                        .trim()
                        .to_string();
                    let mut body = Self::extract_text_body(&parsed);
                    if body.is_empty() {
                        body = "(empty email body)".to_string();
                    }
                    let body = body
                        .chars()
                        .take(self.config.max_body_chars)
                        .collect::<String>();
                    let content = format!(
                        "Email received.\nFrom: {sender}\nSubject: {subject}\nDate: {date_value}\n\n{body}"
                    );

                    messages.push(InboundEmail {
                        sender,
                        subject,
                        message_id,
                        date_value,
                        content,
                        uid: uid.clone(),
                    });

                    if dedupe && !uid.is_empty() {
                        let mut processed = self.processed_uids.lock().expect("poisoned mutex");
                        processed.insert(uid.clone());
                        if processed.len() > MAX_PROCESSED_UIDS {
                            processed.clear();
                        }
                    }

                    if mark_seen {
                        let _ = session.store(seq_id.to_string(), "+FLAGS (\\Seen)");
                    }
                }
            }
            Ok(messages)
        })();

        let _ = session.logout();
        result
    }

    fn smtp_send(&self, email_msg: Message) -> Result<()> {
        let creds = Credentials::new(
            self.config.smtp_username.clone(),
            self.config.smtp_password.clone(),
        );
        let builder = if self.config.smtp_use_ssl {
            SmtpTransport::relay(self.config.smtp_host.as_str())?.port(self.config.smtp_port)
        } else if self.config.smtp_use_tls {
            SmtpTransport::starttls_relay(self.config.smtp_host.as_str())?
                .port(self.config.smtp_port)
        } else {
            SmtpTransport::builder_dangerous(self.config.smtp_host.as_str())
                .port(self.config.smtp_port)
        };
        let sender = builder.credentials(creds).build();
        sender
            .send(&email_msg)
            .context("failed to send SMTP message")?;
        Ok(())
    }
}

#[async_trait]
impl Channel for EmailChannel {
    fn name(&self) -> &str {
        "email"
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
        if !self.config.consent_granted {
            eprintln!(
                "Email channel disabled: consent_granted=false. Grant explicit permission before mailbox access."
            );
            return Ok(());
        }
        if let Err(err) = self.validate_config() {
            eprintln!("{err}");
            return Ok(());
        }

        self.running.store(true, Ordering::Relaxed);
        let poll_seconds = self.config.poll_interval_seconds.max(5);
        while self.running.load(Ordering::Relaxed) {
            match self.fetch_new_messages() {
                Ok(inbound_items) => {
                    for item in inbound_items {
                        if !item.subject.is_empty() {
                            self.last_subject_by_chat
                                .lock()
                                .expect("poisoned mutex")
                                .insert(item.sender.clone(), item.subject.clone());
                        }
                        if !item.message_id.is_empty() {
                            self.last_message_id_by_chat
                                .lock()
                                .expect("poisoned mutex")
                                .insert(item.sender.clone(), item.message_id.clone());
                        }
                        let mut metadata = Map::new();
                        metadata.insert("message_id".to_string(), Value::String(item.message_id));
                        metadata.insert("subject".to_string(), Value::String(item.subject));
                        metadata.insert("date".to_string(), Value::String(item.date_value));
                        metadata.insert(
                            "sender_email".to_string(),
                            Value::String(item.sender.clone()),
                        );
                        metadata.insert("uid".to_string(), Value::String(item.uid));

                        let _ = self
                            .handle_message(
                                item.sender.clone(),
                                item.sender,
                                item.content,
                                Vec::new(),
                                metadata,
                            )
                            .await;
                    }
                }
                Err(err) => {
                    eprintln!("email polling error: {err}");
                }
            }

            tokio::time::sleep(std::time::Duration::from_secs(poll_seconds)).await;
        }
        Ok(())
    }

    async fn stop(&self) -> Result<()> {
        self.running.store(false, Ordering::Relaxed);
        Ok(())
    }

    async fn send(&self, msg: &OutboundMessage) -> Result<()> {
        if !self.config.consent_granted {
            eprintln!("skip email send: consent_granted=false");
            return Ok(());
        }

        let force_send = msg
            .metadata
            .get("force_send")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if !self.config.auto_reply_enabled && !force_send {
            return Ok(());
        }
        if self.config.smtp_host.trim().is_empty() {
            eprintln!("email channel SMTP host not configured");
            return Ok(());
        }

        let to_addr = msg.chat_id.trim();
        if to_addr.is_empty() {
            return Ok(());
        }

        let base_subject = self
            .last_subject_by_chat
            .lock()
            .expect("poisoned mutex")
            .get(to_addr)
            .cloned()
            .unwrap_or_else(|| "nanobot reply".to_string());
        let mut subject = self.reply_subject(&base_subject);
        if let Some(s) = msg.metadata.get("subject").and_then(Value::as_str)
            && !s.trim().is_empty()
        {
            subject = s.trim().to_string();
        }

        let from_addr = if self.config.from_address.trim().is_empty() {
            if !self.config.smtp_username.trim().is_empty() {
                self.config.smtp_username.as_str()
            } else {
                self.config.imap_username.as_str()
            }
        } else {
            self.config.from_address.as_str()
        };

        let mut builder = Message::builder()
            .from(from_addr.parse().context("invalid from email address")?)
            .to(to_addr.parse().context("invalid recipient email address")?)
            .subject(subject);

        if let Some(in_reply_to) = self
            .last_message_id_by_chat
            .lock()
            .expect("poisoned mutex")
            .get(to_addr)
            .cloned()
            .filter(|v| !v.trim().is_empty())
        {
            builder = builder.header(InReplyTo::from(in_reply_to.clone()));
            builder = builder.header(References::from(in_reply_to));
        }

        let email_msg = builder.body(msg.content.clone())?;
        self.smtp_send(email_msg)
    }
}

#[cfg(test)]
mod tests {
    use super::EmailChannel;
    use crate::bus::MessageBus;
    use crate::config::EmailConfig;
    use std::sync::Arc;

    fn test_config() -> EmailConfig {
        EmailConfig {
            enabled: true,
            consent_granted: true,
            imap_host: "imap.example.com".to_string(),
            imap_port: 993,
            imap_username: "bot@example.com".to_string(),
            imap_password: "secret".to_string(),
            imap_mailbox: "INBOX".to_string(),
            imap_use_ssl: true,
            smtp_host: "smtp.example.com".to_string(),
            smtp_port: 587,
            smtp_username: "bot@example.com".to_string(),
            smtp_password: "secret".to_string(),
            smtp_use_tls: true,
            smtp_use_ssl: false,
            from_address: "bot@example.com".to_string(),
            auto_reply_enabled: true,
            poll_interval_seconds: 30,
            mark_seen: true,
            max_body_chars: 12_000,
            subject_prefix: "Re: ".to_string(),
            allow_from: Vec::new(),
        }
    }

    #[test]
    fn reply_subject_keeps_existing_re_prefix() {
        let channel = EmailChannel::new(test_config(), Arc::new(MessageBus::new(4)));
        assert_eq!(channel.reply_subject("Re: status"), "Re: status");
        assert_eq!(channel.reply_subject(""), "Re: nanobot reply");
    }

    #[test]
    fn html_to_text_converts_basic_markup() {
        let out = EmailChannel::html_to_text("<p>Hello<br>world</p>");
        assert!(out.contains("Hello"));
        assert!(out.contains("world"));
    }

    #[test]
    fn extract_sender_prefers_address() {
        let sender = EmailChannel::extract_sender("Alice <alice@example.com>");
        assert_eq!(sender, "alice@example.com");
    }
}
