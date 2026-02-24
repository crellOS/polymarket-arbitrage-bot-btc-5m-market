//! Polymarket RTDS (Real-Time Data Socket) — Chainlink crypto/USD prices (btc, eth, sol, xrp).
//! Per docs: https://docs.polymarket.com/developers/RTDS/RTDS-crypto-prices
//! Topic: crypto_prices_chainlink, filter per symbol (e.g. btc/usd, eth/usd).
//! Price-to-beat: use the message whose feed_ts is at (or within 2s of) the period start.

use crate::discovery::period_start_et_unix_for_timestamp;
use anyhow::{Context, Result};
use futures_util::{SinkExt, StreamExt};
use log::{info, warn};
use serde::Deserialize;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::time::{interval, Duration};
use tokio_tungstenite::{connect_async, tungstenite::Message};

const PING_INTERVAL_SECS: u64 = 5;
/// Only use a price as price-to-beat when feed_ts is in [period_start, period_start + 2).
const FEED_TS_CAPTURE_WINDOW_SECS: i64 = 2;

#[derive(Debug, Deserialize)]
struct ChainlinkPayload {
    symbol: String,
    #[serde(deserialize_with = "deser_ts")]
    timestamp: i64,
    #[serde(deserialize_with = "deser_f64")]
    value: f64,
}

fn deser_ts<'de, D>(d: D) -> Result<i64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error;
    let v = serde_json::Value::deserialize(d)?;
    match v {
        serde_json::Value::Number(n) => n.as_i64().ok_or_else(|| D::Error::custom("bad ts")),
        serde_json::Value::String(s) => s.parse::<i64>().map_err(D::Error::custom),
        _ => Err(D::Error::custom("timestamp must be number or string")),
    }
}

fn deser_f64<'de, D>(d: D) -> Result<f64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error;
    let v = serde_json::Value::deserialize(d)?;
    match v {
        serde_json::Value::Number(n) => n.as_f64().ok_or_else(|| D::Error::custom("bad value")),
        serde_json::Value::String(s) => s.parse::<f64>().map_err(D::Error::custom),
        _ => Err(D::Error::custom("value must be number or string")),
    }
}

#[derive(Debug, Deserialize)]
struct ChainlinkMessage {
    topic: Option<String>,
    #[serde(rename = "type")]
    msg_type: Option<String>,
    payload: Option<ChainlinkPayload>,
}

/// Map symbol (e.g. "btc") -> period_start -> price-to-beat.
pub type PriceCacheMulti = Arc<RwLock<std::collections::HashMap<String, std::collections::HashMap<i64, f64>>>>;

/// RTDS symbol to feed symbol (e.g. btc -> btc/usd).
fn rtds_feed_symbol(symbol: &str) -> String {
    format!("{}/usd", symbol.to_lowercase())
}

/// Normalize payload symbol "btc/usd" -> "btc". Returns None if not a known format.
fn payload_symbol_to_key(s: &str) -> Option<String> {
    let s = s.trim().to_lowercase();
    if let Some(slash) = s.find('/') {
        Some(s[..slash].to_string())
    } else {
        Some(s)
    }
}

/// Connect to Polymarket RTDS, subscribe to crypto_prices_chainlink for given symbols.
/// When feed_ts is in [period_start, period_start+2), set price-to-beat for that (symbol, period).
pub async fn run_rtds_chainlink_multi(
    ws_url: &str,
    symbols: &[String],
    price_cache_15: PriceCacheMulti,
    price_cache_5: PriceCacheMulti,
) -> Result<()> {
    let url = ws_url.trim_end_matches('/');
    let symbol_set: std::collections::HashSet<String> = symbols.iter().map(|s| s.to_lowercase()).collect();
    info!(
        "RTDS connecting: {} (topic: crypto_prices_chainlink, symbols: {:?})",
        url, symbols
    );

    let (mut ws_stream, _) = connect_async(url).await.context("RTDS connect failed")?;
    // Polymarket RTDS: subscribe with empty filters to receive ALL Chainlink symbols (btc/usd, eth/usd, sol/usd, xrp/usd).
    // Multiple subscription entries for the same topic may not all be applied; one subscription with filters="" streams everything.
    let sub = serde_json::json!({
        "action": "subscribe",
        "subscriptions": [{
            "topic": "crypto_prices_chainlink",
            "type": "*",
            "filters": ""
        }]
    });
    ws_stream
        .send(Message::Text(sub.to_string()))
        .await
        .context("RTDS send subscribe failed")?;
    info!("RTDS subscribed to crypto_prices_chainlink (all symbols); filtering for {:?}", symbols);

    let mut ping = interval(Duration::from_secs(PING_INTERVAL_SECS));
    ping.tick().await;

    loop {
        tokio::select! {
            Some(msg) = ws_stream.next() => {
                let msg = msg.context("RTDS stream error")?;
                match msg {
                    Message::Text(text) => {
                        if let Ok(m) = serde_json::from_str::<ChainlinkMessage>(&text) {
                            if m.topic.as_deref() == Some("crypto_prices_chainlink") {
                                if let Some(p) = m.payload {
                                    let key = match payload_symbol_to_key(&p.symbol) {
                                        Some(k) if symbol_set.contains(&k) => k,
                                        _ => continue,
                                    };
                                    // API docs: timestamp in Unix milliseconds; accept seconds if value is small.
                                    let ts_sec = if p.timestamp > 1_000_000_000_000 {
                                        p.timestamp / 1000
                                    } else {
                                        p.timestamp
                                    };
                                    let period_15 = period_start_et_unix_for_timestamp(ts_sec, 15);
                                    let period_5 = period_start_et_unix_for_timestamp(ts_sec, 5);
                                    let in_capture_15 = ts_sec >= period_15 && ts_sec < period_15 + FEED_TS_CAPTURE_WINDOW_SECS;
                                    let in_capture_5 = ts_sec >= period_5 && ts_sec < period_5 + FEED_TS_CAPTURE_WINDOW_SECS;
                                    if in_capture_15 {
                                        let mut cache = price_cache_15.write().await;
                                        let per_symbol = cache.entry(key.clone()).or_default();
                                        if !per_symbol.contains_key(&period_15) {
                                            per_symbol.insert(period_15, p.value);
                                            info!("RTDS Chainlink price-to-beat 15m {}: period {} -> {:.2} USD (feed_ts={})", key, period_15, p.value, ts_sec);
                                        }
                                    }
                                    if in_capture_5 {
                                        let mut cache = price_cache_5.write().await;
                                        let per_symbol = cache.entry(key.clone()).or_default();
                                        if !per_symbol.contains_key(&period_5) {
                                            per_symbol.insert(period_5, p.value);
                                            info!("RTDS Chainlink price-to-beat 5m {}: period {} -> {:.2} USD (feed_ts={})", key, period_5, p.value, ts_sec);
                                        }
                                    }
                                }
                            }
                        }
                    }
                    Message::Ping(data) => {
                        let _ = ws_stream.send(Message::Pong(data)).await;
                    }
                    Message::Close(_) => break,
                    _ => {}
                }
            }
            _ = ping.tick() => {
                if ws_stream.send(Message::Ping(vec![])).await.is_err() {
                    break;
                }
            }
        }
    }
    warn!("RTDS connection closed");
    Ok(())
}
