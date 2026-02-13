use crate::utils::{get_data_path, safe_filename, timestamp};
use anyhow::{Context, Result};
use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub key: String,
    pub messages: Vec<Value>,
    pub created_at: DateTime<Local>,
    pub updated_at: DateTime<Local>,
    pub metadata: Map<String, Value>,
}

impl Session {
    pub fn new(key: impl Into<String>) -> Self {
        let now = Local::now();
        Self {
            key: key.into(),
            messages: Vec::new(),
            created_at: now,
            updated_at: now,
            metadata: Map::new(),
        }
    }

    pub fn add_message(&mut self, role: &str, content: &str) {
        self.add_message_with_tools(role, content, None);
    }

    pub fn add_message_with_tools(
        &mut self,
        role: &str,
        content: &str,
        tools_used: Option<&[String]>,
    ) {
        let mut message = json!({
            "role": role,
            "content": content,
            "timestamp": timestamp(),
        });
        if let Some(tools) = tools_used
            && !tools.is_empty()
        {
            message["tools_used"] = Value::Array(
                tools
                    .iter()
                    .map(|tool| Value::String(tool.clone()))
                    .collect(),
            );
        }
        self.messages.push(message);
        self.updated_at = Local::now();
    }

    fn to_llm_message(m: &Value) -> Value {
        json!({
            "role": m.get("role").and_then(Value::as_str).unwrap_or("user"),
            "content": m.get("content").and_then(Value::as_str).unwrap_or(""),
        })
    }

    pub fn get_history(&self, max_messages: usize) -> Vec<Value> {
        // Guard against model self-contamination:
        // only replay user-side history back into context.
        let user_messages = self
            .messages
            .iter()
            .filter(|m| m.get("role").and_then(Value::as_str) == Some("user"))
            .collect::<Vec<_>>();

        let start = user_messages.len().saturating_sub(max_messages);
        user_messages[start..]
            .iter()
            .map(|m| Self::to_llm_message(m))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::Session;

    #[test]
    fn history_excludes_assistant_messages() {
        let mut session = Session::new("cli:test");
        session.add_message("user", "u1");
        session.add_message("assistant", "a1");
        session.add_message("user", "u2");

        let history = session.get_history(10);
        assert_eq!(history.len(), 2);
        assert_eq!(history[0]["role"], "user");
        assert_eq!(history[0]["content"], "u1");
        assert_eq!(history[1]["role"], "user");
        assert_eq!(history[1]["content"], "u2");
    }
}

pub struct SessionManager {
    sessions_dir: PathBuf,
    cache: Mutex<HashMap<String, Session>>,
}

impl SessionManager {
    pub fn new() -> Result<Self> {
        let sessions_dir = get_data_path()?.join("sessions");
        std::fs::create_dir_all(&sessions_dir)?;
        Ok(Self {
            sessions_dir,
            cache: Mutex::new(HashMap::new()),
        })
    }

    fn session_path(&self, key: &str) -> PathBuf {
        let safe_key = safe_filename(&key.replace(':', "_"));
        self.sessions_dir.join(format!("{safe_key}.jsonl"))
    }

    pub fn get_or_create(&self, key: &str) -> Session {
        if let Some(cached) = self.cache.lock().ok().and_then(|c| c.get(key).cloned()) {
            return cached;
        }

        let loaded = self.load(key).unwrap_or_else(|_| Session::new(key));
        if let Ok(mut cache) = self.cache.lock() {
            cache.insert(key.to_string(), loaded.clone());
        }
        loaded
    }

    pub fn save(&self, session: &Session) -> Result<()> {
        let path = self.session_path(&session.key);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let mut lines = Vec::new();
        lines.push(serde_json::to_string(&json!({
            "_type": "metadata",
            "created_at": session.created_at.to_rfc3339(),
            "updated_at": session.updated_at.to_rfc3339(),
            "metadata": session.metadata,
        }))?);

        for msg in &session.messages {
            lines.push(serde_json::to_string(msg)?);
        }
        std::fs::write(&path, format!("{}\n", lines.join("\n")))?;

        if let Ok(mut cache) = self.cache.lock() {
            cache.insert(session.key.clone(), session.clone());
        }
        Ok(())
    }

    pub fn delete(&self, key: &str) -> bool {
        if let Ok(mut cache) = self.cache.lock() {
            cache.remove(key);
        }
        let path = self.session_path(key);
        if path.exists() {
            std::fs::remove_file(path).is_ok()
        } else {
            false
        }
    }

    fn load(&self, key: &str) -> Result<Session> {
        let path = self.session_path(key);
        let content = std::fs::read_to_string(&path)
            .with_context(|| format!("failed reading {}", path.display()))?;

        let mut session = Session::new(key);
        for line in content
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
        {
            let value: Value = serde_json::from_str(line)?;
            if value.get("_type").and_then(Value::as_str) == Some("metadata") {
                if let Some(raw) = value.get("created_at").and_then(Value::as_str) {
                    if let Ok(ts) = DateTime::parse_from_rfc3339(raw) {
                        session.created_at = ts.with_timezone(&Local);
                    }
                }
                if let Some(raw) = value.get("updated_at").and_then(Value::as_str) {
                    if let Ok(ts) = DateTime::parse_from_rfc3339(raw) {
                        session.updated_at = ts.with_timezone(&Local);
                    }
                }
                session.metadata = value
                    .get("metadata")
                    .and_then(Value::as_object)
                    .cloned()
                    .unwrap_or_default();
            } else {
                session.messages.push(value);
            }
        }
        Ok(session)
    }
}
