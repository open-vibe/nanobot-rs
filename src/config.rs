use crate::utils::{expand_tilde, get_data_path};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct ProviderConfig {
    pub api_key: String,
    pub api_base: Option<String>,
    pub extra_headers: Option<HashMap<String, String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ProvidersConfig {
    pub anthropic: ProviderConfig,
    pub openai: ProviderConfig,
    pub openrouter: ProviderConfig,
    pub aihubmix: ProviderConfig,
    pub deepseek: ProviderConfig,
    pub groq: ProviderConfig,
    pub zhipu: ProviderConfig,
    pub dashscope: ProviderConfig,
    pub vllm: ProviderConfig,
    pub gemini: ProviderConfig,
    pub moonshot: ProviderConfig,
}

impl Default for ProvidersConfig {
    fn default() -> Self {
        Self {
            anthropic: ProviderConfig::default(),
            openai: ProviderConfig::default(),
            openrouter: ProviderConfig::default(),
            aihubmix: ProviderConfig::default(),
            deepseek: ProviderConfig::default(),
            groq: ProviderConfig::default(),
            zhipu: ProviderConfig::default(),
            dashscope: ProviderConfig::default(),
            vllm: ProviderConfig::default(),
            gemini: ProviderConfig::default(),
            moonshot: ProviderConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AgentDefaults {
    pub workspace: String,
    pub model: String,
    pub max_tokens: u32,
    pub temperature: f32,
    pub max_tool_iterations: u32,
}

impl Default for AgentDefaults {
    fn default() -> Self {
        Self {
            workspace: "~/.nanobot/workspace".to_string(),
            model: "anthropic/claude-opus-4-5".to_string(),
            max_tokens: 8192,
            temperature: 0.7,
            max_tool_iterations: 20,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct AgentsConfig {
    pub defaults: AgentDefaults,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct WebSearchConfig {
    pub api_key: String,
    pub max_results: usize,
}

impl Default for WebSearchConfig {
    fn default() -> Self {
        Self {
            api_key: String::new(),
            max_results: 5,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct WebToolsConfig {
    pub search: WebSearchConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ExecToolConfig {
    pub timeout: u64,
}

impl Default for ExecToolConfig {
    fn default() -> Self {
        Self { timeout: 60 }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct ToolsConfig {
    pub web: WebToolsConfig,
    pub exec: ExecToolConfig,
    pub restrict_to_workspace: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct GatewayConfig {
    pub host: String,
    pub port: u16,
}

impl Default for GatewayConfig {
    fn default() -> Self {
        Self {
            host: "0.0.0.0".to_string(),
            port: 18790,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct TelegramConfig {
    pub enabled: bool,
    pub token: String,
    pub allow_from: Vec<String>,
    pub proxy: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct WhatsAppConfig {
    pub enabled: bool,
    pub bridge_url: String,
    pub allow_from: Vec<String>,
}

impl Default for WhatsAppConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            bridge_url: "ws://localhost:3001".to_string(),
            allow_from: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct DiscordConfig {
    pub enabled: bool,
    pub token: String,
    pub allow_from: Vec<String>,
    pub gateway_url: String,
    pub intents: u32,
}

impl Default for DiscordConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            token: String::new(),
            allow_from: Vec::new(),
            gateway_url: "wss://gateway.discord.gg/?v=10&encoding=json".to_string(),
            intents: 37377,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct FeishuConfig {
    pub enabled: bool,
    pub app_id: String,
    pub app_secret: String,
    pub encrypt_key: String,
    pub verification_token: String,
    pub allow_from: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct DingTalkConfig {
    pub enabled: bool,
    pub client_id: String,
    pub client_secret: String,
    pub allow_from: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct EmailConfig {
    pub enabled: bool,
    pub consent_granted: bool,
    pub imap_host: String,
    pub imap_port: u16,
    pub imap_username: String,
    pub imap_password: String,
    pub imap_mailbox: String,
    pub imap_use_ssl: bool,
    pub smtp_host: String,
    pub smtp_port: u16,
    pub smtp_username: String,
    pub smtp_password: String,
    pub smtp_use_tls: bool,
    pub smtp_use_ssl: bool,
    pub from_address: String,
    pub auto_reply_enabled: bool,
    pub poll_interval_seconds: u64,
    pub mark_seen: bool,
    pub max_body_chars: usize,
    pub subject_prefix: String,
    pub allow_from: Vec<String>,
}

impl Default for EmailConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            consent_granted: false,
            imap_host: String::new(),
            imap_port: 993,
            imap_username: String::new(),
            imap_password: String::new(),
            imap_mailbox: "INBOX".to_string(),
            imap_use_ssl: true,
            smtp_host: String::new(),
            smtp_port: 587,
            smtp_username: String::new(),
            smtp_password: String::new(),
            smtp_use_tls: true,
            smtp_use_ssl: false,
            from_address: String::new(),
            auto_reply_enabled: true,
            poll_interval_seconds: 30,
            mark_seen: true,
            max_body_chars: 12_000,
            subject_prefix: "Re: ".to_string(),
            allow_from: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct ChannelsConfig {
    pub whatsapp: WhatsAppConfig,
    pub telegram: TelegramConfig,
    pub discord: DiscordConfig,
    pub feishu: FeishuConfig,
    pub dingtalk: DingTalkConfig,
    pub email: EmailConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct Config {
    pub agents: AgentsConfig,
    pub channels: ChannelsConfig,
    pub providers: ProvidersConfig,
    pub gateway: GatewayConfig,
    pub tools: ToolsConfig,
}

impl Config {
    pub fn workspace_path(&self) -> PathBuf {
        expand_tilde(&self.agents.defaults.workspace)
    }

    fn match_provider(
        &self,
        model: Option<&str>,
    ) -> (Option<&ProviderConfig>, Option<&'static str>) {
        let m = model.unwrap_or(&self.agents.defaults.model).to_lowercase();
        let mapping: [(&str, &[&str]); 11] = [
            ("openrouter", &["openrouter"]),
            ("aihubmix", &["aihubmix"]),
            ("anthropic", &["anthropic", "claude"]),
            ("openai", &["openai", "gpt"]),
            ("deepseek", &["deepseek"]),
            ("gemini", &["gemini"]),
            ("zhipu", &["zhipu", "glm", "zai"]),
            ("dashscope", &["qwen", "dashscope"]),
            ("moonshot", &["moonshot", "kimi"]),
            ("vllm", &["vllm"]),
            ("groq", &["groq"]),
        ];

        for (name, keywords) in mapping {
            let provider = self.provider_by_name(name);
            if keywords.iter().any(|kw| m.contains(kw)) && !provider.api_key.is_empty() {
                return (Some(provider), Some(name));
            }
        }

        for name in [
            "openrouter",
            "aihubmix",
            "anthropic",
            "openai",
            "deepseek",
            "gemini",
            "zhipu",
            "dashscope",
            "moonshot",
            "vllm",
            "groq",
        ] {
            let provider = self.provider_by_name(name);
            if !provider.api_key.is_empty() {
                return (Some(provider), Some(name));
            }
        }
        (None, None)
    }

    fn provider_by_name(&self, name: &str) -> &ProviderConfig {
        match name {
            "openrouter" => &self.providers.openrouter,
            "aihubmix" => &self.providers.aihubmix,
            "anthropic" => &self.providers.anthropic,
            "openai" => &self.providers.openai,
            "deepseek" => &self.providers.deepseek,
            "gemini" => &self.providers.gemini,
            "zhipu" => &self.providers.zhipu,
            "dashscope" => &self.providers.dashscope,
            "moonshot" => &self.providers.moonshot,
            "vllm" => &self.providers.vllm,
            "groq" => &self.providers.groq,
            _ => &self.providers.openai,
        }
    }

    pub fn get_provider(&self, model: Option<&str>) -> Option<&ProviderConfig> {
        let (provider, _) = self.match_provider(model);
        provider
    }

    pub fn get_provider_name(&self, model: Option<&str>) -> Option<String> {
        let (_, name) = self.match_provider(model);
        name.map(ToOwned::to_owned)
    }

    pub fn get_api_key(&self, model: Option<&str>) -> Option<String> {
        if let Some(provider) = self.get_provider(model) {
            return Some(provider.api_key.clone());
        }
        None
    }

    pub fn get_api_base(&self, model: Option<&str>) -> Option<String> {
        let (provider, name) = self.match_provider(model);
        if let Some(provider) = provider {
            if provider.api_base.is_some() {
                return provider.api_base.clone();
            }
        }
        match name {
            Some("openrouter") => Some(
                self.providers
                    .openrouter
                    .api_base
                    .clone()
                    .unwrap_or_else(|| "https://openrouter.ai/api/v1".to_string()),
            ),
            Some("aihubmix") => Some(
                self.providers
                    .aihubmix
                    .api_base
                    .clone()
                    .unwrap_or_else(|| "https://aihubmix.com/v1".to_string()),
            ),
            _ => None,
        }
    }
}

pub fn get_config_path() -> Result<PathBuf> {
    Ok(get_data_path()?.join("config.json"))
}

pub fn load_config(config_path: Option<&Path>) -> Result<Config> {
    let path = match config_path {
        Some(p) => p.to_path_buf(),
        None => get_config_path()?,
    };

    if !path.exists() {
        return Ok(Config::default());
    }

    let raw = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read config: {}", path.display()))?;
    let mut value: Value = serde_json::from_str(&raw)
        .with_context(|| format!("invalid JSON in {}", path.display()))?;
    migrate_config(&mut value);
    let config = serde_json::from_value(value).context("failed to parse config structure")?;
    Ok(config)
}

pub fn save_config(config: &Config, config_path: Option<&Path>) -> Result<()> {
    let path = match config_path {
        Some(p) => p.to_path_buf(),
        None => get_config_path()?,
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let text = serde_json::to_string_pretty(config)?;
    std::fs::write(&path, text)?;
    Ok(())
}

fn migrate_config(value: &mut Value) {
    let Some(root) = value.as_object_mut() else {
        return;
    };
    let Some(tools) = root.get_mut("tools").and_then(Value::as_object_mut) else {
        return;
    };
    let should_migrate = tools.get("restrictToWorkspace").is_none();
    if should_migrate {
        if let Some(exec) = tools.get_mut("exec").and_then(Value::as_object_mut) {
            if let Some(v) = exec.remove("restrictToWorkspace") {
                tools.insert("restrictToWorkspace".to_string(), v);
            }
        }
    }
}

pub fn providers_status(config: &Config) -> Map<String, Value> {
    let mut map = Map::new();
    map.insert(
        "openrouter".to_string(),
        Value::Bool(!config.providers.openrouter.api_key.is_empty()),
    );
    map.insert(
        "aihubmix".to_string(),
        Value::Bool(!config.providers.aihubmix.api_key.is_empty()),
    );
    map.insert(
        "anthropic".to_string(),
        Value::Bool(!config.providers.anthropic.api_key.is_empty()),
    );
    map.insert(
        "openai".to_string(),
        Value::Bool(!config.providers.openai.api_key.is_empty()),
    );
    map.insert(
        "deepseek".to_string(),
        Value::Bool(!config.providers.deepseek.api_key.is_empty()),
    );
    map.insert(
        "gemini".to_string(),
        Value::Bool(!config.providers.gemini.api_key.is_empty()),
    );
    map.insert(
        "zhipu".to_string(),
        Value::Bool(!config.providers.zhipu.api_key.is_empty()),
    );
    map.insert(
        "dashscope".to_string(),
        Value::Bool(!config.providers.dashscope.api_key.is_empty()),
    );
    map.insert(
        "moonshot".to_string(),
        Value::Bool(!config.providers.moonshot.api_key.is_empty()),
    );
    map.insert(
        "vllm".to_string(),
        Value::Bool(config.providers.vllm.api_base.is_some()),
    );
    map.insert(
        "groq".to_string(),
        Value::Bool(!config.providers.groq.api_key.is_empty()),
    );
    map
}
