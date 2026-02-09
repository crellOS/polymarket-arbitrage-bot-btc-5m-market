use crate::config::SignalConfig;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MarketSignal {
    Good,
    Bad,
    Unknown,
}

pub fn evaluate_place_signal(
    cfg: &SignalConfig,
    up_price: f64,
    down_price: f64,
    time_remaining_secs: i64,
) -> MarketSignal {
    if !cfg.enabled {
        return MarketSignal::Good;
    }

    let time_remaining_mins = time_remaining_secs / 60;

    if up_price >= cfg.clear_threshold && time_remaining_mins > cfg.clear_remaining_mins as i64 {
        return MarketSignal::Bad;
    }
    if down_price >= cfg.clear_threshold && time_remaining_mins > cfg.clear_remaining_mins as i64 {
        return MarketSignal::Bad;
    }

    let up_stable = up_price >= cfg.stable_min && up_price <= cfg.stable_max;
    let down_stable = down_price >= cfg.stable_min && down_price <= cfg.stable_max;

    if up_stable && down_stable {
        return MarketSignal::Good;
    }

    MarketSignal::Bad
}

pub fn is_danger_signal(cfg: &SignalConfig, matched_token_price: f64) -> bool {
    if !cfg.enabled {
        return false;
    }
    matched_token_price <= cfg.danger_price
}
