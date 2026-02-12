# nanobot-rs

English: [README.en.md](README.en.md)

## Ultra-Lightweight Personal AI Assistant, Rust Edition

`nanobot-rs` 是 [`HKUDS/nanobot`](https://github.com/HKUDS/nanobot) 的 Rust 版本，延续 ultra-lightweight 的 Agent 设计与工具工作流。

- Rust 重写：更稳定的并发执行、更清晰的部署和工程化体验
- 已由 [`open-vibe/open-vibe`](https://github.com/open-vibe/open-vibe) 接入，作为其 `nanobot` Rust 实现
- 作为 `open-vibe` 后续核心运行时之一持续演进
- 灵感来源于 [`OpenClaw`](https://github.com/openclaw/openclaw)

## Open Vibe 集成

- Open Vibe 当前集成重点：DingTalk stream bridge + relay workflow 到 Open Vibe threads

## 特性

- Agent 主循环：LLM 调用、工具调用、会话上下文、错误恢复
- 配置系统：`~/.nanobot/config.json`，支持 provider 自动匹配
- 会话与记忆：JSONL 会话持久化 + `memory/MEMORY.md`
- 多模态输入：会将入站图片附件转换为 OpenAI 兼容的 `image_url` 内容片段
- 工具系统：
  - `read_file` / `write_file` / `edit_file` / `list_dir`
  - `exec`
  - `web_search` / `web_fetch` / `http_request`
  - `message` / `spawn` / `cron`
  - `spawn` 子代理具备当前时间上下文、`edit_file` 能力与 `skills/` 路径提示
- 定时任务与心跳：
  - `CronService`（add/list/remove/enable/run + 持久化）
  - `HeartbeatService`
- 多渠道接入：
  - Telegram（long polling，支持媒体下载与语音转写）
  - Discord（Gateway + REST，支持 typing 指示）
  - WhatsApp（Node bridge）
  - Feishu（REST 发送；WebSocket 接收可选特性）
  - Mochat（Claw IM，HTTP watch/polling）
  - DingTalk（Stream 接收可选特性）
  - Email（IMAP 收信 + SMTP 发信，需显式 consent）
  - Slack（Socket Mode）
  - QQ（可选特性，`qq-botrs`）
- 内置 skills：同步原项目 `skills/*`

## 环境要求

- Rust stable（建议 1.85+）
- 可选：
  - Node.js 18+（WhatsApp bridge 登录）
  - Brave Search API Key（`web_search`，可选；未配置时自动降级到 DuckDuckGo 无 key 搜索）
  - Groq API Key（语音转写）

## 快速开始

### 1. 初始化

```bash
cargo run -- onboard
```

该步骤会初始化工作区基础结构，包括 `memory/MEMORY.md` 与用于本地自定义技能的 `skills/` 目录。

### 2. 配置 API Key

编辑 `~/.nanobot/config.json`，最小配置示例：

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

如需使用 MiniMax，可在 `providers.minimax` 中配置密钥，并将模型设置为包含 `minimax` 的名称（例如 `minimax/MiniMax-M2.1`）：

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

如果你的密钥来自 MiniMax 中国大陆平台（minimaxi.com），请设置：

```json
{
  "providers": {
    "minimax": {
      "apiBase": "https://api.minimaxi.com/v1"
    }
  }
}
```

`nanobot-rs` 现在按 Python 版 `nanobot` 的 LiteLLM 路由方式工作。你可以直接填写模型（不再需要 `litellm/` 前缀），例如：

```json
{
  "agents": {
    "defaults": {
      "model": "anthropic/claude-3-7-sonnet"
    }
  }
}
```

`web_search` 默认优先使用 Brave（若配置了 key）；未配置 `BRAVE_API_KEY` 时会自动使用 DuckDuckGo 无 key 兜底。  
`web_fetch` 一直可用，可直接抓取指定 URL 的正文内容。
`http_request` 可直接发起 API 请求（支持 `GET/POST/PUT/PATCH/DELETE`、headers、query、json/body），适合访问本机端口或内网服务。

如需切换 `web_search` provider（Perplexity / Grok），可在 `tools.web.search` 配置：

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

Grok 配置示例：

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

如需使用钉钉，还可在 `channels` 中增加：

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

如需使用 Email 通道（IMAP + SMTP）：

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

如需使用 Slack 通道（Socket Mode）：

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

如需使用 QQ 通道（当前仅支持单聊）：

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

如需使用 Mochat 通道（Claw IM）：

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

### 3. 直接对话

```bash
cargo run -- agent -m "Hello"
```

### 4. 启动网关

```bash
cargo run -- gateway
```

## Windows 服务（NSSM）

`nanobot-rs` 支持通过 `nssm` 注册为 Windows 后台服务，并提供统一命令：

- `service install`
- `service remove`
- `service start`
- `service stop`
- `service restart`
- `service status`

先构建 release（建议带上你需要的功能特性）：

```powershell
cargo build --release --all-features
```

安装服务（默认服务名：`NanobotService`，默认参数：`gateway`）：

```powershell
.\target\release\nanobot.exe service install
```

服务名可选覆盖：

```powershell
.\target\release\nanobot.exe service install --name NanobotService2
```

当你传入 `--name` 时，程序会把该名字写入 `~/.nanobot/config.json` 的 `service.name`，后续 `start/stop/status` 可直接省略 `--name`。

### 服务账号模式

1. 使用 `LocalSystem`（系统账号）：

```powershell
.\target\release\nanobot.exe service install --system
```

2. 使用当前用户（推荐，便于读取你用户目录下的 `~/.nanobot/config.json`）：

```powershell
.\target\release\nanobot.exe service install --use-current-user --password "你的Windows登录密码"
```

也可用环境变量避免命令行明文密码：

```powershell
$env:NANOBOT_SERVICE_PASSWORD="你的Windows登录密码"
.\target\release\nanobot.exe service install --use-current-user
Remove-Item Env:NANOBOT_SERVICE_PASSWORD
```

### 常用服务命令

```powershell
.\target\release\nanobot.exe service status
.\target\release\nanobot.exe service start
.\target\release\nanobot.exe service stop
.\target\release\nanobot.exe service restart
.\target\release\nanobot.exe service remove
```

### 注意事项

- 请使用“管理员 PowerShell”执行服务安装/启停/删除。
- `--use-current-user` 的密码是 Windows 登录密码，不是 PIN。
- `Error 1069` 通常表示服务登录凭据错误或缺少“作为服务登录”权限。
- 如果提示“服务已标记为删除”，请关闭 `services.msc` 等窗口后稍等重试；必要时重启系统。

## 常用命令

```bash
# 状态与版本
cargo run -- status
cargo run -- version

# 交互模式
cargo run -- agent

# 渠道
cargo run -- channels status
cargo run -- channels login

# 定时任务
cargo run -- cron list
cargo run -- cron add -n daily -m "Good morning" --cron "0 9 * * *"
cargo run -- cron enable <job_id>
cargo run -- cron run <job_id>
cargo run -- cron remove <job_id>
```

交互模式退出命令：`exit`、`quit`、`/exit`、`/quit`、`:q`，或 `Ctrl+C`/`Ctrl+D`。

## Feishu WebSocket 接收

默认构建下可正常发送消息。要启用 Feishu WebSocket 接收：

```bash
cargo run --features feishu-websocket -- gateway
```

## DingTalk Stream 接收

默认构建不启用钉钉 Stream。要启用钉钉接收：

```bash
cargo run --features dingtalk-stream -- gateway
```

## Mochat 通道（Claw IM）

默认关闭。启用后使用 HTTP watch/polling 方式收发消息：

1. 可选：让 nanobot 自动接入 Mochat
- 你可以先在 agent 模式里发这段提示词（把邮箱替换成你的）：

```text
Read https://raw.githubusercontent.com/HKUDS/MoChat/refs/heads/main/skills/nanobot/skill.md and register on MoChat. My Email account is xxx@xxx Bind me as your owner and DM me on MoChat.
```

- nanobot 会尝试自动注册并写入 `~/.nanobot/config.json`。

2. 手动配置（推荐你确认一次配置）
- 在 `~/.nanobot/config.json` 配置 `channels.mochat`：
- `clawToken`：必填，作为 `X-Claw-Token` 访问 Mochat API
- `sessions` / `panels`：可填具体 ID，或 `["*"]` 自动发现
- `groups` + `mention.requireInGroups`：控制群聊是否必须 @ 才触发

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

3. 启动网关：

```bash
cargo run -- gateway
```

4. 发送消息测试
- 私聊会话：使用 `session_xxx` 目标
- 群/面板会话：使用 panel/group 目标

## QQ 通道（当前仅支持单聊）

默认构建不启用 QQ；需通过 `qq-botrs` 特性开启。

1. 注册并创建机器人
- 访问 [QQ 开放平台](https://q.qq.com) 注册开发者并创建机器人应用
- 在开发设置中获取 `AppID` 和 `AppSecret`

2. 完成沙箱测试配置
- 在机器人控制台进入沙箱配置
- 将你的 QQ 号加入消息测试成员
- 使用手机 QQ 扫码后，进入机器人会话测试收发

3. 配置 `~/.nanobot/config.json`
- 使用上面的 `qq` 配置片段，填入 `appId`、`secret`
- `allowFrom` 为空表示不限制；若需限制，可填入允许的用户 openid（可从运行日志中获取）

4. 运行网关

```bash
cargo run --features qq-botrs -- gateway
```

启动后，向机器人发送 QQ 单聊消息即可收到回复。

## Slack 通道

使用 Socket Mode，无需公网回调 URL。

1. 创建 Slack App
- 打开 [Slack API](https://api.slack.com/apps) -> Create New App -> From scratch
- 选择工作区并创建应用

2. 配置应用能力
- Socket Mode：开启，并创建 App-Level Token（`connections:write`，形如 `xapp-...`）
- OAuth & Permissions：添加 bot scopes：`chat:write`、`reactions:write`、`app_mentions:read`
- Event Subscriptions：开启并订阅 `message.im`、`message.channels`、`app_mention`
- App Home：开启 Messages Tab，并允许从 Messages Tab 发消息
- Install App：安装到工作区，获取 Bot Token（`xoxb-...`）

3. 配置 `~/.nanobot/config.json`

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

4. 启动网关

```bash
cargo run -- gateway
```

你可以在私聊中直接消息机器人，或在频道里 @ 机器人触发回复。

## WhatsApp 登录

`channels login` 会自动：

- 准备 `~/.nanobot/bridge`
- 执行 `npm install`
- 执行 `npm run build`
- 启动 bridge 并在终端展示二维码登录

## 开发

```bash
cargo fmt
cargo test
cargo check --features feishu-websocket
cargo check --features dingtalk-stream
cargo check --features qq-botrs
```

## License

MIT
