# Polymarket Wallet Tracker

Real-time Telegram alerts for Polymarket wallet activity. Monitors proxy wallets via the Polymarket Data API and sends notifications when trades occur.

## Features

- Poll Polymarket Data API for trades by watched proxy wallets
- Telegram bot commands to manage watchlist (`/add`, `/remove`, `/list`, `/status`)
- Admin access control for wallet management
- In-memory dedup — only alerts trades that occur while the app is running
- Health/readiness HTTP endpoints (`/healthz`, `/readyz`)
- Bot score analysis (`/botscore`) — heuristic scoring pipeline to detect automated wallets
- Structured JSON logging via `tracing`
- Graceful shutdown (SIGINT/SIGTERM)

## Prerequisites

- Rust 1.75+
- Supabase project (or any PostgreSQL database)
- Telegram bot token (from [@BotFather](https://t.me/BotFather))

## Environment Variables

| Variable | Required | Default | Description |
|---|---|---|---|
| `TELEGRAM_BOT_TOKEN` | Yes | - | Telegram bot API token |
| `ADMIN_USER_IDS` | Yes | - | Comma-separated Telegram user IDs for admin access |
| `DATABASE_URL` | Yes | - | PostgreSQL connection string |
| `POLYMARKET_API_BASE_URL` | No | `https://data-api.polymarket.com` | Polymarket Data API base URL |
| `POLL_INTERVAL_SECS` | No | `15` | Polling interval in seconds |
| `MAX_CONCURRENCY` | No | `5` | Max concurrent wallet polls |
| `HTTP_PORT` | No | `8080` | Health server port |
| `RUST_LOG` | No | `info` | Log level filter |

## Setup

1. **Create a Supabase project** (or use any Postgres instance). Copy the direct connection string from Settings > Database.

2. **Create a Telegram bot** via [@BotFather](https://t.me/BotFather) and note the token.

3. **Get your Telegram user ID** (send `/start` to [@userinfobot](https://t.me/userinfobot)).

4. **Configure environment**:
   ```bash
   cp .env.example .env
   # Edit .env with your values
   ```

5. **Run**:
   ```bash
   cargo run
   ```

   Migrations run automatically on startup.

## Telegram Commands

| Command | Admin Only | Description |
|---|---|---|
| `/help` | No | Show available commands |
| `/add <0xAddress> [alias]` | Yes | Add a proxy wallet to watch |
| `/remove <0xAddress>` | Yes | Remove a wallet from watchlist |
| `/list` | No | List all watched wallets |
| `/status` | No | Show uptime, wallet count, last poll time |
| `/subscribe` | Yes | Subscribe this chat to receive trade alerts |
| `/botscore <0xAddress>` | Yes | Analyze if a wallet is likely a bot (score 0–100) |

## Running Tests

```bash
cargo test
```

## How It Works

### Setup
1. Admin sends `/subscribe` to the bot to register their chat for trade alerts
2. Admin sends `/add 0x<proxyWallet> [alias]` to add wallets to watch

### Runtime
1. On startup, records the current timestamp as `startup_timestamp`
2. Loads watched wallets from the database
3. Every `POLL_INTERVAL_SECS`, polls the Polymarket Data API for trades for each watched wallet
4. Filters to only trades with `timestamp > startup_timestamp` and not yet seen (in-memory `HashSet`)
5. Sends Telegram alerts to all subscribed chats
6. On restart, starts fresh — no duplicate alerts, no recovery of missed trades

### Bot Score Analysis

The `/botscore` command fetches the last 7 days of trades for a wallet and runs a pipeline of heuristic rules:

| Points | Signal | Threshold |
|---|---|---|
| +30 | Trade frequency | > 200 trades/day |
| +25 | Micro-trade ratio | > 70% of trades under $5 |
| +20 | Maker fill ratio | > 70% maker fills |
| +15 | 24h uniformity | > 0.80 (no sleep gaps) |
| +10 | Market diversity | > 50 unique markets in 7d |

Score interpretation:
- **0–25**: HumanLikely
- **26–60**: Hybrid (power user / semi-bot)
- **61–100**: BotLikely (market maker / arb / copy bot)

The pipeline is extensible — new rules can be added by implementing the `ScoringRule` trait.
