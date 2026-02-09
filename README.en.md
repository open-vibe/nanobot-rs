# nanobot-rs

中文文档: [README.md](README.md)

## Ultra-Lightweight Personal AI Assistant, Rust Edition

`nanobot-rs` is the Rust version of [`HKUDS/nanobot`](https://github.com/HKUDS/nanobot), keeping the same ultra-lightweight agent philosophy and tool-driven workflow.

- Rust rewrite for stronger concurrency stability and cleaner deployment ergonomics
- Already integrated by [`open-vibe/open-vibe`](https://github.com/open-vibe/open-vibe) as its Rust implementation of `nanobot`
- Evolving as one of the core runtimes for `open-vibe`

## Open Vibe Integration

- Current Open Vibe integration focus: DingTalk stream bridge + relay workflow into Open Vibe threads

## Features

- Agent loop: LLM calls, tool execution, session context, and error handling
- Config system: `~/.nanobot/config.json` with provider auto-matching
- Session and memory: JSONL session persistence + `memory/MEMORY.md`
- Tooling:
  - `read_file` / `write_file` / `edit_file` / `list_dir`
  - `exec`
  - `web_search` / `web_fetch`
  - `message` / `spawn` / `cron`
- Scheduling and heartbeat:
  - `CronService` (add/list/remove/enable/run + persistence)
  - `HeartbeatService`
- Multi-channel support:
  - Telegram (long polling, media download, voice transcription)
  - Discord (Gateway + REST, with typing indicator)
  - WhatsApp (Node bridge)
  - Feishu (REST send; optional WebSocket receive feature)
  - DingTalk (optional Stream receive feature)
- Built-in skills synced from the original project (`skills/*`)

## Requirements

- Rust stable (recommended 1.85+)
- Optional:
  - Node.js 18+ (for WhatsApp bridge login)
  - Brave Search API key (`web_search`)
  - Groq API key (audio transcription)

## Quick Start

### 1. Initialize

```bash
cargo run -- onboard
```

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

### 3. Chat directly

```bash
cargo run -- agent -m "Hello"
```

### 4. Start gateway

```bash
cargo run -- gateway
```

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
```

## License

MIT
