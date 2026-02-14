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

/// 15m vs 5m arbitrage: place both sides when sum of asks < threshold; verify fills after N secs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrategyConfig {
    /// Max sum of (15m one side ask + 5m opposite side ask) to trigger arb (e.g. 0.99).
    #[serde(default = "default_sum_threshold")]
    pub sum_threshold: f64,
    /// Size in shares per side for each arb leg.
    pub shares: f64,
    /// Seconds to wait after placing orders before checking if both filled.
    #[serde(default = "default_verify_fill_secs")]
    pub verify_fill_secs: u64,
    #[serde(default)]
    pub simulation_mode: bool,
    /// Seconds after market start before price-to-beat API returns data (15m ~2min, 5m ~30s; use 30 for both).
    #[serde(default = "default_price_to_beat_delay_secs")]
    pub price_to_beat_delay_secs: u64,
    /// Interval (seconds) between price-to-beat API polls once delay has passed.
    #[serde(default = "default_price_to_beat_poll_interval_secs")]
    pub price_to_beat_poll_interval_secs: u64,
}

fn default_sum_threshold() -> f64 {
    0.99
}
fn default_verify_fill_secs() -> u64 {
    10
}
fn default_price_to_beat_delay_secs() -> u64 {
    30
}
fn default_price_to_beat_poll_interval_secs() -> u64 {
    10
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
    /// WebSocket base URL for market channel (e.g. wss://ws-subscriptions-clob.polymarket.com).
    #[serde(default = "default_ws_url")]
    pub ws_url: String,
}

fn default_ws_url() -> String {
    "wss://ws-subscriptions-clob.polymarket.com".to_string()
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
                ws_url: default_ws_url(),
            },
            strategy: StrategyConfig {
                sum_threshold: 0.99,
                shares: 5.0,
                verify_fill_secs: 10,
                simulation_mode: false,
                price_to_beat_delay_secs: 30,
                price_to_beat_poll_interval_secs: 10,
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
