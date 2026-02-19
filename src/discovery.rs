use crate::api::PolymarketApi;
use anyhow::Result;
use chrono::{TimeZone, Timelike};
use chrono_tz::America::New_York;
use std::sync::Arc;

pub const MARKET_15M_DURATION_SECS: i64 = 15 * 60; // 900
pub const MARKET_5M_DURATION_SECS: i64 = 5 * 60;  // 300

/// Polymarket aligns 15m/5m markets to Eastern Time (ET). Period start = start of current window in ET, as Unix timestamp.
fn period_start_et_unix(minutes: i64) -> i64 {
    let utc_now = chrono::Utc::now();
    let et = New_York;
    let now_et = utc_now.with_timezone(&et);
    let minute_floor = (now_et.minute() as i64 / minutes) * minutes;
    let truncated_naive = now_et
        .date_naive()
        .and_hms_opt(now_et.hour(), minute_floor as u32, 0)
        .unwrap();
    let dt_et = et
        .from_local_datetime(&truncated_naive)
        .single()
        .or_else(|| et.from_local_datetime(&truncated_naive).earliest())
        .expect("ET period start");
    dt_et.timestamp()
}

/// BTC 15m slug: btc-updown-15m-{timestamp}
pub fn build_15m_slug(period_start_unix: i64) -> String {
    format!("btc-updown-15m-{}", period_start_unix)
}

/// 5m slug for any symbol: {symbol}-updown-5m-{timestamp} (e.g. btc, eth, sol, xrp).
pub fn build_5m_slug(symbol: &str, period_start_unix: i64) -> String {
    format!("{}-updown-5m-{}", symbol.to_lowercase(), period_start_unix)
}

/// Current 15-minute period start (Unix). Aligned to 15m boundaries in Eastern Time (Polymarket uses ET).
pub fn current_15m_period_start() -> i64 {
    period_start_et_unix(15)
}

/// Current 5-minute period start (Unix). Aligned to 5m boundaries in Eastern Time (Polymarket uses ET).
pub fn current_5m_period_start() -> i64 {
    period_start_et_unix(5)
}

/// ET-aligned period start (Unix) that contains the given timestamp. Used to match RTDS price timestamp to market.
pub fn period_start_et_unix_for_timestamp(ts_sec: i64, minutes: i64) -> i64 {
    let utc_dt = match chrono::Utc.timestamp_opt(ts_sec, 0).single() {
        Some(dt) => dt,
        None => return ts_sec,
    };
    let et = New_York;
    let et_dt = utc_dt.with_timezone(&et);
    let minute_floor = (et_dt.minute() as i64 / minutes) * minutes;
    let truncated_naive = et_dt
        .date_naive()
        .and_hms_opt(et_dt.hour(), minute_floor as u32, 0)
        .unwrap();
    let dt_et = et
        .from_local_datetime(&truncated_naive)
        .single()
        .or_else(|| et.from_local_datetime(&truncated_naive).earliest())
        .expect("ET period for timestamp");
    dt_et.timestamp()
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

    /// Fetch 5m market by symbol and period start; returns condition_id and price-to-beat if parseable.
    pub async fn get_5m_market(&self, symbol: &str, period_start: i64) -> Result<Option<(String, Option<f64>)>> {
        let slug = build_5m_slug(symbol, period_start);
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
