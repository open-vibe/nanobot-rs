use crate::tools::base::Tool;
use anyhow::{Result, anyhow};
use async_trait::async_trait;
use regex::Regex;
use serde_json::{Map, Value, json};
use std::path::{Component, Path, PathBuf};
use tokio::process::Command;
use tokio::time::{Duration, timeout};

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

pub struct ExecTool {
    timeout_s: u64,
    working_dir: Option<PathBuf>,
    deny_patterns: Vec<String>,
    allow_patterns: Vec<String>,
    restrict_to_workspace: bool,
}

impl ExecTool {
    pub fn new(
        timeout_s: u64,
        working_dir: Option<PathBuf>,
        deny_patterns: Option<Vec<String>>,
        allow_patterns: Option<Vec<String>>,
        restrict_to_workspace: bool,
    ) -> Self {
        Self {
            timeout_s,
            working_dir,
            deny_patterns: deny_patterns.unwrap_or_else(|| {
                vec![
                    r"\brm\s+-[rf]{1,2}\b",
                    r"\bdel\s+/[fq]\b",
                    r"\brmdir\s+/s\b",
                    r"\b(format|mkfs|diskpart)\b",
                    r"\bdd\s+if=",
                    r">\s*/dev/sd",
                    r"\b(shutdown|reboot|poweroff)\b",
                    r":\(\)\s*\{.*\};\s*:",
                ]
                .into_iter()
                .map(str::to_string)
                .collect()
            }),
            allow_patterns: allow_patterns.unwrap_or_default(),
            restrict_to_workspace,
        }
    }

    fn guard_command(&self, command: &str, cwd: &Path) -> Option<String> {
        let trimmed = command.trim();
        let lower = trimmed.to_lowercase();

        for pattern in &self.deny_patterns {
            if let Ok(re) = Regex::new(pattern) {
                if re.is_match(&lower) {
                    return Some(
                        "Error: Command blocked by safety guard (dangerous pattern detected)"
                            .to_string(),
                    );
                }
            }
        }

        if !self.allow_patterns.is_empty() {
            let allowed = self.allow_patterns.iter().any(|pattern| {
                Regex::new(pattern)
                    .map(|re| re.is_match(&lower))
                    .unwrap_or(false)
            });
            if !allowed {
                return Some(
                    "Error: Command blocked by safety guard (not in allowlist)".to_string(),
                );
            }
        }

        if self.restrict_to_workspace {
            if lower.contains("..\\") || lower.contains("../") {
                return Some(
                    "Error: Command blocked by safety guard (path traversal detected)".to_string(),
                );
            }

            let cwd = normalize_path(cwd);
            let win_paths = Regex::new(r#"[A-Za-z]:\\[^\\\"'\s]+"#)
                .ok()
                .map(|re| {
                    re.find_iter(trimmed)
                        .map(|m| m.as_str().to_string())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let posix_paths = Regex::new(r#"/[^\s\"']+"#)
                .ok()
                .map(|re| {
                    re.find_iter(trimmed)
                        .map(|m| m.as_str().to_string())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();

            for raw in win_paths.into_iter().chain(posix_paths) {
                let p = normalize_path(Path::new(&raw));
                if !p.starts_with(&cwd) && p != cwd {
                    return Some(
                        "Error: Command blocked by safety guard (path outside working dir)"
                            .to_string(),
                    );
                }
            }
        }

        None
    }
}

#[async_trait]
impl Tool for ExecTool {
    fn name(&self) -> &str {
        "exec"
    }

    fn description(&self) -> &str {
        "Execute a shell command and return its output. Use with caution."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "command": { "type": "string", "description": "The shell command to execute" },
                "working_dir": { "type": "string", "description": "Optional working directory for the command" }
            },
            "required": ["command"]
        })
    }

    async fn execute(&self, params: &Map<String, Value>) -> Result<String> {
        let command = params
            .get("command")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("missing required string field: command"))?;

        let cwd = params
            .get("working_dir")
            .and_then(Value::as_str)
            .map(PathBuf::from)
            .or_else(|| self.working_dir.clone())
            .unwrap_or(std::env::current_dir()?);

        if let Some(err) = self.guard_command(command, &cwd) {
            return Ok(err);
        }

        let mut process = if cfg!(target_os = "windows") {
            let mut cmd = Command::new("cmd");
            cmd.args(["/C", command]);
            cmd
        } else {
            let mut cmd = Command::new("sh");
            cmd.args(["-c", command]);
            cmd
        };

        process.current_dir(&cwd);
        let output = timeout(Duration::from_secs(self.timeout_s), process.output()).await;
        let output = match output {
            Ok(result) => result?,
            Err(_) => {
                return Ok(format!(
                    "Error: Command timed out after {} seconds",
                    self.timeout_s
                ));
            }
        };

        let mut output_parts = Vec::new();
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        if !stdout.is_empty() {
            output_parts.push(stdout);
        }
        if !stderr.trim().is_empty() {
            output_parts.push(format!("STDERR:\n{stderr}"));
        }
        if !output.status.success() {
            output_parts.push(format!(
                "\nExit code: {}",
                output.status.code().unwrap_or(-1)
            ));
        }

        let mut result = if output_parts.is_empty() {
            "(no output)".to_string()
        } else {
            output_parts.join("\n")
        };
        let max_len = 10_000;
        if result.len() > max_len {
            result = format!(
                "{}\n... (truncated, {} more chars)",
                &result[..max_len],
                result.len() - max_len
            );
        }
        Ok(result)
    }
}
