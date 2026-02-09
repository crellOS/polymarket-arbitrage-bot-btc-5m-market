mod api;
mod config;
mod models;
mod discovery;
mod signals;
mod strategy;


use anyhow::Result;
use clap::Parser;
use config::{Args, Config};
use std::io::Write;
use std::sync::Arc;
use api::PolymarketApi;
use strategy::PreLimitStrategy;

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::from_default_env()
        .filter_level(log::LevelFilter::Info)
        .format(|buf, record| {
            writeln!(buf, "{}", record.args())
        })
        .init();

    let args = Args::parse();
    let config = Config::load(&args.config)?;
    let shares = config.strategy.shares;
    let price = config.strategy.price_limit;
    let cost_per_side = shares * price;
    let payout_per_trade = cost_per_side * 2.0;
    const N_ASSETS: u32 = 4;
    let four_assets = (N_ASSETS as f64) * cost_per_side;

    eprintln!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    eprintln!("📋 Confirming configuration");
    eprintln!("   shares per side        {:.0}", shares);
    eprintln!("   ave price per share   ${:.2}", price);
    eprintln!("   payout per trade      ${:.0} × 2 = ${:.0}", cost_per_side, payout_per_trade);
    eprintln!("   {} assets              ${:.0}", N_ASSETS, four_assets);
    eprintln!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

    eprintln!("🚀 Starting Polymarket Pre-Limit Order Bot");
    if config.strategy.simulation_mode {
        eprintln!("🎮 SIMULATION MODE ENABLED - No real orders will be placed");
        eprintln!("   Orders will match when prices hit ${:.2} or below", config.strategy.price_limit);
    }
    eprintln!("📈 Strategy: Placing Up/Down limit orders at ${:.2} for 1h markets (BTC, ETH, SOL, XRP)", config.strategy.price_limit);
    if config.strategy.signal.enabled {
        eprintln!("   📡 Signal-based risk management: enabled (place on good signal, skip on bad, sell early on danger)");
    }

    let api = Arc::new(PolymarketApi::new(
        config.polymarket.gamma_api_url.clone(),
        config.polymarket.clob_api_url.clone(),
        config.polymarket.api_key.clone(),
        config.polymarket.api_secret.clone(),
        config.polymarket.api_passphrase.clone(),
        config.polymarket.private_key.clone(),
        config.polymarket.proxy_wallet_address.clone(),
        config.polymarket.signature_type,
    ));

    if config.polymarket.private_key.is_some() {
        if let Err(e) = api.authenticate().await {
            log::error!("Authentication failed: {}", e);
            anyhow::bail!("Authentication failed. Please check your credentials.");
        }
    } else {
        log::warn!("⚠️ No private key provided. Bot will only be able to monitor markets.");
    }

    let strategy = PreLimitStrategy::new(api, config);
    strategy.run().await?;

    Ok(())
}
