use crate::memory::MemoryStore;
use crate::skills::SkillsLoader;
use chrono::Local;
use serde_json::{Value, json};
use std::path::PathBuf;

pub struct ContextBuilder {
    workspace: PathBuf,
    memory: MemoryStore,
    skills: SkillsLoader,
}

impl ContextBuilder {
    pub fn new(workspace: PathBuf) -> anyhow::Result<Self> {
        let memory = MemoryStore::new(workspace.clone())?;
        let skills = SkillsLoader::new(workspace.clone(), None);
        Ok(Self {
            workspace,
            memory,
            skills,
        })
    }

    pub fn build_system_prompt(&self, skill_names: Option<&[String]>) -> String {
        let mut parts = Vec::new();

        let now = Local::now().format("%Y-%m-%d %H:%M (%A)").to_string();
        let runtime = format!("{} {}", std::env::consts::OS, std::env::consts::ARCH);
        let workspace = self.workspace.display().to_string();
        parts.push(format!(
            "# nanobot-rs\n\nYou are nanobot, a helpful AI assistant.\n\n## Current Time\n{now}\n\n## Runtime\n{runtime}\n\n## Workspace\n{workspace}\n- Memory files: {workspace}/memory/MEMORY.md\n- Daily notes: {workspace}/memory/YYYY-MM-DD.md\n\nIMPORTANT: Respond directly in text for normal chat.\nOnly use the 'message' tool for proactive channel messages."
        ));

        let bootstrap_files = ["AGENTS.md", "SOUL.md", "USER.md", "TOOLS.md", "IDENTITY.md"];
        let mut bootstrap_parts = Vec::new();
        for filename in bootstrap_files {
            let path = self.workspace.join(filename);
            if let Ok(content) = std::fs::read_to_string(&path) {
                bootstrap_parts.push(format!("## {filename}\n\n{content}"));
            }
        }
        if !bootstrap_parts.is_empty() {
            parts.push(bootstrap_parts.join("\n\n"));
        }

        let memory_context = self.memory.get_memory_context();
        if !memory_context.is_empty() {
            parts.push(format!("# Memory\n\n{memory_context}"));
        }

        let always_skills = self.skills.get_always_skills();
        if !always_skills.is_empty() {
            let content = self.skills.load_skills_for_context(&always_skills);
            if !content.is_empty() {
                parts.push(format!("# Active Skills\n\n{content}"));
            }
        }

        if let Some(skill_names) = skill_names {
            if !skill_names.is_empty() {
                let content = self.skills.load_skills_for_context(skill_names);
                if !content.is_empty() {
                    parts.push(format!("# Requested Skills\n\n{content}"));
                }
            }
        }

        let summary = self.skills.build_skills_summary();
        if !summary.is_empty() {
            parts.push(format!(
                "# Skills\n\nThe following skills extend your capabilities. To use a skill, read its SKILL.md file using the read_file tool.\n\n{summary}"
            ));
        }

        parts.join("\n\n---\n\n")
    }

    pub fn build_messages(
        &self,
        history: &[Value],
        current_message: &str,
        skill_names: Option<&[String]>,
        channel: Option<&str>,
        chat_id: Option<&str>,
    ) -> Vec<Value> {
        let mut system_prompt = self.build_system_prompt(skill_names);
        if let (Some(channel), Some(chat_id)) = (channel, chat_id) {
            system_prompt.push_str(&format!(
                "\n\n## Current Session\nChannel: {channel}\nChat ID: {chat_id}"
            ));
        }

        let mut messages = Vec::new();
        messages.push(json!({
            "role": "system",
            "content": system_prompt,
        }));
        messages.extend(history.iter().cloned());
        messages.push(json!({
            "role": "user",
            "content": current_message,
        }));
        messages
    }

    pub fn add_tool_result(
        &self,
        messages: &mut Vec<Value>,
        tool_call_id: &str,
        tool_name: &str,
        result: &str,
    ) {
        messages.push(json!({
            "role": "tool",
            "tool_call_id": tool_call_id,
            "name": tool_name,
            "content": result,
        }));
    }

    pub fn add_assistant_message(
        &self,
        messages: &mut Vec<Value>,
        content: Option<&str>,
        tool_calls: Option<Vec<Value>>,
    ) {
        let mut msg = json!({
            "role": "assistant",
            "content": content.unwrap_or(""),
        });
        if let Some(calls) = tool_calls {
            msg["tool_calls"] = Value::Array(calls);
        }
        messages.push(msg);
    }
}
