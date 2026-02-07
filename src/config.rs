use crate::utils::{expand_tilde, get_data_path};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct ProviderConfig {
    pub api_key: String,
    pub api_base: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ProvidersConfig {
    pub anthropic: ProviderConfig,
    pub openai: ProviderConfig,
    pub openrouter: ProviderConfig,
    pub deepseek: ProviderConfig,
    pub groq: ProviderConfig,
    pub zhipu: ProviderConfig,
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
            deepseek: ProviderConfig::default(),
            groq: ProviderConfig::default(),
            zhipu: ProviderConfig::default(),
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
pub struct ChannelsConfig {
    pub whatsapp: WhatsAppConfig,
    pub telegram: TelegramConfig,
    pub discord: DiscordConfig,
    pub feishu: FeishuConfig,
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

    fn match_provider(&self, model: Option<&str>) -> Option<&ProviderConfig> {
        let m = model.unwrap_or(&self.agents.defaults.model).to_lowercase();
        let mapping: [(&str, &ProviderConfig); 14] = [
            ("openrouter", &self.providers.openrouter),
            ("deepseek", &self.providers.deepseek),
            ("anthropic", &self.providers.anthropic),
            ("claude", &self.providers.anthropic),
            ("openai", &self.providers.openai),
            ("gpt", &self.providers.openai),
            ("gemini", &self.providers.gemini),
            ("zhipu", &self.providers.zhipu),
            ("glm", &self.providers.zhipu),
            ("zai", &self.providers.zhipu),
            ("groq", &self.providers.groq),
            ("moonshot", &self.providers.moonshot),
            ("kimi", &self.providers.moonshot),
            ("vllm", &self.providers.vllm),
        ];

        for (keyword, provider) in mapping {
            if m.contains(keyword) && !provider.api_key.is_empty() {
                return Some(provider);
            }
        }
        None
    }

    pub fn get_api_key(&self, model: Option<&str>) -> Option<String> {
        if let Some(provider) = self.match_provider(model) {
            return Some(provider.api_key.clone());
        }
        for provider in [
            &self.providers.openrouter,
            &self.providers.deepseek,
            &self.providers.anthropic,
            &self.providers.openai,
            &self.providers.gemini,
            &self.providers.zhipu,
            &self.providers.moonshot,
            &self.providers.vllm,
            &self.providers.groq,
        ] {
            if !provider.api_key.is_empty() {
                return Some(provider.api_key.clone());
            }
        }
        None
    }

    pub fn get_api_base(&self, model: Option<&str>) -> Option<String> {
        let m = model.unwrap_or(&self.agents.defaults.model).to_lowercase();
        if m.contains("openrouter") {
            return Some(
                self.providers
                    .openrouter
                    .api_base
                    .clone()
                    .unwrap_or_else(|| "https://openrouter.ai/api/v1".to_string()),
            );
        }
        if m.contains("zhipu") || m.contains("glm") || m.contains("zai") {
            return self.providers.zhipu.api_base.clone();
        }
        if m.contains("vllm") {
            return self.providers.vllm.api_base.clone();
        }
        None
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
        "anthropic".to_string(),
        Value::Bool(!config.providers.anthropic.api_key.is_empty()),
    );
    map.insert(
        "openai".to_string(),
        Value::Bool(!config.providers.openai.api_key.is_empty()),
    );
    map.insert(
        "gemini".to_string(),
        Value::Bool(!config.providers.gemini.api_key.is_empty()),
    );
    map.insert(
        "vllm".to_string(),
        Value::Bool(config.providers.vllm.api_base.is_some()),
    );
    map
}
