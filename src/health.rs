use crate::VERSION;
use crate::config::{Config, get_config_path, providers_status, save_config};
use crate::utils::{get_data_path, get_workspace_path};
use anyhow::{Result, anyhow};
use chrono::Local;
use semver::Version;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::process::Command;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CheckLevel {
    Ok,
    Warn,
    Fail,
}

#[derive(Debug, Clone, Serialize)]
pub struct HealthCheck {
    pub id: String,
    pub label: String,
    pub level: CheckLevel,
    pub detail: String,
    pub fix_hint: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct HealthSummary {
    pub ok: usize,
    pub warn: usize,
    pub fail: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct HealthReport {
    pub generated_at: String,
    pub checks: Vec<HealthCheck>,
    pub summary: HealthSummary,
}

#[derive(Debug, Clone, Serialize)]
pub struct DoctorResult {
    pub report: HealthReport,
    pub changed: bool,
    pub actions: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct UpdateReport {
    pub current_version: String,
    pub latest_version: Option<String>,
    pub update_available: bool,
    pub registry_error: Option<String>,
    pub git: GitStatus,
}

#[derive(Debug, Clone, Serialize)]
pub struct GitStatus {
    pub inside_repo: bool,
    pub branch: Option<String>,
    pub dirty: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct CratesIoResponse {
    #[serde(default)]
    #[serde(rename = "crate")]
    crate_info: CrateInfo,
}

#[derive(Debug, Default, Deserialize)]
struct CrateInfo {
    #[serde(default)]
    max_stable_version: String,
}

fn parse_semver_or_none(v: &str) -> Option<Version> {
    Version::parse(v.trim_start_matches('v')).ok()
}

fn count_summary(checks: &[HealthCheck]) -> HealthSummary {
    let mut summary = HealthSummary {
        ok: 0,
        warn: 0,
        fail: 0,
    };
    for check in checks {
        match check.level {
            CheckLevel::Ok => summary.ok += 1,
            CheckLevel::Warn => summary.warn += 1,
            CheckLevel::Fail => summary.fail += 1,
        }
    }
    summary
}

fn enabled_channels(config: &Config) -> Vec<&'static str> {
    let mut out = Vec::new();
    if config.channels.telegram.enabled {
        out.push("telegram");
    }
    if config.channels.discord.enabled {
        out.push("discord");
    }
    if config.channels.whatsapp.enabled {
        out.push("whatsapp");
    }
    if config.channels.feishu.enabled {
        out.push("feishu");
    }
    if config.channels.mochat.enabled {
        out.push("mochat");
    }
    if config.channels.dingtalk.enabled {
        out.push("dingtalk");
    }
    if config.channels.email.enabled {
        out.push("email");
    }
    if config.channels.slack.enabled {
        out.push("slack");
    }
    if config.channels.qq.enabled {
        out.push("qq");
    }
    out
}

fn cron_jobs_count(data_dir: &Path) -> usize {
    let path = data_dir.join("cron").join("jobs.json");
    let raw = match std::fs::read_to_string(path) {
        Ok(v) => v,
        Err(_) => return 0,
    };
    let value: serde_json::Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(_) => return 0,
    };
    value
        .get("jobs")
        .and_then(serde_json::Value::as_array)
        .map(|arr| arr.len())
        .unwrap_or(0)
}

fn has_any_provider(config: &Config) -> bool {
    providers_status(config)
        .values()
        .filter_map(serde_json::Value::as_bool)
        .any(|v| v)
}

fn check_workspace_files(workspace: &Path) -> (bool, Vec<String>) {
    let required = [
        workspace.join("AGENTS.md"),
        workspace.join("SOUL.md"),
        workspace.join("USER.md"),
        workspace.join("HEARTBEAT.md"),
        workspace.join("memory").join("MEMORY.md"),
        workspace.join("memory").join("HISTORY.md"),
    ];
    let mut missing = Vec::new();
    for path in required {
        if !path.exists() {
            missing.push(path.display().to_string());
        }
    }
    (missing.is_empty(), missing)
}

pub fn collect_health(config: &Config) -> Result<HealthReport> {
    let config_path = get_config_path()?;
    let data_path = get_data_path()?;
    let workspace = config.workspace_path();
    let channels = enabled_channels(config);
    let cron_count = cron_jobs_count(&data_path);
    let (workspace_ok, missing_workspace_files) = check_workspace_files(&workspace);
    let checks = vec![
        HealthCheck {
            id: "config.file".to_string(),
            label: "Config file".to_string(),
            level: if config_path.exists() {
                CheckLevel::Ok
            } else {
                CheckLevel::Fail
            },
            detail: format!("{}", config_path.display()),
            fix_hint: if config_path.exists() {
                None
            } else {
                Some("Run `nanobot-rs onboard` or `nanobot-rs doctor --fix`.".to_string())
            },
        },
        HealthCheck {
            id: "workspace.dir".to_string(),
            label: "Workspace directory".to_string(),
            level: if workspace.exists() {
                CheckLevel::Ok
            } else {
                CheckLevel::Fail
            },
            detail: format!("{}", workspace.display()),
            fix_hint: if workspace.exists() {
                None
            } else {
                Some("Run `nanobot-rs onboard` or `nanobot-rs doctor --fix`.".to_string())
            },
        },
        HealthCheck {
            id: "workspace.files".to_string(),
            label: "Workspace baseline files".to_string(),
            level: if workspace_ok {
                CheckLevel::Ok
            } else {
                CheckLevel::Warn
            },
            detail: if workspace_ok {
                "required files present".to_string()
            } else {
                format!("missing {} file(s)", missing_workspace_files.len())
            },
            fix_hint: if workspace_ok {
                None
            } else {
                Some("Run `nanobot-rs doctor --fix`.".to_string())
            },
        },
        HealthCheck {
            id: "provider.api".to_string(),
            label: "Provider API credentials".to_string(),
            level: if has_any_provider(config) {
                CheckLevel::Ok
            } else {
                CheckLevel::Fail
            },
            detail: "at least one providers.*.apiKey".to_string(),
            fix_hint: if has_any_provider(config) {
                None
            } else {
                Some("Set providers.*.apiKey in ~/.nanobot/config.json.".to_string())
            },
        },
        HealthCheck {
            id: "agent.model".to_string(),
            label: "Default model".to_string(),
            level: if config.agents.defaults.model.trim().is_empty() {
                CheckLevel::Warn
            } else {
                CheckLevel::Ok
            },
            detail: if config.agents.defaults.model.trim().is_empty() {
                "(empty)".to_string()
            } else {
                config.agents.defaults.model.clone()
            },
            fix_hint: if config.agents.defaults.model.trim().is_empty() {
                Some("Set agents.defaults.model in config.".to_string())
            } else {
                None
            },
        },
        HealthCheck {
            id: "channels.enabled".to_string(),
            label: "Enabled channels".to_string(),
            level: if channels.is_empty() {
                CheckLevel::Warn
            } else {
                CheckLevel::Ok
            },
            detail: if channels.is_empty() {
                "none".to_string()
            } else {
                channels.join(", ")
            },
            fix_hint: if channels.is_empty() {
                Some("Enable at least one channel in config if you use gateway mode.".to_string())
            } else {
                None
            },
        },
        HealthCheck {
            id: "cron.jobs".to_string(),
            label: "Scheduled jobs".to_string(),
            level: if cron_count == 0 {
                CheckLevel::Warn
            } else {
                CheckLevel::Ok
            },
            detail: format!("{cron_count} job(s)"),
            fix_hint: if cron_count == 0 {
                Some("Use `nanobot-rs cron add ...` if you rely on automation.".to_string())
            } else {
                None
            },
        },
    ];
    Ok(HealthReport {
        generated_at: Local::now().to_rfc3339(),
        summary: count_summary(&checks),
        checks,
    })
}

fn ensure_workspace_baseline(workspace: &Path, actions: &mut Vec<String>) -> Result<()> {
    std::fs::create_dir_all(workspace)?;
    let templates = [
        (
            "AGENTS.md",
            "# Agent Instructions\n\nYou are a helpful AI assistant. Be concise and accurate.\n",
        ),
        (
            "SOUL.md",
            "# Soul\n\nI am nanobot-rs, a lightweight Rust AI assistant.\n",
        ),
        (
            "USER.md",
            "# User\n\nRecord user preferences and context here.\n",
        ),
        (
            "HEARTBEAT.md",
            "# Heartbeat\n\n- [ ] Add periodic tasks here.\n",
        ),
    ];
    for (name, content) in templates {
        let path = workspace.join(name);
        if !path.exists() {
            std::fs::write(&path, content)?;
            actions.push(format!("created {}", path.display()));
        }
    }

    let memory_dir = workspace.join("memory");
    std::fs::create_dir_all(&memory_dir)?;
    let memory_file = memory_dir.join("MEMORY.md");
    if !memory_file.exists() {
        std::fs::write(
            &memory_file,
            "# Long-term Memory\n\nThis file stores important information across sessions.\n",
        )?;
        actions.push(format!("created {}", memory_file.display()));
    }
    let history_file = memory_dir.join("HISTORY.md");
    if !history_file.exists() {
        std::fs::write(&history_file, "")?;
        actions.push(format!("created {}", history_file.display()));
    }

    let skills_dir = workspace.join("skills");
    if !skills_dir.exists() {
        std::fs::create_dir_all(&skills_dir)?;
        actions.push(format!("created {}", skills_dir.display()));
    }
    Ok(())
}

pub fn run_doctor(apply_fix: bool) -> Result<DoctorResult> {
    let config_path = get_config_path()?;
    let mut config = crate::config::load_config(Some(&config_path)).unwrap_or_default();
    let mut changed = false;
    let mut actions = Vec::new();

    if apply_fix {
        if !config_path.exists() {
            if let Some(parent) = config_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            save_config(&config, Some(&config_path))?;
            changed = true;
            actions.push(format!("created {}", config_path.display()));
        }

        let workspace = get_workspace_path(Some(&config.agents.defaults.workspace))?;
        ensure_workspace_baseline(&workspace, &mut actions)?;

        if config.agents.defaults.model.trim().is_empty() {
            config.agents.defaults.model = "deepseek/deepseek-reasoner".to_string();
            changed = true;
            actions.push("set agents.defaults.model=deepseek/deepseek-reasoner".to_string());
        }

        if changed {
            save_config(&config, Some(&config_path))?;
        }
    }

    if !actions.is_empty() {
        changed = true;
    }

    let report = collect_health(&config)?;
    Ok(DoctorResult {
        report,
        changed,
        actions,
    })
}

fn git_capture(args: &[&str]) -> Option<String> {
    let output = Command::new("git").args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if text.is_empty() { None } else { Some(text) }
}

fn git_status() -> GitStatus {
    let inside = git_capture(&["rev-parse", "--is-inside-work-tree"])
        .map(|v| v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    if !inside {
        return GitStatus {
            inside_repo: false,
            branch: None,
            dirty: None,
        };
    }
    let branch = git_capture(&["rev-parse", "--abbrev-ref", "HEAD"]);
    let dirty = Command::new("git")
        .args(["status", "--porcelain"])
        .output()
        .ok()
        .map(|o| !String::from_utf8_lossy(&o.stdout).trim().is_empty());
    GitStatus {
        inside_repo: true,
        branch,
        dirty,
    }
}

pub async fn check_update(crate_name: &str) -> Result<UpdateReport> {
    if crate_name.trim().is_empty() {
        return Err(anyhow!("crate name cannot be empty"));
    }
    let url = format!("https://crates.io/api/v1/crates/{crate_name}");
    let mut latest_version = None::<String>;
    let mut registry_error = None::<String>;

    let response = reqwest::Client::new()
        .get(url)
        .header("User-Agent", "nanobot-rs-update-check")
        .send()
        .await;
    match response {
        Ok(resp) if resp.status().is_success() => {
            let body: CratesIoResponse = resp.json().await.unwrap_or(CratesIoResponse {
                crate_info: CrateInfo::default(),
            });
            if !body.crate_info.max_stable_version.trim().is_empty() {
                latest_version = Some(body.crate_info.max_stable_version);
            }
        }
        Ok(resp) => {
            registry_error = Some(format!("registry responded with status {}", resp.status()));
        }
        Err(err) => {
            registry_error = Some(format!("request failed: {err}"));
        }
    }

    let update_available = if let (Some(current), Some(latest)) =
        (parse_semver_or_none(VERSION), latest_version.clone())
    {
        parse_semver_or_none(&latest)
            .map(|v| v > current)
            .unwrap_or(false)
    } else {
        false
    };

    Ok(UpdateReport {
        current_version: VERSION.to_string(),
        latest_version,
        update_available,
        registry_error,
        git: git_status(),
    })
}
