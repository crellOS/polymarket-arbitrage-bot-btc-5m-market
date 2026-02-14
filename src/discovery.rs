use crate::api::PolymarketApi;
use anyhow::Result;
use std::sync::Arc;

pub const MARKET_15M_DURATION_SECS: i64 = 15 * 60; // 900
pub const MARKET_5M_DURATION_SECS: i64 = 5 * 60;  // 300

/// BTC 15m slug: btc-updown-15m-{timestamp}
pub fn build_15m_slug(period_start_unix: i64) -> String {
    format!("btc-updown-15m-{}", period_start_unix)
}

/// BTC 5m slug: btc-updown-5m-{timestamp}
pub fn build_5m_slug(period_start_unix: i64) -> String {
    format!("btc-updown-5m-{}", period_start_unix)
}

/// Current 15-minute period start (Unix, aligned to 15m boundaries).
pub fn current_15m_period_start() -> i64 {
    let now = chrono::Utc::now().timestamp();
    (now / MARKET_15M_DURATION_SECS) * MARKET_15M_DURATION_SECS
}

/// Current 5-minute period start (Unix, aligned to 5m boundaries).
pub fn current_5m_period_start() -> i64 {
    let now = chrono::Utc::now().timestamp();
    (now / MARKET_5M_DURATION_SECS) * MARKET_5M_DURATION_SECS
}

/// True when we're in the last 5 minutes of the current 15m market (overlap with 5m for arb).
pub fn is_last_5min_of_15m(now_ts: i64, period_15m_start: i64) -> bool {
    let elapsed = now_ts - period_15m_start;
    elapsed >= 10 * 60 && elapsed < 15 * 60 // 600..900 sec
}

/// Parse price-to-beat from market question (e.g. "Will Bitcoin be above $97,500 at ...").
pub fn parse_price_to_beat_from_question(question: &str) -> Option<f64> {
    let q = question.to_lowercase();
    let idx = q.find("above ").or_else(|| q.find('$'))?;
    let after = &question[idx..];
    let mut num_start_byte = 0;
    for (i, c) in after.char_indices() {
        if c == '$' || c.is_ascii_digit() {
            num_start_byte = if c == '$' {
                i + c.len_utf8()
            } else {
                i
            };
            break;
        }
    }
    let num_str: String = after[num_start_byte..]
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.' || *c == ',')
        .filter(|c| *c != ',')
        .collect();
    if num_str.is_empty() {
        return None;
    }
    num_str.parse::<f64>().ok()
}

pub struct MarketDiscovery {
    api: Arc<PolymarketApi>,
}

impl MarketDiscovery {
    pub fn new(api: Arc<PolymarketApi>) -> Self {
        Self { api }
    }

    pub async fn get_market_tokens(&self, condition_id: &str) -> Result<(String, String)> {
        let details = self.api.get_market(condition_id).await?;
        let mut up_token = None;
        let mut down_token = None;

        for token in details.tokens {
            let outcome = token.outcome.to_uppercase();
            if outcome.contains("UP") || outcome == "1" {
                up_token = Some(token.token_id);
            } else if outcome.contains("DOWN") || outcome == "0" {
                down_token = Some(token.token_id);
            }
        }

        let up = up_token.ok_or_else(|| anyhow::anyhow!("Up token not found"))?;
        let down = down_token.ok_or_else(|| anyhow::anyhow!("Down token not found"))?;

        Ok((up, down))
    }

    /// Fetch BTC 15m market by period start; returns condition_id and price-to-beat if parseable.
    pub async fn get_15m_market(&self, period_start: i64) -> Result<Option<(String, Option<f64>)>> {
        let slug = build_15m_slug(period_start);
        let market = match self.api.get_market_by_slug(&slug).await {
            Ok(m) => m,
            Err(_) => return Ok(None),
        };
        if !market.active || market.closed {
            return Ok(None);
        }
        let price_to_beat = parse_price_to_beat_from_question(&market.question);
        Ok(Some((market.condition_id, price_to_beat)))
    }

    /// Fetch BTC 5m market by period start; returns condition_id and price-to-beat if parseable.
    pub async fn get_5m_market(&self, period_start: i64) -> Result<Option<(String, Option<f64>)>> {
        let slug = build_5m_slug(period_start);
        let market = match self.api.get_market_by_slug(&slug).await {
            Ok(m) => m,
            Err(_) => return Ok(None),
        };
        if !market.active || market.closed {
            return Ok(None);
        }
        let price_to_beat = parse_price_to_beat_from_question(&market.question);
        Ok(Some((market.condition_id, price_to_beat)))
    }
}
