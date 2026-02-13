use crate::config::{Config, load_config, save_config};
use crate::utils::get_data_path;
use anyhow::{Result, anyhow};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

const EXPIRE_MS: i64 = 24 * 60 * 60 * 1000;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingPairing {
    pub channel: String,
    pub sender_id: String,
    pub chat_id: String,
    pub code: String,
    pub created_at_ms: i64,
    pub last_seen_at_ms: i64,
    pub request_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct PairingStore {
    pending: Vec<PendingPairing>,
}

#[derive(Debug, Clone)]
pub struct PairingIssue {
    pub code: String,
    pub is_new: bool,
}

fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

fn store_path() -> Result<PathBuf> {
    Ok(get_data_path()?.join("pairing").join("pending.json"))
}

fn load_store() -> Result<PairingStore> {
    let path = store_path()?;
    if !path.exists() {
        return Ok(PairingStore::default());
    }
    let raw = std::fs::read_to_string(path)?;
    let store = serde_json::from_str(&raw).unwrap_or_default();
    Ok(store)
}

fn save_store(store: &PairingStore) -> Result<()> {
    let path = store_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let text = serde_json::to_string_pretty(store)?;
    std::fs::write(path, text)?;
    Ok(())
}

fn cleanup_expired(store: &mut PairingStore) {
    let threshold = now_ms() - EXPIRE_MS;
    store
        .pending
        .retain(|entry| entry.last_seen_at_ms >= threshold);
}

fn new_code() -> String {
    Uuid::new_v4()
        .simple()
        .to_string()
        .chars()
        .take(6)
        .collect::<String>()
        .to_ascii_uppercase()
}

pub fn issue_pairing(channel: &str, sender_id: &str, chat_id: &str) -> Result<PairingIssue> {
    if channel.trim().is_empty() || sender_id.trim().is_empty() || chat_id.trim().is_empty() {
        return Err(anyhow!("channel/sender/chat cannot be empty"));
    }
    let mut store = load_store()?;
    cleanup_expired(&mut store);

    if let Some(entry) = store
        .pending
        .iter_mut()
        .find(|p| p.channel == channel && p.sender_id == sender_id)
    {
        entry.last_seen_at_ms = now_ms();
        entry.request_count = entry.request_count.saturating_add(1);
        let code = entry.code.clone();
        save_store(&store)?;
        return Ok(PairingIssue {
            code,
            is_new: false,
        });
    }

    let pending = PendingPairing {
        channel: channel.to_string(),
        sender_id: sender_id.to_string(),
        chat_id: chat_id.to_string(),
        code: new_code(),
        created_at_ms: now_ms(),
        last_seen_at_ms: now_ms(),
        request_count: 1,
    };
    let code = pending.code.clone();
    store.pending.push(pending);
    save_store(&store)?;
    Ok(PairingIssue { code, is_new: true })
}

pub fn list_pending() -> Result<Vec<PendingPairing>> {
    let mut store = load_store()?;
    cleanup_expired(&mut store);
    save_store(&store)?;
    store
        .pending
        .sort_by(|a, b| b.last_seen_at_ms.cmp(&a.last_seen_at_ms));
    Ok(store.pending)
}

fn channel_allowlist_mut<'a>(config: &'a mut Config, channel: &str) -> Option<&'a mut Vec<String>> {
    match channel {
        "telegram" => Some(&mut config.channels.telegram.allow_from),
        "discord" => Some(&mut config.channels.discord.allow_from),
        "whatsapp" => Some(&mut config.channels.whatsapp.allow_from),
        "feishu" => Some(&mut config.channels.feishu.allow_from),
        "dingtalk" => Some(&mut config.channels.dingtalk.allow_from),
        "email" => Some(&mut config.channels.email.allow_from),
        "mochat" => Some(&mut config.channels.mochat.allow_from),
        "qq" => Some(&mut config.channels.qq.allow_from),
        "slack" => Some(&mut config.channels.slack.dm.allow_from),
        _ => None,
    }
}

pub fn approve_pairing(channel: &str, code: &str) -> Result<PendingPairing> {
    let mut store = load_store()?;
    cleanup_expired(&mut store);
    let idx = store
        .pending
        .iter()
        .position(|p| p.channel == channel && p.code.eq_ignore_ascii_case(code))
        .ok_or_else(|| anyhow!("pending pairing not found for channel={channel}, code={code}"))?;
    let pending = store.pending.remove(idx);

    let mut config = load_config(None).unwrap_or_default();
    if channel == "slack" {
        config.channels.slack.dm.policy = "allowlist".to_string();
    }
    let allowlist = channel_allowlist_mut(&mut config, channel)
        .ok_or_else(|| anyhow!("channel '{channel}' does not support allowlist pairing"))?;

    if !allowlist.iter().any(|v| v == &pending.sender_id) {
        allowlist.push(pending.sender_id.clone());
    }
    save_config(&config, None)?;
    save_store(&store)?;
    Ok(pending)
}

pub fn reject_pairing(channel: &str, code: &str) -> Result<bool> {
    let mut store = load_store()?;
    cleanup_expired(&mut store);
    let before = store.pending.len();
    store
        .pending
        .retain(|p| !(p.channel == channel && p.code.eq_ignore_ascii_case(code)));
    let changed = store.pending.len() != before;
    if changed {
        save_store(&store)?;
    }
    Ok(changed)
}

pub fn pairing_prompt(issue: &PairingIssue) -> String {
    if issue.is_new {
        format!(
            "Access requires pairing.\nCode: {}\nOwner command: nanobot-rs pairing approve <channel> {}",
            issue.code, issue.code
        )
    } else {
        format!(
            "Pairing pending.\nCode: {}\nOwner command: nanobot-rs pairing approve <channel> {}",
            issue.code, issue.code
        )
    }
}
