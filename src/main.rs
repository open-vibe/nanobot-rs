use anyhow::{Result, anyhow};
use clap::{Parser, Subcommand};
use nanobot::VERSION;
use nanobot::agent::AgentLoop;
use nanobot::bus::{MessageBus, OutboundMessage};
use nanobot::channels::manager::ChannelManager;
use nanobot::config::{Config, get_config_path, load_config, providers_status, save_config};
use nanobot::cron::{CronSchedule, CronService};
use nanobot::heartbeat::{DEFAULT_HEARTBEAT_INTERVAL_S, HeartbeatService};
use nanobot::providers::openai::OpenAIProvider;
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
        #[arg(short, long, default_value = "cli:default")]
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

async fn cmd_gateway(port: u16, _verbose: bool) -> Result<()> {
    let config = load_config(None).unwrap_or_default();
    let model = config.agents.defaults.model.clone();
    let is_bedrock = model.starts_with("bedrock/");
    let api_key = config.get_api_key(Some(&model));
    if api_key.is_none() && !is_bedrock {
        return Err(anyhow!("No API key configured."));
    }

    let bus = Arc::new(MessageBus::new(1024));
    let provider = Arc::new(OpenAIProvider::new(
        api_key.unwrap_or_else(|| "dummy".to_string()),
        config.get_api_base(Some(&model)),
        model.clone(),
        config
            .get_provider(Some(&model))
            .and_then(|p| p.extra_headers.clone()),
    ));
    let session_manager = Arc::new(SessionManager::new()?);

    let cron_store_path = get_data_path()?.join("cron").join("jobs.json");
    let cron = Arc::new(CronService::new(cron_store_path));

    let agent = Arc::new(AgentLoop::new(
        bus.clone(),
        provider,
        config.workspace_path(),
        Some(model.clone()),
        config.agents.defaults.max_tool_iterations,
        if config.tools.web.search.api_key.is_empty() {
            None
        } else {
            Some(config.tools.web.search.api_key.clone())
        },
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

    let channels = Arc::new(ChannelManager::new(
        &config,
        bus.clone(),
        Some(session_manager),
    ));
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
    let is_bedrock = model.starts_with("bedrock/");
    let api_key = config.get_api_key(Some(&model));
    if api_key.is_none() && !is_bedrock {
        println!("Error: No API key configured.");
        println!("Set one in ~/.nanobot/config.json under providers.*.apiKey");
        return Ok(());
    }

    let bus = Arc::new(MessageBus::new(1024));
    let provider = Arc::new(OpenAIProvider::new(
        api_key.unwrap_or_else(|| "dummy".to_string()),
        config.get_api_base(Some(&model)),
        model.clone(),
        config
            .get_provider(Some(&model))
            .and_then(|p| p.extra_headers.clone()),
    ));
    let agent_loop = AgentLoop::new(
        bus,
        provider,
        config.workspace_path(),
        Some(model),
        config.agents.defaults.max_tool_iterations,
        if config.tools.web.search.api_key.is_empty() {
            None
        } else {
            Some(config.tools.web.search.api_key.clone())
        },
        config.tools.exec.timeout,
        config.tools.restrict_to_workspace,
        None,
        None,
    )?;

    if let Some(content) = message {
        let response = agent_loop
            .process_direct(&content, Some(session), None, None)
            .await?;
        println!("nanobot-rs: {response}");
    } else {
        println!("nanobot-rs interactive mode (Ctrl+C to exit)");
        let stdin = std::io::stdin();
        for line in stdin.lock().lines() {
            let input = line?;
            if input.trim().is_empty() {
                continue;
            }
            let response = agent_loop
                .process_direct(&input, Some(session), None, None)
                .await?;
            println!("nanobot-rs: {response}");
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
        }
        ChannelCommand::Login => {
            cmd_channels_login().await?;
        }
    }
    Ok(())
}

async fn cmd_channels_login() -> Result<()> {
    let bridge_dir = prepare_bridge_dir().await?;
    println!("Starting WhatsApp bridge...");
    println!("Scan the QR code in the terminal to connect.\n");
    run_npm(&["start"], &bridge_dir).await
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
    run_npm(&["install"], &user_bridge).await?;
    println!("Building bridge...");
    run_npm(&["run", "build"], &user_bridge).await?;
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

async fn run_npm(args: &[&str], cwd: &Path) -> Result<()> {
    let status = Command::new("npm")
        .args(args)
        .current_dir(cwd)
        .stdin(std::process::Stdio::inherit())
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit())
        .status()
        .await?;
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
