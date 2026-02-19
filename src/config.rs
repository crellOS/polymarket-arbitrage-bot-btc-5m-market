use clap::Parser;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
pub struct Args {
    #[arg(short, long, default_value = "config.json")]
    pub config: PathBuf,

    #[arg(long)]
    pub redeem: bool,

    #[arg(long, requires = "redeem")]
    pub condition_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub polymarket: PolymarketConfig,
    pub strategy: StrategyConfig,
}

/// 5m pre-order trading: symbols to trade, pre-order size/side/improvement.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrategyConfig {
    /// 5m market symbols (e.g. btc, eth, sol, xrp). Slug format: {symbol}-updown-5m-{period}.
    #[serde(default = "default_symbols")]
    pub symbols: Vec<String>,
    /// Max sum of (15m one side ask + 5m opposite side ask) to trigger arb (e.g. 0.99).
    #[serde(default = "default_sum_threshold")]
    pub sum_threshold: f64,
    /// Seconds to wait after placing orders before checking if both filled.
    #[serde(default = "default_verify_fill_secs")]
    pub verify_fill_secs: u64,
    #[serde(default)]
    pub simulation_mode: bool,
    /// Enable pre-order: place one limit buy per 5m market per period.
    #[serde(default)]
    pub pre_order_enabled: bool,
    /// Pre-order size in shares (e.g. "10").
    #[serde(default = "default_pre_order_size")]
    pub pre_order_size: String,
    /// Pre-order side: "up", "down", or "favor" (side the market currently favors).
    #[serde(default = "default_pre_order_side")]
    pub pre_order_side: String,
    /// Ticks to improve vs best ask (1 tick = 0.01). 0 = use best ask.
    #[serde(default)]
    pub pre_order_improve_ticks: u32,
}

fn default_symbols() -> Vec<String> {
    vec!["btc".into(), "eth".into(), "sol".into(), "xrp".into()]
}
fn default_sum_threshold() -> f64 {
    0.99
}
fn default_verify_fill_secs() -> u64 {
    10
}
fn default_pre_order_size() -> String {
    "10".to_string()
}
fn default_pre_order_side() -> String {
    "favor".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolymarketConfig {
    pub gamma_api_url: String,
    pub clob_api_url: String,
    pub api_key: Option<String>,
    pub api_secret: Option<String>,
    pub api_passphrase: Option<String>,
    pub private_key: Option<String>,
    pub proxy_wallet_address: Option<String>,
    pub signature_type: Option<u8>,
    /// Polygon RPC URL for redemption (Safe reads + sendTransaction). Defaults to polygon-rpc.com if unset.
    #[serde(default)]
    pub rpc_url: Option<String>,
    /// WebSocket base URL for market channel (e.g. wss://ws-subscriptions-clob.polymarket.com).
    #[serde(default = "default_ws_url")]
    pub ws_url: String,
    /// RTDS WebSocket URL for Chainlink BTC price (price-to-beat). Topic: crypto_prices_chainlink, symbol: btc/usd.
    #[serde(default = "default_rtds_ws_url")]
    pub rtds_ws_url: String,
}

fn default_ws_url() -> String {
    "wss://ws-subscriptions-clob.polymarket.com".to_string()
}

fn default_rtds_ws_url() -> String {
    "wss://ws-live-data.polymarket.com".to_string()
}

impl Default for Config {
    fn default() -> Self {
        Self {
            polymarket: PolymarketConfig {
                gamma_api_url: "https://gamma-api.polymarket.com".to_string(),
                clob_api_url: "https://clob.polymarket.com".to_string(),
                api_key: None,
                api_secret: None,
                api_passphrase: None,
                private_key: None,
                proxy_wallet_address: None,
                signature_type: None,
                rpc_url: None,
                ws_url: default_ws_url(),
                rtds_ws_url: default_rtds_ws_url(),
            },
            strategy: StrategyConfig {
                symbols: default_symbols(),
                sum_threshold: 0.99,
                verify_fill_secs: 10,
                simulation_mode: false,
                pre_order_enabled: false,
                pre_order_size: default_pre_order_size(),
                pre_order_side: default_pre_order_side(),
                pre_order_improve_ticks: 0,
            },
        }
    }
}

impl Config {
    pub fn load(path: &PathBuf) -> anyhow::Result<Self> {
        if path.exists() {
            let content = std::fs::read_to_string(path)?;
            Ok(serde_json::from_str(&content)?)
        } else {
            let config = Config::default();
            let content = serde_json::to_string_pretty(&config)?;
            std::fs::write(path, content)?;
            Ok(config)
        }
    }
}
