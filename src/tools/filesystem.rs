use crate::tools::base::Tool;
use anyhow::{Result, anyhow};
use async_trait::async_trait;
use serde_json::{Map, Value, json};
use std::path::{Component, Path, PathBuf};

fn normalize_path(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

fn resolve_path(path: &str, allowed_dir: Option<&PathBuf>) -> Result<PathBuf> {
    let input = PathBuf::from(path);
    let absolute = if input.is_absolute() {
        input
    } else {
        std::env::current_dir()?.join(input)
    };
    let resolved = normalize_path(&absolute);

    if let Some(allowed) = allowed_dir {
        let allowed = normalize_path(allowed);
        if !resolved.starts_with(&allowed) {
            return Err(anyhow!(
                "Path {path} is outside allowed directory {}",
                allowed.display()
            ));
        }
    }
    Ok(resolved)
}

fn get_required_string<'a>(params: &'a Map<String, Value>, key: &str) -> Result<&'a str> {
    params
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("missing required string field: {key}"))
}

pub struct ReadFileTool {
    allowed_dir: Option<PathBuf>,
}

impl ReadFileTool {
    pub fn new(allowed_dir: Option<PathBuf>) -> Self {
        Self { allowed_dir }
    }
}

#[async_trait]
impl Tool for ReadFileTool {
    fn name(&self) -> &str {
        "read_file"
    }

    fn description(&self) -> &str {
        "Read the contents of a file at the given path."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "The file path to read" }
            },
            "required": ["path"]
        })
    }

    async fn execute(&self, params: &Map<String, Value>) -> Result<String> {
        let path = get_required_string(params, "path")?;
        let resolved = resolve_path(path, self.allowed_dir.as_ref())?;

        if !resolved.exists() {
            return Ok(format!("Error: File not found: {path}"));
        }
        if !resolved.is_file() {
            return Ok(format!("Error: Not a file: {path}"));
        }

        let content = tokio::fs::read_to_string(&resolved).await?;
        Ok(content)
    }
}

pub struct WriteFileTool {
    allowed_dir: Option<PathBuf>,
}

impl WriteFileTool {
    pub fn new(allowed_dir: Option<PathBuf>) -> Self {
        Self { allowed_dir }
    }
}

#[async_trait]
impl Tool for WriteFileTool {
    fn name(&self) -> &str {
        "write_file"
    }

    fn description(&self) -> &str {
        "Write content to a file at the given path. Creates parent directories if needed."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "The file path to write to" },
                "content": { "type": "string", "description": "The content to write" }
            },
            "required": ["path", "content"]
        })
    }

    async fn execute(&self, params: &Map<String, Value>) -> Result<String> {
        let path = get_required_string(params, "path")?;
        let content = get_required_string(params, "content")?;
        let resolved = resolve_path(path, self.allowed_dir.as_ref())?;

        if let Some(parent) = resolved.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        tokio::fs::write(&resolved, content).await?;
        Ok(format!(
            "Successfully wrote {} bytes to {path}",
            content.len()
        ))
    }
}

pub struct EditFileTool {
    allowed_dir: Option<PathBuf>,
}

impl EditFileTool {
    pub fn new(allowed_dir: Option<PathBuf>) -> Self {
        Self { allowed_dir }
    }
}

#[async_trait]
impl Tool for EditFileTool {
    fn name(&self) -> &str {
        "edit_file"
    }

    fn description(&self) -> &str {
        "Edit a file by replacing old_text with new_text. old_text must appear exactly once."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "The file path to edit" },
                "old_text": { "type": "string", "description": "The exact text to find and replace" },
                "new_text": { "type": "string", "description": "The replacement text" }
            },
            "required": ["path", "old_text", "new_text"]
        })
    }

    async fn execute(&self, params: &Map<String, Value>) -> Result<String> {
        let path = get_required_string(params, "path")?;
        let old_text = get_required_string(params, "old_text")?;
        let new_text = get_required_string(params, "new_text")?;
        let resolved = resolve_path(path, self.allowed_dir.as_ref())?;

        if !resolved.exists() {
            return Ok(format!("Error: File not found: {path}"));
        }

        let content = tokio::fs::read_to_string(&resolved).await?;
        if !content.contains(old_text) {
            return Ok(
                "Error: old_text not found in file. Make sure it matches exactly.".to_string(),
            );
        }
        let count = content.matches(old_text).count();
        if count > 1 {
            return Ok(format!(
                "Warning: old_text appears {count} times. Please provide more context to make it unique."
            ));
        }

        let updated = content.replacen(old_text, new_text, 1);
        tokio::fs::write(&resolved, updated).await?;
        Ok(format!("Successfully edited {path}"))
    }
}

pub struct ListDirTool {
    allowed_dir: Option<PathBuf>,
}

impl ListDirTool {
    pub fn new(allowed_dir: Option<PathBuf>) -> Self {
        Self { allowed_dir }
    }
}

#[async_trait]
impl Tool for ListDirTool {
    fn name(&self) -> &str {
        "list_dir"
    }

    fn description(&self) -> &str {
        "List the contents of a directory."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "The directory path to list" }
            },
            "required": ["path"]
        })
    }

    async fn execute(&self, params: &Map<String, Value>) -> Result<String> {
        let path = get_required_string(params, "path")?;
        let resolved = resolve_path(path, self.allowed_dir.as_ref())?;

        if !resolved.exists() {
            return Ok(format!("Error: Directory not found: {path}"));
        }
        if !resolved.is_dir() {
            return Ok(format!("Error: Not a directory: {path}"));
        }

        let mut entries = tokio::fs::read_dir(&resolved).await?;
        let mut items = Vec::new();
        while let Some(entry) = entries.next_entry().await? {
            let metadata = entry.metadata().await?;
            let prefix = if metadata.is_dir() { "[DIR]" } else { "[FILE]" };
            items.push(format!("{prefix} {}", entry.file_name().to_string_lossy()));
        }
        items.sort();

        if items.is_empty() {
            Ok(format!("Directory {path} is empty"))
        } else {
            Ok(items.join("\n"))
        }
    }
}
