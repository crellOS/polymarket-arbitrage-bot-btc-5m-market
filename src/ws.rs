//! CLOB Market WebSocket: subscribe to asset_ids and stream best bid/ask updates.

use anyhow::{Context, Result};
use futures_util::{SinkExt, StreamExt};
use log::{debug, error, info};
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio_tungstenite::{connect_async, tungstenite::Message};

const WS_MARKET_PATH: &str = "ws/market";

#[derive(Debug, Deserialize)]
struct WsBookLevel {
    price: String,
    size: String,
}

#[derive(Debug, Deserialize)]
struct WsBookMessage {
    #[serde(rename = "event_type")]
    event_type: Option<String>,
    #[serde(rename = "asset_id")]
    asset_id: String,
    #[serde(default, alias = "bids")]
    buys: Vec<WsBookLevel>,
    #[serde(default, alias = "asks")]
    sells: Vec<WsBookLevel>,
}

#[derive(Debug, Deserialize)]
struct WsPriceChangeItem {
    #[serde(rename = "asset_id")]
    asset_id: String,
    #[serde(rename = "best_bid")]
    best_bid: Option<String>,
    #[serde(rename = "best_ask")]
    best_ask: Option<String>,
}

#[derive(Debug, Deserialize)]
struct WsPriceChangeMessage {
    #[serde(rename = "event_type")]
    event_type: Option<String>,
    #[serde(rename = "price_changes")]
    price_changes: Vec<WsPriceChangeItem>,
}

/// Best bid/ask for one token. None if unknown.
#[derive(Debug, Clone, Default)]
pub struct BestPrices {
    pub bid: Option<f64>,
    pub ask: Option<f64>,
}

/// Shared state: token_id -> best bid/ask. Updated by WS task.
pub type PricesSnapshot = Arc<RwLock<HashMap<String, BestPrices>>>;

fn parse_f64(s: &str) -> Option<f64> {
    s.trim().parse().ok()
}

/// Run WebSocket market channel: connect, subscribe to asset_ids, and update shared prices.
/// Exits when the stream closes or on unrecoverable error.
pub async fn run_market_ws(
    ws_base_url: &str,
    asset_ids: Vec<String>,
    prices: PricesSnapshot,
) -> Result<()> {
    let url = format!(
        "{}/{}",
        ws_base_url.trim_end_matches('/'),
        WS_MARKET_PATH
    );
    info!("Connecting to market WebSocket: {}", url);

    let (ws_stream, _) = connect_async(&url)
        .await
        .context("Failed to connect to CLOB WebSocket")?;

    let (mut write, mut read) = ws_stream.split();

    let sub = serde_json::json!({
        "assets_ids": asset_ids,
        "type": "market"
    });
    let sub_msg = Message::Text(serde_json::to_string(&sub)?);
    write.send(sub_msg).await.context("Send subscribe failed")?;
    info!("Subscribed to {} assets", asset_ids.len());

    while let Some(msg) = read.next().await {
        match msg {
            Ok(Message::Text(text)) => {
                if text == "PONG" || text == "pong" {
                    continue;
                }
                if let Err(e) = process_message(&text, &prices).await {
                    debug!("WS parse error: {} for message: {}", e, &text[..text.len().min(200)]);
                }
            }
            Ok(Message::Ping(data)) => {
                let _ = write.send(Message::Pong(data)).await;
            }
            Ok(Message::Close(_)) => {
                info!("WebSocket closed by server");
                break;
            }
            Err(e) => {
                error!("WebSocket error: {}", e);
                break;
            }
            _ => {}
        }
    }

    Ok(())
}

async fn process_message(text: &str, prices: &PricesSnapshot) -> Result<()> {
    let v: serde_json::Value = serde_json::from_str(text).context("Parse JSON")?;
    let event_type = v.get("event_type").and_then(|t| t.as_str());

    if event_type == Some("book") {
        let book: WsBookMessage = serde_json::from_value(v).context("Parse book")?;
        let bid = book.buys.first().and_then(|b| parse_f64(&b.price));
        let ask = book.sells.first().and_then(|a| parse_f64(&a.price));
        if bid.is_some() || ask.is_some() {
            let mut w = prices.write().await;
            let entry = w.entry(book.asset_id).or_default();
            if let Some(b) = bid {
                entry.bid = Some(b);
            }
            if let Some(a) = ask {
                entry.ask = Some(a);
            }
        }
        return Ok(());
    }

    if event_type == Some("price_change") {
        let msg: WsPriceChangeMessage = serde_json::from_value(v).context("Parse price_change")?;
        let mut w = prices.write().await;
        for pc in msg.price_changes {
            let bid = pc.best_bid.and_then(|s| parse_f64(&s));
            let ask = pc.best_ask.and_then(|s| parse_f64(&s));
            if bid.is_some() || ask.is_some() {
                let entry = w.entry(pc.asset_id).or_default();
                if let Some(b) = bid {
                    entry.bid = Some(b);
                }
                if let Some(a) = ask {
                    entry.ask = Some(a);
                }
            }
        }
        return Ok(());
    }

    Ok(())
}
