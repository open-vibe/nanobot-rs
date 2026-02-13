use anyhow::{Result, anyhow};
use clap::{ArgAction, Parser, Subcommand};
use nanobot::VERSION;
use nanobot::agent::AgentLoop;
use nanobot::bus::{MessageBus, OutboundMessage};
use nanobot::channels::manager::ChannelManager;
use nanobot::config::{Config, get_config_path, load_config, providers_status, save_config};
use nanobot::cron::{CronSchedule, CronService};
use nanobot::heartbeat::{DEFAULT_HEARTBEAT_INTERVAL_S, HeartbeatService};
use nanobot::providers::base::LLMProvider;
use nanobot::providers::litellm::LiteLLMProvider;
use nanobot::service::{self, ServiceAccount, ServiceInstallOptions};
use nanobot::session::SessionManager;
use nanobot::utils::{get_data_path, get_workspace_path};
use std::fs;
use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::process::Command;
use which::which;

#[derive(Debug, Parser)]
#[command(
    name = "nanobot-rs",
    about = "nanobot: Rust port of the lightweight personal AI assistant"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    Onboard,
    Gateway {
        #[arg(short, long, default_value_t = 18790)]
        port: u16,
        #[arg(short, long, default_value_t = false)]
        verbose: bool,
    },
    Agent {
        #[arg(short, long)]
        message: Option<String>,
        #[arg(short, long, default_value = "cli:direct")]
        session: String,
    },
    Status,
    Version,
    Channels {
        #[command(subcommand)]
        command: ChannelCommand,
    },
    Cron {
        #[command(subcommand)]
        command: CronCommand,
    },
    Service {
        #[command(subcommand)]
        command: ServiceCommand,
    },
}

#[derive(Debug, Subcommand)]
enum ChannelCommand {
    Status,
    Login,
}

#[derive(Debug, Subcommand)]
enum CronCommand {
    List {
        #[arg(short, long, default_value_t = false)]
        all: bool,
    },
    Add {
        #[arg(short, long)]
        name: String,
        #[arg(short, long)]
        message: String,
        #[arg(short = 'e', long)]
        every: Option<i64>,
        #[arg(short = 'c', long)]
        cron: Option<String>,
        #[arg(long)]
        at: Option<String>,
        #[arg(short, long, default_value_t = false)]
        deliver: bool,
        #[arg(long)]
        to: Option<String>,
        #[arg(long)]
        channel: Option<String>,
    },
    Remove {
        job_id: String,
    },
    Enable {
        job_id: String,
        #[arg(long, default_value_t = false)]
        disable: bool,
    },
    Run {
        job_id: String,
        #[arg(short, long, default_value_t = false)]
        force: bool,
    },
}

#[derive(Debug, Subcommand)]
enum ServiceCommand {
    Install {
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        bin: Option<PathBuf>,
        #[arg(long, default_value = "gateway")]
        args: String,
        #[arg(long)]
        workdir: Option<PathBuf>,
        #[arg(long, action = ArgAction::SetTrue)]
        system: bool,
        #[arg(long, action = ArgAction::SetTrue)]
        use_current_user: bool,
        #[arg(long)]
        password: Option<String>,
        #[arg(long, default_value_t = true, action = ArgAction::Set)]
        auto_install_nssm: bool,
        #[arg(long, default_value_t = true, action = ArgAction::Set)]
        autostart: bool,
    },
    Remove {
        #[arg(long)]
        name: Option<String>,
    },
    Start {
        #[arg(long)]
        name: Option<String>,
    },
    Stop {
        #[arg(long)]
        name: Option<String>,
    },
    Restart {
        #[arg(long)]
        name: Option<String>,
    },
    Status {
        #[arg(long)]
        name: Option<String>,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Onboard => cmd_onboard()?,
        Commands::Status => cmd_status()?,
        Commands::Version => println!("nanobot-rs v{VERSION}"),
        Commands::Gateway { port, verbose } => cmd_gateway(port, verbose).await?,
        Commands::Agent { message, session } => cmd_agent(message, &session).await?,
        Commands::Channels { command } => cmd_channels(command).await?,
        Commands::Cron { command } => cmd_cron(command).await?,
        Commands::Service { command } => cmd_service(command)?,
    }
    Ok(())
}

fn cmd_onboard() -> Result<()> {
    let config_path = get_config_path()?;
    if config_path.exists() {
        println!("Config already exists at {}", config_path.display());
        return Ok(());
    }

    let config = Config::default();
    save_config(&config, Some(&config_path))?;
    println!("Created config at {}", config_path.display());

    let workspace = get_workspace_path(Some(&config.agents.defaults.workspace))?;
    println!("Created workspace at {}", workspace.display());

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
            println!("Created {}", path.display());
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
        println!("Created {}", memory_file.display());
    }
    let history_file = memory_dir.join("HISTORY.md");
    if !history_file.exists() {
        std::fs::write(&history_file, "")?;
        println!("Created {}", history_file.display());
    }

    let skills_dir = workspace.join("skills");
    std::fs::create_dir_all(&skills_dir)?;

    println!("nanobot-rs is ready.");
    println!("Next steps:");
    println!("1. Add your API key to {}", config_path.display());
    println!("2. Chat: nanobot-rs agent -m \"Hello!\"");
    Ok(())
}

fn cmd_status() -> Result<()> {
    let config_path = get_config_path()?;
    let config = load_config(Some(&config_path)).unwrap_or_default();
    let workspace = config.workspace_path();

    println!("nanobot-rs Status");
    println!(
        "Config: {} {}",
        config_path.display(),
        if config_path.exists() {
            "OK"
        } else {
            "MISSING"
        }
    );
    println!(
        "Workspace: {} {}",
        workspace.display(),
        if workspace.exists() { "OK" } else { "MISSING" }
    );
    println!("Model: {}", config.agents.defaults.model);

    let status = providers_status(&config);
    println!(
        "OpenRouter API: {}",
        if status
            .get("openrouter")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            "SET"
        } else {
            "NOT SET"
        }
    );
    println!(
        "Anthropic API: {}",
        if status
            .get("anthropic")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            "SET"
        } else {
            "NOT SET"
        }
    );
    println!(
        "OpenAI API: {}",
        if status
            .get("openai")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            "SET"
        } else {
            "NOT SET"
        }
    );
    println!(
        "Gemini API: {}",
        if status
            .get("gemini")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            "SET"
        } else {
            "NOT SET"
        }
    );
    println!(
        "MiniMax API: {}",
        if status
            .get("minimax")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            "SET"
        } else {
            "NOT SET"
        }
    );
    println!(
        "vLLM/Local: {}",
        if status
            .get("vllm")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            "SET"
        } else {
            "NOT SET"
        }
    );

    Ok(())
}

fn build_provider(config: &Config, model: &str, api_key: String) -> Arc<dyn LLMProvider> {
    let api_base = config.get_api_base(Some(model));
    let extra_headers = config
        .get_provider(Some(model))
        .and_then(|p| p.extra_headers.clone());
    let provider_name = config.get_provider_name(Some(model));
    Arc::new(LiteLLMProvider::new(
        api_key,
        api_base,
        model.to_string(),
        extra_headers,
        provider_name.as_deref(),
    ))
}

async fn cmd_gateway(port: u16, _verbose: bool) -> Result<()> {
    let config = load_config(None).unwrap_or_default();
    let model = config.agents.defaults.model.clone();
    let normalized_model = model.strip_prefix("litellm/").unwrap_or(&model);
    let is_bedrock = normalized_model.starts_with("bedrock/");
    let api_key = config.get_api_key(Some(&model));
    if api_key.is_none() && !is_bedrock {
        return Err(anyhow!("No API key configured."));
    }

    let bus = Arc::new(MessageBus::new(1024));
    let provider = build_provider(
        &config,
        &model,
        api_key.unwrap_or_else(|| "dummy".to_string()),
    );
    let session_manager = Arc::new(SessionManager::new()?);

    let cron_store_path = get_data_path()?.join("cron").join("jobs.json");
    let cron = Arc::new(CronService::new(cron_store_path));

    let agent = Arc::new(AgentLoop::new(
        bus.clone(),
        provider,
        config.workspace_path(),
        Some(model.clone()),
        config.agents.defaults.max_tool_iterations,
        config.agents.defaults.memory_window,
        config.tools.web.search.clone(),
        config.tools.exec.timeout,
        config.tools.restrict_to_workspace,
        Some(cron.clone()),
        Some(session_manager.clone()),
    )?);

    let bus_for_cron = bus.clone();
    let agent_for_cron = agent.clone();
    cron.set_on_job(Arc::new(move |job| {
        let bus = bus_for_cron.clone();
        let agent = agent_for_cron.clone();
        Box::pin(async move {
            let response = agent
                .process_direct(
                    &job.payload.message,
                    Some(&format!("cron:{}", job.id)),
                    job.payload.channel.as_deref(),
                    job.payload.to.as_deref(),
                )
                .await?;

            if job.payload.deliver {
                if let (Some(channel), Some(to)) =
                    (job.payload.channel.clone(), job.payload.to.clone())
                {
                    bus.publish_outbound(OutboundMessage::new(channel, to, response.clone()))
                        .await?;
                }
            }
            Ok(Some(response))
        })
    }))
    .await;
    cron.start().await?;

    let heartbeat = Arc::new(HeartbeatService::new(
        config.workspace_path(),
        DEFAULT_HEARTBEAT_INTERVAL_S,
        true,
    ));
    let agent_for_heartbeat = agent.clone();
    heartbeat
        .set_on_heartbeat(Arc::new(move |prompt| {
            let agent = agent_for_heartbeat.clone();
            Box::pin(async move {
                agent
                    .process_direct(&prompt, Some("heartbeat"), None, None)
                    .await
                    .unwrap_or_default()
            })
        }))
        .await;
    heartbeat.start().await;

    let channels = Arc::new(ChannelManager::new(&config, bus.clone()));
    let enabled_channels = channels.enabled_channels();
    if enabled_channels.is_empty() {
        println!("Warning: No channels enabled");
    } else {
        println!("Channels enabled: {}", enabled_channels.join(", "));
    }
    println!("Gateway started on port {port}");

    let agent_task = {
        let agent = agent.clone();
        tokio::spawn(async move {
            let _ = agent.run().await;
        })
    };
    let channels_task = {
        let channels = channels.clone();
        tokio::spawn(async move {
            channels.start_all().await;
        })
    };

    tokio::signal::ctrl_c().await?;
    println!("Shutting down...");
    agent.stop();
    heartbeat.stop().await;
    cron.stop().await;
    channels.stop_all().await;
    agent_task.abort();
    channels_task.abort();
    Ok(())
}

async fn cmd_agent(message: Option<String>, session: &str) -> Result<()> {
    let config = load_config(None).unwrap_or_default();
    let model = config.agents.defaults.model.clone();
    let normalized_model = model.strip_prefix("litellm/").unwrap_or(&model);
    let is_bedrock = normalized_model.starts_with("bedrock/");
    let api_key = config.get_api_key(Some(&model));
    if api_key.is_none() && !is_bedrock {
        println!("Error: No API key configured.");
        println!("Set one in ~/.nanobot/config.json under providers.*.apiKey");
        return Ok(());
    }

    let bus = Arc::new(MessageBus::new(1024));
    let provider = build_provider(
        &config,
        &model,
        api_key.unwrap_or_else(|| "dummy".to_string()),
    );
    let session_manager = Arc::new(SessionManager::new()?);
    let cron_store_path = get_data_path()?.join("cron").join("jobs.json");
    let cron = Arc::new(CronService::new(cron_store_path));
    let channels = Arc::new(ChannelManager::new(&config, bus.clone()));

    let agent_loop = Arc::new(AgentLoop::new(
        bus.clone(),
        provider,
        config.workspace_path(),
        Some(model.clone()),
        config.agents.defaults.max_tool_iterations,
        config.agents.defaults.memory_window,
        config.tools.web.search.clone(),
        config.tools.exec.timeout,
        config.tools.restrict_to_workspace,
        Some(cron.clone()),
        Some(session_manager.clone()),
    )?);

    let bus_for_cron = bus.clone();
    let agent_for_cron = agent_loop.clone();
    let channels_for_cron = channels.clone();
    cron.set_on_job(Arc::new(move |job| {
        let bus = bus_for_cron.clone();
        let agent = agent_for_cron.clone();
        let channels = channels_for_cron.clone();
        Box::pin(async move {
            let response = agent
                .process_direct(
                    &job.payload.message,
                    Some(&format!("cron:{}", job.id)),
                    job.payload.channel.as_deref(),
                    job.payload.to.as_deref(),
                )
                .await?;

            if job.payload.deliver
                && let (Some(channel), Some(to)) =
                    (job.payload.channel.clone(), job.payload.to.clone())
            {
                let outbound = OutboundMessage::new(channel.clone(), to, response.clone());
                if channel == "cli" {
                    println!("nanobot-rs[cron]: {response}");
                } else if let Some(adapter) = channels.get_channel(&channel) {
                    adapter.send(&outbound).await?;
                } else {
                    bus.publish_outbound(outbound).await?;
                }
            }
            Ok(Some(response))
        })
    }))
    .await;
    cron.start().await?;

    if let Some(content) = message {
        let response = agent_loop
            .process_direct(&content, Some(session), None, None)
            .await?;
        println!("nanobot-rs: {response}");
    } else {
        println!("nanobot-rs interactive mode (type exit/quit or Ctrl+C to exit)");
        let stdin = std::io::stdin();
        for line in stdin.lock().lines() {
            let input = line?;
            let command = input.trim();
            if command.is_empty() {
                continue;
            }
            if is_exit_command(command) {
                break;
            }
            let response = agent_loop
                .process_direct(&input, Some(session), None, None)
                .await?;
            println!("nanobot-rs: {response}");
        }
        println!("Goodbye!");
    }
    cron.stop().await;
    Ok(())
}

fn is_exit_command(command: &str) -> bool {
    matches!(
        command.to_ascii_lowercase().as_str(),
        "exit" | "quit" | "/exit" | "/quit" | ":q"
    )
}

fn resolve_service_name(config: &Config, name: Option<&str>) -> Result<String> {
    let resolved = name
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| {
            let configured = config.service.name.trim();
            if configured.is_empty() {
                None
            } else {
                Some(configured.to_string())
            }
        })
        .unwrap_or_else(|| "NanobotService".to_string());

    if resolved.is_empty() {
        return Err(anyhow!("service name cannot be empty"));
    }
    Ok(resolved)
}

fn persist_service_name_if_overridden(config: &mut Config, name: Option<&str>) -> Result<()> {
    if let Some(raw) = name {
        let normalized = raw.trim();
        if normalized.is_empty() {
            return Err(anyhow!("service name cannot be empty"));
        }
        if config.service.name != normalized {
            config.service.name = normalized.to_string();
            save_config(config, None)?;
            println!(
                "Saved service name '{}' to {}",
                normalized,
                get_config_path()?.display()
            );
        }
    }
    Ok(())
}

fn current_user_for_service() -> Result<String> {
    let username = std::env::var("USERNAME")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .ok_or_else(|| anyhow!("failed to detect current Windows username from USERNAME"))?;
    let domain = std::env::var("USERDOMAIN").unwrap_or_default();
    if domain.trim().is_empty() {
        Ok(format!(".\\{username}"))
    } else {
        Ok(format!("{}\\{}", domain.trim(), username))
    }
}

fn resolve_install_account(
    use_system: bool,
    use_current_user: bool,
    password: Option<String>,
) -> Result<ServiceAccount> {
    if use_system && use_current_user {
        return Err(anyhow!(
            "--system and --use-current-user are mutually exclusive"
        ));
    }

    if use_system {
        return Ok(ServiceAccount::LocalSystem);
    }

    if use_current_user {
        let resolved_password = password
            .or_else(|| std::env::var("NANOBOT_SERVICE_PASSWORD").ok())
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .ok_or_else(|| {
                anyhow!(
                    "missing password: use --password or set NANOBOT_SERVICE_PASSWORD when --use-current-user is enabled"
                )
            })?;
        let username = current_user_for_service()?;
        return Ok(ServiceAccount::CurrentUser {
            username,
            password: resolved_password,
        });
    }

    Ok(ServiceAccount::Inherit)
}

fn cmd_service(command: ServiceCommand) -> Result<()> {
    let mut config = load_config(None).unwrap_or_default();
    match command {
        ServiceCommand::Install {
            name,
            bin,
            args,
            workdir,
            system,
            use_current_user,
            password,
            auto_install_nssm,
            autostart,
        } => {
            let resolved_name = resolve_service_name(&config, name.as_deref())?;
            persist_service_name_if_overridden(&mut config, name.as_deref())?;
            let binary_path = match bin {
                Some(path) => path,
                None => std::env::current_exe()?,
            };
            let working_directory = match workdir {
                Some(path) => path,
                None => std::env::current_dir()?,
            };
            let account = resolve_install_account(system, use_current_user, password)?;
            let options = ServiceInstallOptions {
                name: resolved_name.clone(),
                binary_path,
                arguments: args,
                working_directory,
                log_directory: get_data_path()?.join("logs"),
                account,
                auto_install_nssm,
                autostart,
            };
            service::install_service(&options)?;
            println!("Service '{}' configured successfully.", resolved_name);
            println!("Use `nanobot-rs service start` to start it.");
        }
        ServiceCommand::Remove { name } => {
            let resolved_name = resolve_service_name(&config, name.as_deref())?;
            persist_service_name_if_overridden(&mut config, name.as_deref())?;
            service::remove_service(&resolved_name)?;
            println!("Service '{}' removed.", resolved_name);
        }
        ServiceCommand::Start { name } => {
            let resolved_name = resolve_service_name(&config, name.as_deref())?;
            persist_service_name_if_overridden(&mut config, name.as_deref())?;
            service::start_service(&resolved_name)?;
            println!("Service '{}' started.", resolved_name);
        }
        ServiceCommand::Stop { name } => {
            let resolved_name = resolve_service_name(&config, name.as_deref())?;
            persist_service_name_if_overridden(&mut config, name.as_deref())?;
            service::stop_service(&resolved_name)?;
            println!("Service '{}' stopped.", resolved_name);
        }
        ServiceCommand::Restart { name } => {
            let resolved_name = resolve_service_name(&config, name.as_deref())?;
            persist_service_name_if_overridden(&mut config, name.as_deref())?;
            service::restart_service(&resolved_name)?;
            println!("Service '{}' restarted.", resolved_name);
        }
        ServiceCommand::Status { name } => {
            let resolved_name = resolve_service_name(&config, name.as_deref())?;
            persist_service_name_if_overridden(&mut config, name.as_deref())?;
            let status = service::status_service(&resolved_name)?;
            if !status.exists {
                println!("Service '{}' is not installed.", resolved_name);
            } else {
                println!(
                    "Service '{}' state: {}",
                    resolved_name,
                    status.state.unwrap_or_else(|| "UNKNOWN".to_string())
                );
            }
        }
    }
    Ok(())
}

async fn cmd_channels(command: ChannelCommand) -> Result<()> {
    match command {
        ChannelCommand::Status => {
            let config = load_config(None).unwrap_or_default();
            println!("Channel Status");
            let tg_token = if config.channels.telegram.token.is_empty() {
                "not configured".to_string()
            } else {
                let prefix: String = config.channels.telegram.token.chars().take(10).collect();
                format!("{prefix}...")
            };
            println!(
                "Telegram: {} ({})",
                if config.channels.telegram.enabled {
                    "enabled"
                } else {
                    "disabled"
                },
                tg_token
            );
            println!(
                "WhatsApp: {} ({})",
                if config.channels.whatsapp.enabled {
                    "enabled"
                } else {
                    "disabled"
                },
                config.channels.whatsapp.bridge_url
            );
            let dc_token = if config.channels.discord.token.is_empty() {
                "not configured".to_string()
            } else {
                let prefix: String = config.channels.discord.token.chars().take(8).collect();
                format!("{prefix}...")
            };
            println!(
                "Discord: {} (gateway={}, token={})",
                if config.channels.discord.enabled {
                    "enabled"
                } else {
                    "disabled"
                },
                config.channels.discord.gateway_url,
                dc_token
            );
            let fs_app = if config.channels.feishu.app_id.is_empty() {
                "not configured".to_string()
            } else {
                let prefix: String = config.channels.feishu.app_id.chars().take(8).collect();
                format!("{prefix}...")
            };
            println!(
                "Feishu: {} (app_id={})",
                if config.channels.feishu.enabled {
                    "enabled"
                } else {
                    "disabled"
                },
                fs_app
            );
            let mochat_base = if config.channels.mochat.claw_token.is_empty() {
                "not configured".to_string()
            } else {
                config.channels.mochat.base_url.clone()
            };
            println!(
                "Mochat: {} (base_url={})",
                if config.channels.mochat.enabled {
                    "enabled"
                } else {
                    "disabled"
                },
                mochat_base
            );
            let dt_client = if config.channels.dingtalk.client_id.is_empty() {
                "not configured".to_string()
            } else {
                let prefix: String = config.channels.dingtalk.client_id.chars().take(8).collect();
                format!("{prefix}...")
            };
            println!(
                "DingTalk: {} (client_id={})",
                if config.channels.dingtalk.enabled {
                    "enabled"
                } else {
                    "disabled"
                },
                dt_client
            );
            let email_user = if config.channels.email.imap_username.is_empty() {
                "not configured".to_string()
            } else {
                let prefix: String = config
                    .channels
                    .email
                    .imap_username
                    .chars()
                    .take(12)
                    .collect();
                format!("{prefix}...")
            };
            println!(
                "Email: {} (consent={}, imap_user={})",
                if config.channels.email.enabled {
                    "enabled"
                } else {
                    "disabled"
                },
                if config.channels.email.consent_granted {
                    "granted"
                } else {
                    "not granted"
                },
                email_user
            );
            let slack_mode = if config.channels.slack.bot_token.is_empty()
                || config.channels.slack.app_token.is_empty()
            {
                "not configured".to_string()
            } else {
                config.channels.slack.mode.clone()
            };
            println!(
                "Slack: {} ({})",
                if config.channels.slack.enabled {
                    "enabled"
                } else {
                    "disabled"
                },
                slack_mode
            );
            let qq_app = if config.channels.qq.app_id.is_empty() {
                "not configured".to_string()
            } else {
                let prefix: String = config.channels.qq.app_id.chars().take(8).collect();
                format!("{prefix}...")
            };
            println!(
                "QQ: {} (app_id={})",
                if config.channels.qq.enabled {
                    "enabled"
                } else {
                    "disabled"
                },
                qq_app
            );
        }
        ChannelCommand::Login => {
            cmd_channels_login().await?;
        }
    }
    Ok(())
}

async fn cmd_channels_login() -> Result<()> {
    let config = load_config(None).unwrap_or_default();
    let bridge_dir = prepare_bridge_dir().await?;
    println!("Starting WhatsApp bridge...");
    println!("Scan the QR code in the terminal to connect.\n");
    let mut env_vars = Vec::new();
    if !config.channels.whatsapp.bridge_token.is_empty() {
        env_vars.push((
            "BRIDGE_TOKEN".to_string(),
            config.channels.whatsapp.bridge_token.clone(),
        ));
    }
    run_npm(&["start"], &bridge_dir, &env_vars).await
}

async fn prepare_bridge_dir() -> Result<PathBuf> {
    let user_bridge = get_data_path()?.join("bridge");
    if user_bridge.join("dist").join("index.js").exists() {
        return Ok(user_bridge);
    }

    if which("npm").is_err() {
        return Err(anyhow!("npm not found. Please install Node.js >= 18."));
    }

    let source = find_bridge_source().ok_or_else(|| {
        anyhow!("bridge source not found. Expected a bridge/ directory with package.json")
    })?;

    println!("Setting up WhatsApp bridge from {}", source.display());
    if user_bridge.exists() {
        fs::remove_dir_all(&user_bridge)?;
    }
    copy_bridge_tree(&source, &user_bridge)?;

    println!("Installing bridge dependencies...");
    run_npm(&["install"], &user_bridge, &[] as &[(String, String)]).await?;
    println!("Building bridge...");
    run_npm(&["run", "build"], &user_bridge, &[] as &[(String, String)]).await?;
    println!("Bridge ready at {}\n", user_bridge.display());
    Ok(user_bridge)
}

fn find_bridge_source() -> Option<PathBuf> {
    let mut candidates = Vec::new();
    let manifest_bridge = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("bridge");
    candidates.push(manifest_bridge);

    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd.join("bridge"));
        candidates.push(cwd.join("..").join("bridge"));
        candidates.push(cwd.join("..").join("nanobot").join("bridge"));
    }

    if let Ok(exe) = std::env::current_exe()
        && let Some(exe_dir) = exe.parent()
    {
        candidates.push(exe_dir.join("bridge"));
        candidates.push(exe_dir.join("..").join("bridge"));
    }

    candidates
        .into_iter()
        .find(|p| p.join("package.json").exists() && p.join("src").exists())
}

fn copy_bridge_tree(src: &Path, dst: &Path) -> Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if name_str == "node_modules" || name_str == "dist" {
            continue;
        }

        let src_path = entry.path();
        let dst_path = dst.join(&name);
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            copy_bridge_tree(&src_path, &dst_path)?;
        } else if file_type.is_file() {
            if let Some(parent) = dst_path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(&src_path, &dst_path)?;
        }
    }
    Ok(())
}

async fn run_npm(args: &[&str], cwd: &Path, env_vars: &[(String, String)]) -> Result<()> {
    let mut command = Command::new("npm");
    command
        .args(args)
        .current_dir(cwd)
        .stdin(std::process::Stdio::inherit())
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit());

    for (key, value) in env_vars {
        command.env(key, value);
    }

    let status = command.status().await?;
    if status.success() {
        Ok(())
    } else {
        Err(anyhow!(
            "npm {} failed with status: {status}",
            args.join(" ")
        ))
    }
}

async fn cmd_cron(command: CronCommand) -> Result<()> {
    let store_path = get_data_path()?.join("cron").join("jobs.json");
    let cron = Arc::new(CronService::new(store_path));
    let _ = cron.start().await;

    match command {
        CronCommand::List { all } => {
            let jobs = cron.list_jobs(all).await;
            if jobs.is_empty() {
                println!("No scheduled jobs.");
            } else {
                for job in jobs {
                    let schedule = match job.schedule.kind.as_str() {
                        "every" => format!("every {}s", job.schedule.every_ms.unwrap_or(0) / 1000),
                        "cron" => job.schedule.expr.unwrap_or_default(),
                        "at" => format!("at {}", job.schedule.at_ms.unwrap_or_default()),
                        _ => "unknown".to_string(),
                    };
                    println!(
                        "{} {} [{}] next={}",
                        job.id,
                        job.name,
                        schedule,
                        job.state.next_run_at_ms.unwrap_or_default()
                    );
                }
            }
        }
        CronCommand::Add {
            name,
            message,
            every,
            cron: cron_expr,
            at,
            deliver,
            to,
            channel,
        } => {
            let schedule = if let Some(every) = every {
                CronSchedule {
                    kind: "every".to_string(),
                    every_ms: Some(every * 1000),
                    ..Default::default()
                }
            } else if let Some(expr) = cron_expr {
                CronSchedule {
                    kind: "cron".to_string(),
                    expr: Some(expr),
                    ..Default::default()
                }
            } else if let Some(at) = at {
                let ts = chrono::DateTime::parse_from_rfc3339(&at)
                    .map_err(|e| anyhow!("invalid --at value: {e}"))?;
                CronSchedule {
                    kind: "at".to_string(),
                    at_ms: Some(ts.timestamp_millis()),
                    ..Default::default()
                }
            } else {
                return Err(anyhow!("Must specify --every, --cron, or --at"));
            };

            let job = cron
                .add_job(name, schedule, message, deliver, channel, to, false)
                .await?;
            println!("Added job '{}' ({})", job.name, job.id);
        }
        CronCommand::Remove { job_id } => {
            if cron.remove_job(&job_id).await? {
                println!("Removed job {job_id}");
            } else {
                println!("Job {job_id} not found");
            }
        }
        CronCommand::Enable { job_id, disable } => {
            match cron.enable_job(&job_id, !disable).await? {
                Some(job) => {
                    println!(
                        "Job '{}' {}",
                        job.name,
                        if disable { "disabled" } else { "enabled" }
                    );
                }
                None => println!("Job {job_id} not found"),
            }
        }
        CronCommand::Run { job_id, force } => {
            let config = load_config(None).unwrap_or_default();
            let model = config.agents.defaults.model.clone();
            let normalized_model = model.strip_prefix("litellm/").unwrap_or(&model);
            let is_bedrock = normalized_model.starts_with("bedrock/");
            let api_key = config.get_api_key(Some(&model));
            if api_key.is_none() && !is_bedrock {
                return Err(anyhow!(
                    "No API key configured. Set one in ~/.nanobot/config.json under providers.*.apiKey"
                ));
            }

            let bus = Arc::new(MessageBus::new(1024));
            let provider = build_provider(
                &config,
                &model,
                api_key.unwrap_or_else(|| "dummy".to_string()),
            );
            let session_manager = Arc::new(SessionManager::new()?);
            let channels = Arc::new(ChannelManager::new(&config, bus.clone()));
            let agent = Arc::new(AgentLoop::new(
                bus.clone(),
                provider,
                config.workspace_path(),
                Some(model),
                config.agents.defaults.max_tool_iterations,
                config.agents.defaults.memory_window,
                config.tools.web.search.clone(),
                config.tools.exec.timeout,
                config.tools.restrict_to_workspace,
                Some(cron.clone()),
                Some(session_manager),
            )?);

            let bus_for_cron = bus.clone();
            let agent_for_cron = agent.clone();
            let channels_for_cron = channels.clone();
            cron.set_on_job(Arc::new(move |job| {
                let bus = bus_for_cron.clone();
                let agent = agent_for_cron.clone();
                let channels = channels_for_cron.clone();
                Box::pin(async move {
                    let response = agent
                        .process_direct(
                            &job.payload.message,
                            Some(&format!("cron:{}", job.id)),
                            job.payload.channel.as_deref(),
                            job.payload.to.as_deref(),
                        )
                        .await?;

                    if job.payload.deliver
                        && let (Some(channel), Some(to)) =
                            (job.payload.channel.clone(), job.payload.to.clone())
                    {
                        let outbound = OutboundMessage::new(channel.clone(), to, response.clone());
                        if channel == "cli" {
                            println!("nanobot-rs[cron]: {response}");
                        } else if let Some(adapter) = channels.get_channel(&channel) {
                            adapter.send(&outbound).await?;
                        } else {
                            bus.publish_outbound(outbound).await?;
                        }
                    }
                    Ok(Some(response))
                })
            }))
            .await;

            if cron.run_job(&job_id, force).await? {
                println!("Job executed");
            } else {
                println!("Failed to run job {job_id}");
            }
        }
    }

    cron.stop().await;
    Ok(())
}
