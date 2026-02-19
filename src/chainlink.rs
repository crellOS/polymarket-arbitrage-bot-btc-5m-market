//! Price-to-beat from Polymarket RTDS Chainlink (crypto_prices_chainlink) for multiple symbols.
//! Per docs: https://docs.polymarket.com/developers/RTDS/RTDS-crypto-prices
//! Price-to-beat is set when we receive a message whose feed_ts is in [period_start, period_start+2).

use crate::rtds::{run_rtds_chainlink_multi, PriceCacheMulti};
use anyhow::Result;
use log::warn;
use std::sync::Arc;
use tokio::time::Duration;

/// Spawn RTDS Chainlink stream for given symbols; price-to-beat is written per (symbol, period).
pub async fn run_chainlink_multi_poller(
    rtds_ws_url: String,
    symbols: Vec<String>,
    price_cache_15: PriceCacheMulti,
    price_cache_5: PriceCacheMulti,
) -> Result<()> {
    let cache_15 = Arc::clone(&price_cache_15);
    let cache_5 = Arc::clone(&price_cache_5);

    tokio::spawn(async move {
        loop {
            if let Err(e) = run_rtds_chainlink_multi(
                &rtds_ws_url,
                &symbols,
                cache_15.clone(),
                cache_5.clone(),
            )
            .await
            {
                warn!("RTDS Chainlink stream exited: {} (reconnecting in 5s)", e);
            }
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    });

    tokio::time::sleep(Duration::from_secs(2)).await;
    Ok(())
}
