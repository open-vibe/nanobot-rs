# nanobot-rs

English: [README.en.md](README.en.md)

## Ultra-Lightweight Personal AI Assistant, Rust Edition

`nanobot-rs` 是 [`HKUDS/nanobot`](https://github.com/HKUDS/nanobot) 的 Rust 版本，延续 ultra-lightweight 的 Agent 设计与工具工作流。

- Rust 重写：更稳定的并发执行、更清晰的部署和工程化体验
- 已由 [`open-vibe/open-vibe`](https://github.com/open-vibe/open-vibe) 接入，作为其 `nanobot` Rust 实现
- 作为 `open-vibe` 后续核心运行时之一持续演进

## Open Vibe 集成

- Open Vibe 当前集成重点：DingTalk stream bridge + relay workflow 到 Open Vibe threads

## 特性

- Agent 主循环：LLM 调用、工具调用、会话上下文、错误恢复
- 配置系统：`~/.nanobot/config.json`，支持 provider 自动匹配
- 会话与记忆：JSONL 会话持久化 + `memory/MEMORY.md`
- 工具系统：
  - `read_file` / `write_file` / `edit_file` / `list_dir`
  - `exec`
  - `web_search` / `web_fetch`
  - `message` / `spawn` / `cron`
- 定时任务与心跳：
  - `CronService`（add/list/remove/enable/run + 持久化）
  - `HeartbeatService`
- 多渠道接入：
  - Telegram（long polling，支持媒体下载与语音转写）
  - Discord（Gateway + REST，支持 typing 指示）
  - WhatsApp（Node bridge）
  - Feishu（REST 发送；WebSocket 接收可选特性）
  - DingTalk（Stream 接收可选特性）
  - Email（IMAP 收信 + SMTP 发信，需显式 consent）
  - Slack（Socket Mode）
  - QQ（可选特性，`qq-botrs`）
- 内置 skills：同步原项目 `skills/*`

## 环境要求

- Rust stable（建议 1.85+）
- 可选：
  - Node.js 18+（WhatsApp bridge 登录）
  - Brave Search API Key（`web_search`）
  - Groq API Key（语音转写）

## 快速开始

### 1. 初始化

```bash
cargo run -- onboard
```

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

### 3. 直接对话

```bash
cargo run -- agent -m "Hello"
```

### 4. 启动网关

```bash
cargo run -- gateway
```

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
