# Polymarket BTC 15m vs 5m Arbitrage Bot

A high-performance **Rust** arbitrage bot for [Polymarket](https://polymarket.com) that trades the overlap between **Bitcoin 15-minute** and **5-minute** Up/Down prediction markets. It uses real-time order book data over **WebSocket**, fetches **price-to-beat** from Polymarket’s API, and places limit orders when the combined ask across both markets is below a configurable threshold (e.g. &lt; $0.99) for a locked-in profit.

---

## Table of Contents

- [Features](#features)
- [How It Works](#how-it-works)
- [Requirements](#requirements)
- [Installation](#installation)
- [Configuration](#configuration)
- [Usage](#usage)
- [Risk Management](#risk-management)
- [Disclaimer](#disclaimer)
- [Contact](#contact)

---

## Features

- **Dual-market arbitrage** — Trades only when the last 5 minutes of the 15m BTC market overlap with the current 5m BTC market and **price-to-beat** matches for both.
- **Real-time data** — Uses Polymarket’s **WebSocket** market channel for order book updates (no polling for prices).
- **Price-to-beat** — Fetches open price from Polymarket’s crypto-price API for both 15m and 5m markets (with configurable delay and poll interval).
- **Instant execution** — Submits both legs as soon as an arb opportunity is detected (sum of asks &lt; threshold).
- **Fill verification** — After placing orders, checks in ~10 seconds whether both legs filled; if only one filled, sells the matched side and cancels the other.
- **Simulation mode** — Run without placing real orders to test logic and logs.
- **Redeem mode** — CLI option to redeem winning positions for a proxy wallet.

---

## How It Works

1. **Arb window**  
   The bot only runs when:
   - Time is in the **last 5 minutes** of the current 15m BTC market.
   - The overlapping **5m BTC market** exists and is active.
   - **Price-to-beat** is fetched for both markets (after a short delay from market start) and must match.

2. **Opportunity**  
   It subscribes to the **order book** (WebSocket) for the 15m Up/Down and 5m Up/Down tokens. When:
   - **(15m Up ask + 5m Down ask) &lt; threshold**, or  
   - **(15m Down ask + 5m Up ask) &lt; threshold**,  
   it places **limit buy** orders on both sides at the current ask (or your configured logic).

3. **Post-trade**  
   After a short delay (e.g. 10s), it verifies both orders. If only one leg filled, it **sells the filled token** and **cancels the unfilled order** to limit downside.

---

## Requirements

- **Rust** 1.70+ (`rustup` recommended)
- **Polymarket account** with API credentials and (for live trading) a funded proxy wallet
- **Polygon** network for signing (Polymarket CLOB)

---

## Installation

```bash
git clone https://github.com/crellos/polymarket-trading-bot-5m-markets.git
cd polymarket-trading-bot-btc-5m-market
cargo build --release
```

Binary: `target/release/polymarket-arbitrage-bot`

---

## Configuration

Copy or create `config.json` in the project root. Structure:

```json
{
  "polymarket": {
    "gamma_api_url": "https://gamma-api.polymarket.com",
    "clob_api_url": "https://clob.polymarket.com",
    "api_key": "YOUR_API_KEY",
    "api_secret": "YOUR_API_SECRET",
    "api_passphrase": "YOUR_PASSPHRASE",
    "private_key": "YOUR_POLYGON_PRIVATE_KEY_HEX", //consider about signature type carefully
    "proxy_wallet_address": "0x...",
    "signature_type": 2,
    "ws_url": "wss://ws-subscriptions-clob.polymarket.com"
  },
  "strategy": {
    "sum_threshold": 0.99,
    "shares": 5,
    "verify_fill_secs": 10,
    "simulation_mode": true,
    "price_to_beat_delay_secs": 30,
    "price_to_beat_poll_interval_secs": 10
  }
}
```

| Field | Description |
|-------|-------------|
| `sum_threshold` | Max sum of (15m one side ask + 5m opposite ask) to trigger arb (e.g. 0.99). |
| `shares` | Size in shares per leg. |
| `verify_fill_secs` | Seconds to wait before checking if both orders filled. |
| `simulation_mode` | If `true`, no real orders are placed. |
| `price_to_beat_delay_secs` | Seconds after market start before polling price-to-beat (e.g. 30). |
| `price_to_beat_poll_interval_secs` | Poll interval for price-to-beat (e.g. 10). |

Do **not** commit real API keys or `private_key`; use env vars or a secrets manager in production.

---

## Usage

**Run the arbitrage bot (uses `config.json` by default):**

```bash
cargo run --release
# or
./target/release/polymarket-arbitrage-bot
```

**Custom config path:**

```bash
./target/release/polymarket-arbitrage-bot -c /path/to/config.json
```

**Redeem winning positions (proxy wallet):**

```bash
./target/release/polymarket-arbitrage-bot --redeem
# optional: specific condition
./target/release/polymarket-arbitrage-bot --redeem --condition_id 0x...
```

**Logging:** set `RUST_LOG` (e.g. `RUST_LOG=info` or `RUST_LOG=debug`).

---

## Risk Management

- **One leg filled** — After the verification delay, if only one order is filled, the bot **sells that token** (market) and **cancels the other order** to avoid holding a one-sided position.
- **Neither filled** — Both orders are cancelled.
- **Simulation** — Use `simulation_mode: true` to test without sending real orders.

Trading prediction markets and crypto carries risk; only use funds you can afford to lose.

---

## Disclaimer

This bot is for **educational and research purposes**. Polymarket and crypto prediction markets are volatile. The authors are not responsible for any financial loss. Use at your own risk and comply with your jurisdiction’s laws and Polymarket’s terms of service.

---

## Contact

- **Telegram:** [crellos_0x](https://t.me/crellos_0x) (crellOS)

---

## Keywords

Polymarket arbitrage bot · Bitcoin 15 minute 5 minute market · BTC prediction market trading · Rust trading bot · Polymarket WebSocket · price to beat · crypto prediction market automation
