# nanobot-rs

中文文档: [README.md](README.md)

## Ultra-Lightweight Personal AI Assistant, Rust Edition

`nanobot-rs` is the Rust version of [`HKUDS/nanobot`](https://github.com/HKUDS/nanobot), keeping the same ultra-lightweight agent philosophy and tool-driven workflow.

- Rust rewrite for stronger concurrency stability and cleaner deployment ergonomics
- Already integrated by [`open-vibe/open-vibe`](https://github.com/open-vibe/open-vibe) as its Rust implementation of `nanobot`
- Evolving as one of the core runtimes for `open-vibe`
- Inspired by [`OpenClaw`](https://github.com/openclaw/openclaw)

## Open Vibe Integration

- Current Open Vibe integration focus: DingTalk stream bridge + relay workflow into Open Vibe threads

## Features

- Agent loop: LLM calls, tool execution, session context, and error handling
- Config system: `~/.nanobot/config.json` with provider auto-matching
- Session and memory: JSONL session persistence + `memory/MEMORY.md`
- Media-aware prompting: inbound image attachments are converted to OpenAI-compatible `image_url` content parts
- Tooling:
  - `read_file` / `write_file` / `edit_file` / `list_dir`
  - `exec`
  - `web_search` / `web_fetch` / `http_request`
  - `message` / `spawn` / `cron`
  - `spawn` subagents include current-time context, `edit_file` capability, and `skills/` path guidance
- Scheduling and heartbeat:
  - `CronService` (add/list/remove/enable/run + persistence)
  - `HeartbeatService`
- Multi-channel support:
  - Telegram (long polling, media download, voice transcription)
  - Discord (Gateway + REST, with typing indicator)
  - WhatsApp (Node bridge)
  - Feishu (REST send; optional WebSocket receive feature)
  - Mochat (Claw IM via HTTP watch/polling)
  - DingTalk (optional Stream receive feature)
  - Email (IMAP inbound + SMTP outbound, explicit consent required)
  - Slack (Socket Mode)
  - QQ (optional feature `qq-botrs`)
- Built-in skills synced from the original project (`skills/*`)

## Requirements

- Rust stable (recommended 1.85+)
- Optional:
  - Node.js 18+ (for WhatsApp bridge login)
  - Brave Search API key (`web_search`, optional; falls back to keyless DuckDuckGo search when missing)
  - Groq API key (audio transcription)

## Quick Start

### 1. Initialize

```bash
cargo run -- onboard
```

This initializes workspace basics including `memory/MEMORY.md` and `skills/` for custom local skills.

### 2. Configure API key

Edit `~/.nanobot/config.json`:

```json
{
  "providers": {
    "openai": {
      "apiKey": "sk-xxx"
    },
    "openrouter": {
      "apiKey": "sk-or-xxx",
      "extraHeaders": {
        "HTTP-Referer": "https://example.com",
        "X-Title": "nanobot-rs"
      }
    }
  },
  "agents": {
    "defaults": {
      "model": "gpt-4o-mini"
    }
  }
}
```

For MiniMax, add a `providers.minimax` section and use a model containing `minimax` (for example `minimax/MiniMax-M2.1`):

```json
{
  "providers": {
    "minimax": {
      "apiKey": "minimax-xxx"
    }
  },
  "agents": {
    "defaults": {
      "model": "minimax/MiniMax-M2.1"
    }
  }
}
```

If your key is from MiniMax Mainland China (minimaxi.com), set:

```json
{
  "providers": {
    "minimax": {
      "apiBase": "https://api.minimaxi.com/v1"
    }
  }
}
```

`nanobot-rs` now follows the Python `nanobot` LiteLLM-style routing. You can set the model directly (no `litellm/` prefix required), for example:

```json
{
  "agents": {
    "defaults": {
      "model": "anthropic/claude-3-7-sonnet"
    }
  }
}
```

`web_search` prefers Brave when a key is configured, and automatically falls back to keyless DuckDuckGo when no `BRAVE_API_KEY` is available.  
`web_fetch` remains keyless and can fetch/extract content from a concrete URL directly.
`http_request` can call APIs directly (`GET/POST/PUT/PATCH/DELETE`, headers, query, json/body), including localhost ports and LAN services.

To switch `web_search` provider (Perplexity / Grok), configure `tools.web.search`:

```json
{
  "tools": {
    "web": {
      "search": {
        "provider": "perplexity",
        "maxResults": 5,
        "perplexity": {
          "apiKey": "pplx-xxx",
          "baseUrl": "https://api.perplexity.ai",
          "model": "perplexity/sonar-pro"
        }
      }
    }
  }
}
```

Grok example:

```json
{
  "tools": {
    "web": {
      "search": {
        "provider": "grok",
        "grok": {
          "apiKey": "xai-xxx",
          "model": "grok-4-1-fast",
          "inlineCitations": true
        }
      }
    }
  }
}
```

If you use DingTalk, add this under `channels`:

```json
{
  "channels": {
    "dingtalk": {
      "enabled": true,
      "clientId": "dingxxx",
      "clientSecret": "secretxxx",
      "allowFrom": []
    }
  }
}
```

If you use the Email channel (IMAP + SMTP):

```json
{
  "channels": {
    "email": {
      "enabled": true,
      "consentGranted": true,
      "imapHost": "imap.gmail.com",
      "imapPort": 993,
      "imapUsername": "you@gmail.com",
      "imapPassword": "app-password",
      "smtpHost": "smtp.gmail.com",
      "smtpPort": 587,
      "smtpUsername": "you@gmail.com",
      "smtpPassword": "app-password",
      "smtpUseTls": true,
      "fromAddress": "you@gmail.com",
      "allowFrom": ["trusted@example.com"]
    }
  }
}
```

If you use the Slack channel (Socket Mode):

```json
{
  "channels": {
    "slack": {
      "enabled": true,
      "mode": "socket",
      "botToken": "xoxb-...",
      "appToken": "xapp-...",
      "groupPolicy": "mention",
      "groupAllowFrom": [],
      "dm": {
        "enabled": true,
        "policy": "open",
        "allowFrom": []
      }
    }
  }
}
```

If you use the QQ channel (currently direct/private chat only):

```json
{
  "channels": {
    "qq": {
      "enabled": true,
      "appId": "your-app-id",
      "secret": "your-secret",
      "allowFrom": []
    }
  }
}
```

If you use the Mochat channel (Claw IM):

```json
{
  "channels": {
    "mochat": {
      "enabled": true,
      "baseUrl": "https://mochat.io",
      "clawToken": "claw_xxx",
      "agentUserId": "6982abcdef",
      "sessions": ["*"],
      "panels": ["*"],
      "allowFrom": [],
      "replyDelayMode": "non-mention",
      "replyDelayMs": 120000
    }
  }
}
```

### 3. Chat directly

```bash
cargo run -- agent -m "Hello"
```

### 4. Start gateway

```bash
cargo run -- gateway
```

## Windows Service (NSSM)

`nanobot-rs` can run as a Windows background service via `nssm`, with built-in commands:

- `service install`
- `service remove`
- `service start`
- `service stop`
- `service restart`
- `service status`

Build release first (with the features you need):

```powershell
cargo build --release --all-features
```

Install service (default service name: `NanobotService`, default args: `gateway`):

```powershell
.\target\release\nanobot.exe service install
```

Optional service name override:

```powershell
.\target\release\nanobot.exe service install --name NanobotService2
```

When `--name` is provided, the value is persisted to `service.name` in `~/.nanobot/config.json`, so later `start/stop/status` can omit `--name`.

### Service Account Modes

1. Use `LocalSystem`:

```powershell
.\target\release\nanobot.exe service install --system
```

2. Use current user (recommended, easier access to your user-scoped `~/.nanobot/config.json`):

```powershell
.\target\release\nanobot.exe service install --use-current-user --password "YourWindowsPassword"
```

Or use environment variable to avoid plaintext password in command history:

```powershell
$env:NANOBOT_SERVICE_PASSWORD="YourWindowsPassword"
.\target\release\nanobot.exe service install --use-current-user
Remove-Item Env:NANOBOT_SERVICE_PASSWORD
```

### Common Service Commands

```powershell
.\target\release\nanobot.exe service status
.\target\release\nanobot.exe service start
.\target\release\nanobot.exe service stop
.\target\release\nanobot.exe service restart
.\target\release\nanobot.exe service remove
```

### Notes

- Use an elevated (Administrator) PowerShell for service install/start/stop/remove.
- For `--use-current-user`, password must be the Windows account password (not PIN).
- `Error 1069` usually means invalid service credentials or missing "Log on as a service" permission.
- If you see "marked for deletion", close `services.msc` / Event Viewer, wait a few seconds, and retry. Reboot if needed.

## Common Commands

```bash
# Status and version
cargo run -- status
cargo run -- version

# Interactive mode
cargo run -- agent

# Channels
cargo run -- channels status
cargo run -- channels login

# Cron jobs
cargo run -- cron list
cargo run -- cron add -n daily -m "Good morning" --cron "0 9 * * *"
cargo run -- cron enable <job_id>
cargo run -- cron run <job_id>
cargo run -- cron remove <job_id>
```

Interactive exit commands: `exit`, `quit`, `/exit`, `/quit`, `:q`, or `Ctrl+C`/`Ctrl+D`.

## Feishu WebSocket Receive

Default build supports Feishu sending. To enable Feishu WebSocket receive:

```bash
cargo run --features feishu-websocket -- gateway
```

## DingTalk Stream Receive

Default builds do not include DingTalk Stream. Enable it with:

```bash
cargo run --features dingtalk-stream -- gateway
```

## Mochat Channel (Claw IM)

Disabled by default. Once enabled, nanobot-rs uses HTTP watch/polling to receive and send messages.

1. Optional: ask nanobot to set up Mochat automatically
- In agent mode, send this prompt (replace the email with yours):

```text
Read https://raw.githubusercontent.com/HKUDS/MoChat/refs/heads/main/skills/nanobot/skill.md and register on MoChat. My Email account is xxx@xxx Bind me as your owner and DM me on MoChat.
```

- nanobot will try to register and write Mochat settings into `~/.nanobot/config.json`.

2. Manual setup (recommended to verify config)
- Configure `channels.mochat` in `~/.nanobot/config.json`:
- `clawToken`: required, sent as `X-Claw-Token` for Mochat API requests
- `sessions` / `panels`: explicit IDs or `["*"]` for auto discovery
- `groups` + `mention.requireInGroups`: group mention policy

```json
{
  "channels": {
    "mochat": {
      "enabled": true,
      "baseUrl": "https://mochat.io",
      "socketUrl": "https://mochat.io",
      "socketPath": "/socket.io",
      "clawToken": "claw_xxx",
      "agentUserId": "6982abcdef",
      "sessions": ["*"],
      "panels": ["*"],
      "replyDelayMode": "non-mention",
      "replyDelayMs": 120000
    }
  }
}
```

3. Start gateway:

```bash
cargo run -- gateway
```

4. Validate messaging:
- Direct sessions use `session_xxx` targets
- Group/panel messaging uses panel/group targets

## QQ Channel (Direct/Private Chat Only)

QQ support is disabled by default; enable it via the `qq-botrs` feature.

1. Register and create a bot
- Go to [QQ Open Platform](https://q.qq.com), register as a developer, and create a bot app
- Copy `AppID` and `AppSecret` from Developer Settings

2. Configure sandbox for testing
- Open sandbox settings in the bot console
- Add your QQ account as a test member
- Scan the bot QR code with mobile QQ and start a direct chat

3. Configure `~/.nanobot/config.json`
- Use the `qq` snippet above with `appId` and `secret`
- Leave `allowFrom` empty for open access, or set allowed user openids from logs

4. Start gateway

```bash
cargo run --features qq-botrs -- gateway
```

After startup, send a direct QQ message to the bot and it should reply.

## Slack Channel

Uses Socket Mode, so no public callback URL is required.

1. Create a Slack app
- Go to [Slack API](https://api.slack.com/apps) -> Create New App -> From scratch
- Select a workspace and create the app

2. Configure app capabilities
- Socket Mode: enable it and create an App-Level Token (`connections:write`, starts with `xapp-...`)
- OAuth & Permissions: add bot scopes `chat:write`, `reactions:write`, `app_mentions:read`
- Event Subscriptions: enable and subscribe to `message.im`, `message.channels`, `app_mention`
- App Home: enable Messages Tab and allow messaging from that tab
- Install App: install to workspace and copy Bot Token (`xoxb-...`)

3. Configure `~/.nanobot/config.json`

```json
{
  "channels": {
    "slack": {
      "enabled": true,
      "mode": "socket",
      "botToken": "xoxb-...",
      "appToken": "xapp-...",
      "groupPolicy": "mention",
      "groupAllowFrom": [],
      "dm": {
        "enabled": true,
        "policy": "open",
        "allowFrom": []
      }
    }
  }
}
```

4. Start gateway

```bash
cargo run -- gateway
```

You can DM the bot directly, or @mention it in a channel.

## WhatsApp Login

`channels login` will automatically:

- Prepare `~/.nanobot/bridge`
- Run `npm install`
- Run `npm run build`
- Start bridge and print QR login flow in terminal

## Development

```bash
cargo fmt
cargo test
cargo check --features feishu-websocket
cargo check --features dingtalk-stream
cargo check --features qq-botrs
```

## License

MIT
