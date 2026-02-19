//! 5m pre-order trading bot: BTC, ETH, SOL, XRP. Monitor live prices, revert view, and optionally place pre-orders.
//! Price-to-beat from Polymarket RTDS Chainlink (crypto_prices_chainlink) per symbol.

use crate::api::PolymarketApi;
use crate::chainlink::run_chainlink_multi_poller;
use crate::config::Config;
use crate::discovery::{current_5m_period_start, MarketDiscovery};
use crate::models::OrderRequest;
use crate::rtds::PriceCacheMulti;
use crate::ws::{run_market_ws, PricesSnapshot};
use anyhow::Result;
use chrono::Utc;
use log::{error, info, warn};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::time::{sleep, Duration};

const MARKET_5M_DURATION_SECS: i64 = 5 * 60; // 300
const LIVE_PRICE_POLL_MS: u64 = 100;

/// Round to cents for change detection (avoid log spam from tiny float changes).
fn price_key(p: Option<f64>) -> u32 {
    p.map(|v| (v * 100.0).round().max(0.0) as u32).unwrap_or(0)
}

pub struct ArbStrategy {
    api: Arc<PolymarketApi>,
    config: Config,
    discovery: MarketDiscovery,
    /// symbol -> period_start -> price-to-beat (from RTDS Chainlink).
    price_cache_5: PriceCacheMulti,
}

impl ArbStrategy {
    pub fn new(api: Arc<PolymarketApi>, config: Config) -> Self {
        Self {
            discovery: MarketDiscovery::new(api.clone()),
            api,
            config,
            price_cache_5: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Wait until we have the current 5m market and its price-to-beat for the given symbol.
    /// Returns (m5_cid, m5_up_token, m5_down_token, period_5, price_to_beat).
    async fn wait_for_5m_market_and_price(&self, symbol: &str) -> Result<(String, String, String, i64, f64)> {
        const WAIT_POLL_SECS: u64 = 10;
        loop {
            let period_5 = current_5m_period_start();
            let (m5_cid, _) = match self.discovery.get_5m_market(symbol, period_5).await? {
                Some((cid, _)) => (cid, ()),
                None => {
                    warn!("5m {} market not found for period {}. Sleeping 10s.", symbol, period_5);
                    sleep(Duration::from_secs(10)).await;
                    continue;
                }
            };
            let price_to_beat = {
                let cache = self.price_cache_5.read().await;
                cache.get(symbol).and_then(|per_period| per_period.get(&period_5).copied())
            };
            let price_to_beat = match price_to_beat {
                Some(p) => p,
                None => {
                    info!(
                        "5m {} period {}: waiting for price-to-beat from RTDS Chainlink.",
                        symbol, period_5
                    );
                    sleep(Duration::from_secs(WAIT_POLL_SECS)).await;
                    continue;
                }
            };
            let (m5_up, m5_down) = self.discovery.get_market_tokens(&m5_cid).await?;
            info!(
                "5m {} market active: period_start={}, price-to-beat (RTDS Chainlink)={:.2} USD",
                symbol, period_5, price_to_beat
            );
            return Ok((m5_cid, m5_up, m5_down, period_5, price_to_beat));
        }
    }

    /// Resolve pre-order side: "up" | "down" | "favor" (market favors = lower ask).
    fn pre_order_side(&self, market_favors_up: bool) -> &'static str {
        let side = self.config.strategy.pre_order_side.to_lowercase();
        match side.as_str() {
            "up" => "up",
            "down" => "down",
            "favor" => if market_favors_up { "up" } else { "down" },
            _ => if market_favors_up { "up" } else { "down" },
        }
    }

    /// Run one 5m round: subscribe to 5m prices, log revert view, optionally place one pre-order per period.
    async fn run_5m_round(
        &self,
        symbol: &str,
        _m5_cid: &str,
        m5_up: &str,
        m5_down: &str,
        period_5: i64,
        price_to_beat: f64,
    ) -> Result<()> {
        let prices: PricesSnapshot = Arc::new(RwLock::new(HashMap::new()));
        let asset_ids = vec![m5_up.to_string(), m5_down.to_string()];
        let ws_url = self.config.polymarket.ws_url.clone();
        let prices_clone = Arc::clone(&prices);
        let symbol_ws = symbol.to_string();
        let ws_handle = tokio::spawn(async move {
            if let Err(e) = run_market_ws(&ws_url, asset_ids, prices_clone).await {
                warn!("5m {} round WebSocket exited: {}", symbol_ws, e);
            }
        });

        let mut last_key: Option<(u32, u32, u32, u32)> = None;
        let mut pre_order_placed = false;
        let pre_order_enabled = self.config.strategy.pre_order_enabled && !self.config.strategy.simulation_mode;

        loop {
            let now = Utc::now().timestamp();
            if now >= period_5 + MARKET_5M_DURATION_SECS {
                info!("5m {} market ended (period {}). Exiting round.", symbol, period_5);
                break;
            }
            let snap = prices.read().await;
            let ask_up = snap.get(m5_up).and_then(|p| p.ask);
            let ask_down = snap.get(m5_down).and_then(|p| p.ask);
            let bid_up = snap.get(m5_up).and_then(|p| p.bid);
            let bid_down = snap.get(m5_down).and_then(|p| p.bid);
            let key = (price_key(bid_up), price_key(ask_up), price_key(bid_down), price_key(ask_down));
            if (ask_up.is_some() || ask_down.is_some()) && last_key != Some(key) {
                last_key = Some(key);
                let (au, ad) = (ask_up.unwrap_or(0.0), ask_down.unwrap_or(0.0));
                let market_favors_up = au < ad;
                info!(
                    "  {} Revert view: market favors {} | could revert to {} if {} {} ${:.2}",
                    symbol.to_uppercase(),
                    if market_favors_up { "Up" } else { "Down" },
                    if market_favors_up { "Down" } else { "Up" },
                    symbol.to_uppercase(),
                    if market_favors_up { "drops below" } else { "rises above" },
                    price_to_beat
                );

                // Pre-order: place one limit BUY per period when we have a valid best ask.
                if pre_order_enabled && !pre_order_placed {
                    let side = self.pre_order_side(market_favors_up);
                    let (token_id, best_ask) = if side == "up" {
                        (m5_up, ask_up)
                    } else {
                        (m5_down, ask_down)
                    };
                    if let Some(ask) = best_ask {
                        let ticks = self.config.strategy.pre_order_improve_ticks as f64 * 0.01;
                        let price = (ask - ticks).max(0.01);
                        let size = self.config.strategy.pre_order_size.clone();
                        let order = OrderRequest {
                            token_id: token_id.to_string(),
                            side: "BUY".to_string(),
                            size,
                            price: format!("{:.4}", price),
                            order_type: "GTC".to_string(),
                        };
                        drop(snap);
                        match self.api.place_order(&order).await {
                            Ok(_) => {
                                info!("  {} Pre-order placed: BUY {} @ {:.4} ({})", symbol.to_uppercase(), order.size, price, side);
                                pre_order_placed = true;
                            }
                            Err(e) => {
                                warn!("  {} Pre-order failed: {}", symbol.to_uppercase(), e);
                            }
                        }
                        continue;
                    }
                }
            }
            drop(snap);
            sleep(Duration::from_millis(LIVE_PRICE_POLL_MS)).await;
        }
        ws_handle.abort();
        Ok(())
    }

    /// Poll until 5m market is closed and resolved; returns winning outcome (e.g. "Up" or "Down").
    async fn poll_until_5m_resolved(&self, symbol: &str, _m5_cid: &str) -> Option<String> {
        const INITIAL_DELAY_SECS: u64 = 60;
        const POLL_INTERVAL_SECS: u64 = 45;
        const MAX_WAIT_SECS: u64 = 600;
        info!(
            "5m {} market end: waiting {}s then polling every {}s for resolution (max {}s).",
            symbol, INITIAL_DELAY_SECS, POLL_INTERVAL_SECS, MAX_WAIT_SECS
        );
        sleep(Duration::from_secs(INITIAL_DELAY_SECS)).await;
        let started = std::time::Instant::now();
        loop {
            if started.elapsed().as_secs() >= MAX_WAIT_SECS {
                warn!("5m {} resolution timeout within {}s.", symbol, MAX_WAIT_SECS);
                return None;
            }
            let m = match self.api.get_market(_m5_cid).await {
                Ok(m) => m,
                Err(e) => {
                    warn!("Poll 5m {} market failed: {}", symbol, e);
                    sleep(Duration::from_secs(POLL_INTERVAL_SECS)).await;
                    continue;
                }
            };
            let winner = m
                .tokens
                .iter()
                .find(|t| t.winner)
                .map(|t| {
                    if t.outcome.to_uppercase().contains("UP") || t.outcome == "1" {
                        "Up".to_string()
                    } else {
                        "Down".to_string()
                    }
                });
            if m.closed && winner.is_some() {
                info!("5m {} market resolved: winner {}", symbol, winner.as_deref().unwrap_or("?"));
                return winner;
            }
            sleep(Duration::from_secs(POLL_INTERVAL_SECS)).await;
        }
    }

    /// Per-symbol loop: wait for market + price, run round, poll resolution, repeat.
    async fn run_symbol_loop(
        api: Arc<PolymarketApi>,
        config: Config,
        price_cache_5: PriceCacheMulti,
        symbol: String,
    ) -> Result<()> {
        let discovery = MarketDiscovery::new(api.clone());
        let strategy = Self {
            api,
            config,
            discovery,
            price_cache_5,
        };
        loop {
            let (m5_cid, m5_up, m5_down, period_5, price_to_beat) =
                strategy.wait_for_5m_market_and_price(&symbol).await?;
            if let Err(e) = strategy
                .run_5m_round(&symbol, &m5_cid, &m5_up, &m5_down, period_5, price_to_beat)
                .await
            {
                error!("5m {} round error: {}", symbol, e);
            }
            let _ = strategy.poll_until_5m_resolved(&symbol, &m5_cid).await;
            sleep(Duration::from_secs(5)).await;
        }
    }

    pub async fn run(&self) -> Result<()> {
        let symbols = &self.config.strategy.symbols;
        info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
        info!("5m pre-order trading bot (symbols: {:?})", symbols);
        info!("   Price-to-beat: Polymarket RTDS Chainlink (crypto_prices_chainlink)");
        info!("   Pre-order: enabled={}, side={}, size={}", 
            self.config.strategy.pre_order_enabled, 
            self.config.strategy.pre_order_side, 
            self.config.strategy.pre_order_size);
        info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

        let rtds_url = self.config.polymarket.rtds_ws_url.clone();
        let cache_15: PriceCacheMulti = Arc::new(RwLock::new(HashMap::new()));
        let cache_5 = Arc::clone(&self.price_cache_5);
        let symbols_rtds = symbols.clone();
        if let Err(e) = run_chainlink_multi_poller(rtds_url, symbols_rtds, cache_15, cache_5).await {
            warn!("RTDS Chainlink multi poller start: {}", e);
        }
        sleep(Duration::from_secs(2)).await;

        let mut handles = Vec::new();
        for symbol in symbols.clone() {
            let api = Arc::clone(&self.api);
            let config = self.config.clone();
            let price_cache_5 = Arc::clone(&self.price_cache_5);
            handles.push(tokio::spawn(async move {
                if let Err(e) = Self::run_symbol_loop(api, config, price_cache_5, symbol.clone()).await {
                    error!("Symbol loop {} failed: {}", symbol, e);
                }
            }));
        }
        futures_util::future::try_join_all(handles).await?;
        Ok(())
    }
}
