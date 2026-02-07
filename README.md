# nanobot-rs

English: [README.en.md](README.en.md)

`nanobot-rs` 是对原 `nanobot` 的 Rust 完整移植版本，目标是保留原有工作流和工具能力，同时提供更稳定的并发与部署体验。

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
    }
  },
  "agents": {
    "defaults": {
      "model": "gpt-4o-mini"
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

## Feishu WebSocket 接收

默认构建下可正常发送消息。要启用 Feishu WebSocket 接收：

```bash
cargo run --features feishu-websocket -- gateway
```

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
```

## License

MIT
