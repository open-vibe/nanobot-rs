use crate::bus::{InboundMessage, MessageBus, OutboundMessage};
use crate::channels::base::{Channel, is_allowed_sender};
use crate::config::MochatConfig;
use crate::pairing::{issue_pairing, pairing_prompt};
use crate::utils::get_data_path;
use anyhow::{Result, anyhow};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde_json::{Map, Value, json};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio::time::{Duration, sleep};

const MAX_SEEN_MESSAGE_IDS: usize = 2000;

#[derive(Default)]
struct MochatShared {
    cursors: Mutex<HashMap<String, i64>>,
    session_set: Mutex<HashSet<String>>,
    panel_set: Mutex<HashSet<String>>,
    cold_sessions: Mutex<HashSet<String>>,
    session_by_converse: Mutex<HashMap<String, String>>,
    seen_set: Mutex<HashMap<String, HashSet<String>>>,
    seen_queue: Mutex<HashMap<String, VecDeque<String>>>,
}

#[derive(Clone)]
struct Runtime {
    config: MochatConfig,
    bus: Arc<MessageBus>,
    client: reqwest::Client,
    running: Arc<AtomicBool>,
    shared: Arc<MochatShared>,
    auto_discover_sessions: bool,
    auto_discover_panels: bool,
    session_tasks: Arc<Mutex<HashMap<String, JoinHandle<()>>>>,
    panel_tasks: Arc<Mutex<HashMap<String, JoinHandle<()>>>>,
}

pub struct MochatChannel {
    config: MochatConfig,
    bus: Arc<MessageBus>,
    running: Arc<AtomicBool>,
    client: reqwest::Client,
    shared: Arc<MochatShared>,
    cursor_path: PathBuf,
    initial_sessions: Vec<String>,
    initial_panels: Vec<String>,
    auto_discover_sessions: bool,
    auto_discover_panels: bool,
    refresh_task: Arc<Mutex<Option<JoinHandle<()>>>>,
    session_tasks: Arc<Mutex<HashMap<String, JoinHandle<()>>>>,
    panel_tasks: Arc<Mutex<HashMap<String, JoinHandle<()>>>>,
}

impl MochatChannel {
    pub fn new(config: MochatConfig, bus: Arc<MessageBus>) -> Self {
        let (sessions, auto_discover_sessions) = normalize_id_list(&config.sessions);
        let (panels, auto_discover_panels) = normalize_id_list(&config.panels);
        let cursor_path = get_data_path()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join("mochat")
            .join("session_cursors.json");
        Self {
            config,
            bus,
            running: Arc::new(AtomicBool::new(false)),
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .unwrap_or_else(|_| reqwest::Client::new()),
            shared: Arc::new(MochatShared::default()),
            cursor_path,
            initial_sessions: sessions,
            initial_panels: panels,
            auto_discover_sessions,
            auto_discover_panels,
            refresh_task: Arc::new(Mutex::new(None)),
            session_tasks: Arc::new(Mutex::new(HashMap::new())),
            panel_tasks: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    fn runtime(&self) -> Runtime {
        Runtime {
            config: self.config.clone(),
            bus: self.bus.clone(),
            client: self.client.clone(),
            running: self.running.clone(),
            shared: self.shared.clone(),
            auto_discover_sessions: self.auto_discover_sessions,
            auto_discover_panels: self.auto_discover_panels,
            session_tasks: self.session_tasks.clone(),
            panel_tasks: self.panel_tasks.clone(),
        }
    }
}

#[async_trait]
impl Channel for MochatChannel {
    fn name(&self) -> &str {
        "mochat"
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
        if self.config.claw_token.trim().is_empty() {
            eprintln!("Mochat claw_token not configured");
            return Ok(());
        }
        self.running.store(true, Ordering::Relaxed);
        load_cursors(&self.shared, &self.cursor_path).await;
        seed_targets(&self.shared, &self.initial_sessions, &self.initial_panels).await;
        let rt = self.runtime();
        refresh_targets(&rt).await;
        ensure_workers(&rt).await;

        let rt_refresh = rt.clone();
        *self.refresh_task.lock().await = Some(tokio::spawn(async move {
            while rt_refresh.running.load(Ordering::Relaxed) {
                sleep(Duration::from_millis(
                    rt_refresh.config.refresh_interval_ms.max(1000),
                ))
                .await;
                refresh_targets(&rt_refresh).await;
                ensure_workers(&rt_refresh).await;
            }
        }));

        while self.running.load(Ordering::Relaxed) {
            sleep(Duration::from_secs(1)).await;
        }
        Ok(())
    }

    async fn stop(&self) -> Result<()> {
        self.running.store(false, Ordering::Relaxed);
        if let Some(task) = self.refresh_task.lock().await.take() {
            task.abort();
        }
        for (_, task) in self.session_tasks.lock().await.drain() {
            task.abort();
        }
        for (_, task) in self.panel_tasks.lock().await.drain() {
            task.abort();
        }
        save_cursors(&self.shared, &self.cursor_path).await;
        Ok(())
    }

    async fn send(&self, msg: &OutboundMessage) -> Result<()> {
        let content = if msg.media.is_empty() {
            msg.content.trim().to_string()
        } else {
            let mut parts = vec![msg.content.trim().to_string()];
            parts.extend(msg.media.iter().map(|m| m.trim().to_string()));
            parts.retain(|s| !s.is_empty());
            parts.join("\n")
        };
        if content.is_empty() {
            return Ok(());
        }
        let target = resolve_target(&msg.chat_id);
        if target.is_empty() {
            return Ok(());
        }

        let is_panel = {
            let panel_set = self.shared.panel_set.lock().await;
            panel_set.contains(&target) || !target.starts_with("session_")
        };
        let path = if is_panel {
            "/api/claw/groups/panels/send"
        } else {
            "/api/claw/sessions/send"
        };
        let id_key = if is_panel { "panelId" } else { "sessionId" };
        let mut body = json!({ id_key: target, "content": content });
        if let Some(reply_to) = &msg.reply_to {
            body["replyTo"] = Value::String(reply_to.clone());
        }
        if is_panel {
            if let Some(group_id) = msg.metadata.get("group_id").and_then(Value::as_str) {
                if !group_id.trim().is_empty() {
                    body["groupId"] = Value::String(group_id.trim().to_string());
                }
            }
        }
        let _ = post_json(&self.client, &self.config, path, &body).await?;
        Ok(())
    }
}

async fn refresh_targets(rt: &Runtime) {
    if rt.auto_discover_sessions {
        if let Ok(resp) = post_json(
            &rt.client,
            &rt.config,
            "/api/claw/sessions/list",
            &json!({}),
        )
        .await
        {
            if let Some(items) = resp.get("sessions").and_then(Value::as_array) {
                let mut session_set = rt.shared.session_set.lock().await;
                let cursors = rt.shared.cursors.lock().await;
                let mut cold = rt.shared.cold_sessions.lock().await;
                let mut converse = rt.shared.session_by_converse.lock().await;
                for item in items {
                    let Some(obj) = item.as_object() else {
                        continue;
                    };
                    let sid = str_field(obj, &["sessionId"]);
                    if sid.is_empty() {
                        continue;
                    }
                    if session_set.insert(sid.clone()) && !cursors.contains_key(&sid) {
                        cold.insert(sid.clone());
                    }
                    let cid = str_field(obj, &["converseId"]);
                    if !cid.is_empty() {
                        converse.insert(cid, sid);
                    }
                }
            }
        }
    }
    if rt.auto_discover_panels {
        if let Ok(resp) =
            post_json(&rt.client, &rt.config, "/api/claw/groups/get", &json!({})).await
        {
            if let Some(items) = resp.get("panels").and_then(Value::as_array) {
                let mut panel_set = rt.shared.panel_set.lock().await;
                for item in items {
                    let Some(obj) = item.as_object() else {
                        continue;
                    };
                    if obj.get("type").and_then(Value::as_i64).unwrap_or(0) != 0 {
                        continue;
                    }
                    let pid = str_field(obj, &["id", "_id"]);
                    if !pid.is_empty() {
                        panel_set.insert(pid);
                    }
                }
            }
        }
    }
}

async fn ensure_workers(rt: &Runtime) {
    let sessions: Vec<String> = rt.shared.session_set.lock().await.iter().cloned().collect();
    let mut session_tasks = rt.session_tasks.lock().await;
    for sid in sessions {
        if session_tasks.get(&sid).is_some_and(|t| !t.is_finished()) {
            continue;
        }
        let rt_clone = rt.clone();
        let sid_clone = sid.clone();
        session_tasks.insert(
            sid,
            tokio::spawn(async move {
                session_worker(rt_clone, sid_clone).await;
            }),
        );
    }
    drop(session_tasks);

    let panels: Vec<String> = rt.shared.panel_set.lock().await.iter().cloned().collect();
    let mut panel_tasks = rt.panel_tasks.lock().await;
    for pid in panels {
        if panel_tasks.get(&pid).is_some_and(|t| !t.is_finished()) {
            continue;
        }
        let rt_clone = rt.clone();
        let pid_clone = pid.clone();
        panel_tasks.insert(
            pid,
            tokio::spawn(async move {
                panel_worker(rt_clone, pid_clone).await;
            }),
        );
    }
}

async fn session_worker(rt: Runtime, session_id: String) {
    while rt.running.load(Ordering::Relaxed) {
        let cursor = rt
            .shared
            .cursors
            .lock()
            .await
            .get(&session_id)
            .copied()
            .unwrap_or(0);
        let req = json!({
            "sessionId": session_id,
            "cursor": cursor,
            "timeoutMs": rt.config.watch_timeout_ms,
            "limit": rt.config.watch_limit,
        });
        if let Ok(payload) =
            post_json(&rt.client, &rt.config, "/api/claw/sessions/watch", &req).await
        {
            if let Some(c) = payload.get("cursor").and_then(Value::as_i64) {
                rt.shared.cursors.lock().await.insert(session_id.clone(), c);
            }
            if rt.shared.cold_sessions.lock().await.remove(&session_id) {
                continue;
            }
            if let Some(events) = payload.get("events").and_then(Value::as_array) {
                for event in events {
                    if event.get("type").and_then(Value::as_str) == Some("message.add") {
                        process_event(&rt, &session_id, event, "session").await;
                    }
                }
            }
        } else {
            sleep(Duration::from_millis(rt.config.retry_delay_ms.max(100))).await;
        }
    }
}

async fn panel_worker(rt: Runtime, panel_id: String) {
    while rt.running.load(Ordering::Relaxed) {
        let req = json!({
            "panelId": panel_id,
            "limit": rt.config.watch_limit.min(100).max(1),
        });
        if let Ok(resp) = post_json(
            &rt.client,
            &rt.config,
            "/api/claw/groups/panels/messages",
            &req,
        )
        .await
        {
            let group_id = resp
                .get("groupId")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            if let Some(messages) = resp.get("messages").and_then(Value::as_array) {
                for m in messages.iter().rev() {
                    let Some(obj) = m.as_object() else { continue };
                    let event = json!({
                        "type": "message.add",
                        "timestamp": obj.get("createdAt").cloned().unwrap_or(Value::String(Utc::now().to_rfc3339())),
                        "payload": {
                            "messageId": str_field(obj, &["messageId"]),
                            "author": str_field(obj, &["author"]),
                            "content": obj.get("content").cloned().unwrap_or(Value::Null),
                            "meta": obj.get("meta").cloned().unwrap_or_else(|| json!({})),
                            "groupId": group_id,
                            "converseId": panel_id,
                            "authorInfo": obj.get("authorInfo").cloned().unwrap_or_else(|| json!({})),
                        }
                    });
                    process_event(&rt, &panel_id, &event, "panel").await;
                }
            }
        }
        sleep(Duration::from_millis(
            rt.config.refresh_interval_ms.max(1000),
        ))
        .await;
    }
}

async fn process_event(rt: &Runtime, target_id: &str, event: &Value, target_kind: &str) {
    let Some(payload) = event.get("payload").and_then(Value::as_object) else {
        return;
    };
    let author = str_field(payload, &["author"]);
    if author.is_empty()
        || (!rt.config.agent_user_id.is_empty() && author == rt.config.agent_user_id)
    {
        return;
    }
    if !is_allowed_sender(&author, &rt.config.allow_from) {
        if let Ok(issue) = issue_pairing("mochat", &author, target_id) {
            let prompt = pairing_prompt(&issue);
            let _ = rt
                .bus
                .publish_outbound(OutboundMessage::new(
                    "mochat",
                    target_id.to_string(),
                    prompt,
                ))
                .await;
        }
        return;
    }
    let message_id = str_field(payload, &["messageId"]);
    if is_seen(
        &rt.shared,
        &format!("{target_kind}:{target_id}"),
        &message_id,
    )
    .await
    {
        return;
    }
    let body = normalize_content(payload.get("content"));
    if body.is_empty() {
        return;
    }
    let group_id = str_field(payload, &["groupId"]);
    if target_kind == "panel" && !group_id.is_empty() {
        let require = rt
            .config
            .groups
            .get(&group_id)
            .map(|r| r.require_mention)
            .or_else(|| rt.config.groups.get(target_id).map(|r| r.require_mention))
            .or_else(|| rt.config.groups.get("*").map(|r| r.require_mention))
            .unwrap_or(rt.config.mention.require_in_groups);
        if require && !resolve_was_mentioned(payload, &rt.config.agent_user_id) {
            return;
        }
    }

    let mut inbound = InboundMessage::new("mochat", author, target_id.to_string(), body);
    inbound
        .metadata
        .insert("message_id".to_string(), Value::String(message_id));
    inbound
        .metadata
        .insert("group_id".to_string(), Value::String(group_id));
    inbound.metadata.insert(
        "target_kind".to_string(),
        Value::String(target_kind.to_string()),
    );
    if let Some(ts) = parse_timestamp(event.get("timestamp")) {
        inbound
            .metadata
            .insert("timestamp".to_string(), Value::Number(ts.into()));
    }
    let _ = rt.bus.publish_inbound(inbound).await;
}

async fn is_seen(shared: &MochatShared, key: &str, message_id: &str) -> bool {
    if message_id.is_empty() {
        return false;
    }
    let mut seen_set_map = shared.seen_set.lock().await;
    let mut seen_queue_map = shared.seen_queue.lock().await;
    let set = seen_set_map.entry(key.to_string()).or_default();
    if set.contains(message_id) {
        return true;
    }
    set.insert(message_id.to_string());
    let queue = seen_queue_map.entry(key.to_string()).or_default();
    queue.push_back(message_id.to_string());
    while queue.len() > MAX_SEEN_MESSAGE_IDS {
        if let Some(old) = queue.pop_front() {
            set.remove(&old);
        }
    }
    false
}

async fn load_cursors(shared: &MochatShared, path: &PathBuf) {
    let Ok(raw) = std::fs::read_to_string(path) else {
        return;
    };
    let Ok(parsed) = serde_json::from_str::<Value>(&raw) else {
        return;
    };
    let Some(cursors) = parsed.get("cursors").and_then(Value::as_object) else {
        return;
    };
    let mut store = shared.cursors.lock().await;
    for (sid, cur) in cursors {
        if let Some(v) = cur.as_i64()
            && v >= 0
        {
            store.insert(sid.clone(), v);
        }
    }
}

async fn save_cursors(shared: &MochatShared, path: &PathBuf) {
    let data = json!({"schemaVersion": 1, "updatedAt": Utc::now().to_rfc3339(), "cursors": *shared.cursors.lock().await});
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(
        path,
        serde_json::to_string_pretty(&data).unwrap_or_else(|_| "{}".to_string()),
    );
}

async fn seed_targets(shared: &MochatShared, sessions: &[String], panels: &[String]) {
    let mut session_set = shared.session_set.lock().await;
    let mut panel_set = shared.panel_set.lock().await;
    let mut cold = shared.cold_sessions.lock().await;
    let cursors = shared.cursors.lock().await;
    for sid in sessions {
        session_set.insert(sid.clone());
        if !cursors.contains_key(sid) {
            cold.insert(sid.clone());
        }
    }
    for pid in panels {
        panel_set.insert(pid.clone());
    }
}

async fn post_json(
    client: &reqwest::Client,
    config: &MochatConfig,
    path: &str,
    payload: &Value,
) -> Result<Value> {
    let url = format!("{}{}", config.base_url.trim().trim_end_matches('/'), path);
    let response = client
        .post(url)
        .header("Content-Type", "application/json")
        .header("X-Claw-Token", config.claw_token.clone())
        .json(payload)
        .send()
        .await?;
    let status = response.status();
    let raw = response.text().await?;
    if !status.is_success() {
        return Err(anyhow!(
            "Mochat HTTP {status}: {}",
            raw.chars().take(200).collect::<String>()
        ));
    }
    let parsed: Value = serde_json::from_str(&raw).unwrap_or(Value::Null);
    if let Some(code) = parsed.get("code").and_then(Value::as_i64) {
        if code != 200 {
            let msg = parsed
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("request failed");
            return Err(anyhow!("Mochat API error: {msg} (code={code})"));
        }
        return Ok(parsed.get("data").cloned().unwrap_or_else(|| json!({})));
    }
    Ok(parsed)
}

fn normalize_id_list(values: &[String]) -> (Vec<String>, bool) {
    let mut out = HashSet::new();
    let mut auto = false;
    for v in values {
        let trimmed = v.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed == "*" {
            auto = true;
        } else {
            out.insert(trimmed.to_string());
        }
    }
    (out.into_iter().collect(), auto)
}

fn resolve_target(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    for prefix in ["mochat:", "group:", "channel:", "panel:"] {
        if trimmed.to_ascii_lowercase().starts_with(prefix) {
            return trimmed[prefix.len()..].trim().to_string();
        }
    }
    trimmed.to_string()
}

fn str_field(map: &Map<String, Value>, keys: &[&str]) -> String {
    for key in keys {
        if let Some(v) = map.get(*key).and_then(Value::as_str) {
            let trimmed = v.trim();
            if !trimmed.is_empty() {
                return trimmed.to_string();
            }
        }
    }
    String::new()
}

fn normalize_content(value: Option<&Value>) -> String {
    let Some(v) = value else { return String::new() };
    if let Some(s) = v.as_str() {
        return s.trim().to_string();
    }
    if v.is_null() {
        return String::new();
    }
    serde_json::to_string(v).unwrap_or_default()
}

fn resolve_was_mentioned(payload: &Map<String, Value>, agent_user_id: &str) -> bool {
    if agent_user_id.is_empty() {
        return false;
    }
    if let Some(meta) = payload.get("meta").and_then(Value::as_object) {
        if meta
            .get("mentioned")
            .and_then(Value::as_bool)
            .unwrap_or(false)
            || meta
                .get("wasMentioned")
                .and_then(Value::as_bool)
                .unwrap_or(false)
        {
            return true;
        }
    }
    let Some(content) = payload.get("content").and_then(Value::as_str) else {
        return false;
    };
    content.contains(&format!("<@{agent_user_id}>"))
        || content.contains(&format!("@{agent_user_id}"))
}

fn parse_timestamp(value: Option<&Value>) -> Option<i64> {
    let value = value?;
    if let Some(v) = value.as_i64() {
        return Some(v);
    }
    let s = value.as_str()?;
    DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.timestamp_millis())
        .ok()
}
