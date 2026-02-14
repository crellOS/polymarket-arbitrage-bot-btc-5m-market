//! 15m vs 5m BTC arbitrage: only in last 5 min of 15m when price-to-beat matches;
//! monitor via WebSocket; place both sides when sum of asks < threshold.
//! After 10s: verify both filled; if only one side matched, sell that token and cancel the other order.

use crate::api::PolymarketApi;
use crate::config::Config;
use crate::discovery::{
    current_15m_period_start, current_5m_period_start, is_last_5min_of_15m,
    MarketDiscovery, MARKET_15M_DURATION_SECS, MARKET_5M_DURATION_SECS,
};
use crate::models::{OrderRequest, OrderResponse};
use crate::ws::{run_market_ws, PricesSnapshot};
use anyhow::Result;
use chrono::{TimeZone, Utc};
use log::{error, info, warn};
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::time::{sleep, Duration};

fn format_timestamp_iso(unix_secs: i64) -> String {
    Utc.timestamp_opt(unix_secs, 0)
        .single()
        .map(|dt: chrono::DateTime<_>| dt.format("%Y-%m-%dT%H:%M:%SZ").to_string())
        .unwrap_or_else(|| "1970-01-01T00:00:00Z".to_string())
}

/// Returns (filled_a, filled_b). In simulation mode assumes both filled.
async fn check_fill_status(
    api: &PolymarketApi,
    order_id_a: Option<&str>,
    order_id_b: Option<&str>,
    sim: bool,
) -> (bool, bool) {
    if sim {
        return (true, true);
    }
    let ok_a = match order_id_a {
        Some(id) => api.get_order_status(id).await.ok(),
        None => None,
    };
    let ok_b = match order_id_b {
        Some(id) => api.get_order_status(id).await.ok(),
        None => None,
    };
    let filled_a = ok_a
        .and_then(|s| {
            let o: f64 = s.original_size.as_deref()?.parse().ok()?;
            let m: f64 = s.size_matched.as_deref()?.parse().ok()?;
            Some(o > 0.0 && m >= o - 0.001)
        })
        .unwrap_or(false);
    let filled_b = ok_b
        .and_then(|s| {
            let o: f64 = s.original_size.as_deref()?.parse().ok()?;
            let m: f64 = s.size_matched.as_deref()?.parse().ok()?;
            Some(o > 0.0 && m >= o - 0.001)
        })
        .unwrap_or(false);
    (filled_a, filled_b)
}

/// Parameters for verify-and-manage-risk: one arb leg (token A + token B and their order IDs).
struct VerifyRiskParams {
    token_a: String,
    token_b: String,
    order_id_a: Option<String>,
    order_id_b: Option<String>,
    shares: f64,
    verify_secs: u64,
    simulation: bool,
    label_a: String,
    label_b: String,
}

/// After verify_secs: confirm both filled; if only one filled, sell that token and cancel the other order.
async fn verify_and_manage_risk(api: Arc<PolymarketApi>, p: VerifyRiskParams) {
    sleep(Duration::from_secs(p.verify_secs)).await;
    let (filled_a, filled_b) = check_fill_status(
        &api,
        p.order_id_a.as_deref(),
        p.order_id_b.as_deref(),
        p.simulation,
    )
    .await;

    if filled_a && filled_b {
        info!(
            "Both orders confirmed filled ({} + {}). Ready for next opportunity.",
            p.label_a, p.label_b
        );
        return;
    }

    if filled_a && !filled_b {
        warn!(
            "Only {} matched after {}s. Risk exit: selling {} and cancelling {} order.",
            p.label_a, p.verify_secs, p.label_a, p.label_b
        );
        if p.simulation {
            info!(
                "🎮 SIMULATION: Would sell {} shares {} and cancel {} order",
                p.shares, p.label_a, p.label_b
            );
        } else {
            if let Err(e) = api.place_market_order(&p.token_a, p.shares, "SELL", None).await {
                error!("Failed to sell {}: {}", p.label_a, e);
            } else {
                info!("Sold {} shares {}", p.shares, p.label_a);
            }
            if let Some(ref id) = p.order_id_b {
                if let Err(e) = api.cancel_order(id).await {
                    error!("Failed to cancel {} order: {}", p.label_b, e);
                } else {
                    info!("Cancelled {} order", p.label_b);
                }
            }
        }
        return;
    }

    if !filled_a && filled_b {
        warn!(
            "Only {} matched after {}s. Risk exit: selling {} and cancelling {} order.",
            p.label_b, p.verify_secs, p.label_b, p.label_a
        );
        if p.simulation {
            info!(
                "🎮 SIMULATION: Would sell {} shares {} and cancel {} order",
                p.shares, p.label_b, p.label_a
            );
        } else {
            if let Err(e) = api.place_market_order(&p.token_b, p.shares, "SELL", None).await {
                error!("Failed to sell {}: {}", p.label_b, e);
            } else {
                info!("Sold {} shares {}", p.shares, p.label_b);
            }
            if let Some(ref id) = p.order_id_a {
                if let Err(e) = api.cancel_order(id).await {
                    error!("Failed to cancel {} order: {}", p.label_a, e);
                } else {
                    info!("Cancelled {} order", p.label_a);
                }
            }
        }
        return;
    }

    warn!(
        "Neither order matched after {}s. Cancelling both {} and {} orders.",
        p.verify_secs, p.label_a, p.label_b
    );
    if !p.simulation {
        if let Some(ref id) = p.order_id_a {
            let _ = api.cancel_order(id).await;
        }
        if let Some(ref id) = p.order_id_b {
            let _ = api.cancel_order(id).await;
        }
    }
}

/// Token IDs: 15m Up, 15m Down, 5m Up, 5m Down.
pub struct ArbTokens {
    pub m15_up: String,
    pub m15_down: String,
    pub m5_up: String,
    pub m5_down: String,
}

pub struct ArbStrategy {
    api: Arc<PolymarketApi>,
    config: Config,
    discovery: MarketDiscovery,
}

impl ArbStrategy {
    pub fn new(api: Arc<PolymarketApi>, config: Config) -> Self {
        let discovery = MarketDiscovery::new(api.clone());
        Self {
            api,
            config,
            discovery,
        }
    }

    /// Wait until we're in the arbitrage window: last 5 min of 15m and same price-to-beat (from API).
    /// Price-to-beat is not available at market start: we wait delay_secs (e.g. 30s) then poll every poll_interval_secs (e.g. 10s).
    /// Returns (15m condition_id, 5m condition_id, tokens, 15m period_start).
    async fn wait_for_arb_window(&self) -> Result<(String, String, ArbTokens, i64)> {
        let delay_secs = self.config.strategy.price_to_beat_delay_secs as i64;
        let poll_interval = self.config.strategy.price_to_beat_poll_interval_secs;

        loop {
            let now = Utc::now().timestamp();
            let period_15 = current_15m_period_start();
            let period_5 = current_5m_period_start();

            if !is_last_5min_of_15m(now, period_15) {
                let next_window = period_15 + 10 * 60;
                info!(
                    "Not in arb window (need last 5m of 15m). Next window in {}s. Sleeping 30s.",
                    next_window - now
                );
                sleep(Duration::from_secs(30)).await;
                continue;
            }

            let (m15, _) = match self.discovery.get_15m_market(period_15).await? {
                Some((cid, _)) => (cid, ()),
                None => {
                    warn!("15m market not found for period {}. Sleeping 10s.", period_15);
                    sleep(Duration::from_secs(10)).await;
                    continue;
                }
            };
            let (m5, _) = match self.discovery.get_5m_market(period_5).await? {
                Some((cid, _)) => (cid, ()),
                None => {
                    warn!("5m market not found for period {}. Sleeping 10s.", period_5);
                    sleep(Duration::from_secs(10)).await;
                    continue;
                }
            };

            // Price-to-beat API is not available until delay_secs after market start (15m ~2min, 5m ~30s; we use 30s for both).
            let ready_at_15 = period_15 + delay_secs;
            let ready_at_5 = period_5 + delay_secs;
            if now < ready_at_15 || now < ready_at_5 {
                let wait = (ready_at_15.max(ready_at_5) - now).max(1) as u64;
                info!(
                    "Waiting {}s for price-to-beat API (available {}s after market start).",
                    wait, delay_secs
                );
                sleep(Duration::from_secs(wait)).await;
                continue;
            }

            // Fetch price-to-beat for both markets. Use ISO times so they match slug btc-updown-15m-{period_15} / btc-updown-5m-{period_5}.
            let event_start_15m_iso = format_timestamp_iso(period_15);
            let end_date_15m_iso = format_timestamp_iso(period_15 + MARKET_15M_DURATION_SECS);
            let event_start_5m_iso = format_timestamp_iso(period_5);
            let end_date_5m_iso = format_timestamp_iso(period_5 + MARKET_5M_DURATION_SECS);

            let m15_beat = self
                .api
                .get_crypto_price_to_beat("BTC", &event_start_15m_iso, "fifteen", &end_date_15m_iso)
                .await?;
            let m5_beat = self
                .api
                .get_crypto_price_to_beat("BTC", &event_start_5m_iso, "five", &end_date_5m_iso)
                .await?;

            let same_beat = match (m15_beat, m5_beat) {
                (Some(a), Some(b)) => (a - b).abs() < 0.01,
                _ => false,
            };

            if !same_beat {
                info!(
                    "Price-to-beat mismatch: 15m={:?} 5m={:?}. Polling again in {}s.",
                    m15_beat, m5_beat, poll_interval
                );
                sleep(Duration::from_secs(poll_interval)).await;
                continue;
            }

            info!(
                "Arb window active: 15m period {} (last 5m), 5m period {}, price-to-beat {:?}",
                period_15, period_5, m15_beat
            );

            let (m15_up, m15_down) = self.discovery.get_market_tokens(&m15).await?;
            let (m5_up, m5_down) = self.discovery.get_market_tokens(&m5).await?;

            return Ok((
                m15,
                m5,
                ArbTokens {
                    m15_up,
                    m15_down,
                    m5_up,
                    m5_down,
                },
                period_15,
            ));
        }
    }

    fn round_price(price: f64) -> f64 {
        let rounded = (price * 100.0).round() / 100.0;
        rounded.clamp(0.01, 0.99)
    }

    async fn place_limit_buy(&self, token_id: &str, price: f64) -> Result<OrderResponse> {
        let price = Self::round_price(price);
        let shares = self.config.strategy.shares;
        if self.config.strategy.simulation_mode {
            info!(
                "🎮 SIMULATION: Would place BUY {} shares @ ${:.4} for token {}",
                shares, price, &token_id[..token_id.len().min(12)]
            );
            return Ok(OrderResponse {
                order_id: Some(format!("SIM-{}", Utc::now().timestamp())),
                status: "SIMULATED".to_string(),
                message: Some("Simulated".to_string()),
            });
        }
        let order = OrderRequest {
            token_id: token_id.to_string(),
            side: "BUY".to_string(),
            size: shares.to_string(),
            price: price.to_string(),
            order_type: "LIMIT".to_string(),
        };
        self.api.place_order(&order).await
    }

    /// One arb round: run WebSocket until market ends; when sum < threshold place orders, wait 10s, verify, repeat.
    async fn run_arb_round(
        &self,
        tokens: &ArbTokens,
        period_15_start: i64,
    ) -> Result<()> {
        let prices: PricesSnapshot = Arc::new(RwLock::new(Default::default()));
        let asset_ids = vec![
            tokens.m15_up.clone(),
            tokens.m15_down.clone(),
            tokens.m5_up.clone(),
            tokens.m5_down.clone(),
        ];
        let ws_url = self.config.polymarket.ws_url.clone();
        let prices_clone = Arc::clone(&prices);
        let ws_handle = tokio::spawn(async move {
            if let Err(e) = run_market_ws(&ws_url, asset_ids, prices_clone).await {
                warn!("WebSocket exited: {}", e);
            }
        });

        let threshold = self.config.strategy.sum_threshold;
        let verify_secs = self.config.strategy.verify_fill_secs;
        let mut pending_placement = false;
        let mut last_place_time = 0i64;

        loop {
            let now = Utc::now().timestamp();
            if now >= period_15_start + MARKET_15M_DURATION_SECS {
                info!("15m market ended. Exiting arb round.");
                break;
            }

            if pending_placement {
                let elapsed = now - last_place_time;
                if elapsed >= verify_secs as i64 {
                    pending_placement = false;
                    // Verification is done in the block below when we set pending_placement = true
                }
                sleep(Duration::from_millis(50)).await;
                continue;
            }

            let snap = prices.read().await;
            let m15_up_ask = snap.get(&tokens.m15_up).and_then(|p| p.ask);
            let m15_down_ask = snap.get(&tokens.m15_down).and_then(|p| p.ask);
            let m5_up_ask = snap.get(&tokens.m5_up).and_then(|p| p.ask);
            let m5_down_ask = snap.get(&tokens.m5_down).and_then(|p| p.ask);
            drop(snap);

            let (sum1, sum2) = (
                m15_up_ask.zip(m5_down_ask).map(|(a, b)| a + b),
                m15_down_ask.zip(m5_up_ask).map(|(a, b)| a + b),
            );

            let (do_place_1, do_place_2) = (
                sum1.map(|s| s < threshold).unwrap_or(false),
                sum2.map(|s| s < threshold).unwrap_or(false),
            );

            if do_place_1 {
                let ask_a = m15_up_ask.unwrap();
                let ask_b = m5_down_ask.unwrap();
                info!(
                    "Arb opportunity: 15m Up ask {:.4} + 5m Down ask {:.4} = {:.4} < {:.2} — placing both",
                    ask_a, ask_b, ask_a + ask_b, threshold
                );
                let r1 = self.place_limit_buy(&tokens.m15_up, ask_a).await;
                let r2 = self.place_limit_buy(&tokens.m5_down, ask_b).await;
                match (&r1, &r2) {
                    (Ok(_), Ok(_)) => {
                        pending_placement = true;
                        last_place_time = Utc::now().timestamp();
                        let api = self.api.clone();
                        let params = VerifyRiskParams {
                            token_a: tokens.m15_up.clone(),
                            token_b: tokens.m5_down.clone(),
                            order_id_a: r1.as_ref().ok().and_then(|o| o.order_id.clone()),
                            order_id_b: r2.as_ref().ok().and_then(|o| o.order_id.clone()),
                            shares: self.config.strategy.shares,
                            verify_secs: self.config.strategy.verify_fill_secs,
                            simulation: self.config.strategy.simulation_mode,
                            label_a: "15m Up".to_string(),
                            label_b: "5m Down".to_string(),
                        };
                        tokio::spawn(async move {
                            verify_and_manage_risk(api, params).await;
                        });
                    }
                    _ => {
                        error!("Place failed: {:?} / {:?}", r1, r2);
                    }
                }
                sleep(Duration::from_millis(200)).await;
                continue;
            }

            if do_place_2 {
                let ask_a = m15_down_ask.unwrap();
                let ask_b = m5_up_ask.unwrap();
                info!(
                    "Arb opportunity: 15m Down ask {:.4} + 5m Up ask {:.4} = {:.4} < {:.2} — placing both",
                    ask_a, ask_b, ask_a + ask_b, threshold
                );
                let r1 = self.place_limit_buy(&tokens.m15_down, ask_a).await;
                let r2 = self.place_limit_buy(&tokens.m5_up, ask_b).await;
                match (&r1, &r2) {
                    (Ok(_), Ok(_)) => {
                        pending_placement = true;
                        last_place_time = Utc::now().timestamp();
                        let api = self.api.clone();
                        let params = VerifyRiskParams {
                            token_a: tokens.m15_down.clone(),
                            token_b: tokens.m5_up.clone(),
                            order_id_a: r1.as_ref().ok().and_then(|o| o.order_id.clone()),
                            order_id_b: r2.as_ref().ok().and_then(|o| o.order_id.clone()),
                            shares: self.config.strategy.shares,
                            verify_secs: self.config.strategy.verify_fill_secs,
                            simulation: self.config.strategy.simulation_mode,
                            label_a: "15m Down".to_string(),
                            label_b: "5m Up".to_string(),
                        };
                        tokio::spawn(async move {
                            verify_and_manage_risk(api, params).await;
                        });
                    }
                    _ => {
                        error!("Place failed: {:?} / {:?}", r1, r2);
                    }
                }
                sleep(Duration::from_millis(200)).await;
                continue;
            }

            sleep(Duration::from_millis(20)).await;
        }

        ws_handle.abort();
        Ok(())
    }

    pub async fn run(&self) -> Result<()> {
        info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
        info!("BTC 15m vs 5m arbitrage bot");
        info!("   Sum threshold: {} (place when sum of asks < this)", self.config.strategy.sum_threshold);
        info!("   Shares per side: {}", self.config.strategy.shares);
        info!("   Verify fill after {}s", self.config.strategy.verify_fill_secs);
        if self.config.strategy.simulation_mode {
            info!("   🎮 SIMULATION MODE — no real orders");
        }
        info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

        loop {
            let (m15_cid, m5_cid, tokens, period_15) = self.wait_for_arb_window().await?;
            info!("Running arb round: 15m {} … 5m {} …", &m15_cid[..20], &m5_cid[..20]);
            if let Err(e) = self.run_arb_round(&tokens, period_15).await {
                error!("Arb round error: {}", e);
            }
            sleep(Duration::from_secs(5)).await;
        }
    }
}
