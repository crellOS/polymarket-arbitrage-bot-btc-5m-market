# Pre-Limit Order Bot – How It Works (Signal-Based Strategy)

## Core idea

The bot trades **1-hour Up/Down markets** (BTC, ETH, SOL, XRP). It places **limit BUY orders** on both Up and Down at a fixed price (e.g. $0.45). Since Up + Down ≈ $1, buying both at ≤ $0.45 each yields profit when merged.

**Critical :** The bot does **not** place orders blindly. It analyzes the current market and only places orders when conditions are favourable (good signal). It also sells early when danger is detected.

---

## Critical risk (why signals matter)

**Scenario:** BTC 1h market. Bot placed pre-orders for both sides. Only Up matched at $0.45. Market trends down strongly. Up price keeps falling. After 57 minutes (or whatever minutes u set for stop-loss), bot sells Up tokens — but at this point Up price is $0.01.

**Result:** Loss ≈ (0.45 - 0.01) × 100 = **$44** (instead of locking profit by merging).

**Consideration:**  
- Don't place pre-orders in every market — analyze first.  
- When one side is matched, monitor for danger and sell early (don't wait for timeout).

---

## Signal definitions

### Good signal (place orders)

- **Stable market:** Both Up and Down prices are in a balanced range (e.g. $0.35–$0.65).  
  Small vibration, gradual movement, neither side dominating.
- **Not too clear:** No single token at $0.99+ with significant time left (e.g. > 15 mins).  
  If one side is already 0.99 with 20 mins left, the underlying (BTC/ETH) is moving strongly — dangerous to place both sides for the next hour.

### Bad signal (skip placing orders)

- **Market too clear:** Up ≥ 0.99 or Down ≥ 0.99, and remaining time > 15 mins (configurable).  
  Strong directional move; next hour could be similarly one-sided.
- **Unstable:** One token falling sharply (e.g. from 0.5 toward 0.1) — high volatility, risky.
- **Massive move:** Underlying price is rising/falling strongly; market is “decided” too early.

---

## Example 1: Good signal → place pre-orders for next hour

**Setup:** Current market (4–5 PM): Up $0.48, Down $0.52. Time left: 10 mins(configurable). Next hour (5–6 PM) in 3 mins.  
**Config:** `place_order_before_mins: 10`, `price_limit: 0.45`, `shares: 100`.

**Signal check:** Both tokens in [0.35, 0.65], neither at 0.99. **Good signal.**

**Rule:** Good signal + time until next hour ≤ 5 minutes → place limit orders for next hour.

**Action:**  
- Place BUY 100 Up @ $0.45  
- Place BUY 100 Down @ $0.45  

**Position:** Orders on the book for 5–6 PM market.

---

## Example 2: Bad signal → skip next hour

**Setup:** Current market (4–5 PM): Up $0.99, Down $0.02. Time left: 20 mins. BTC rising strongly.  
**Signal check:** Up ≥ 0.99, time left > 15 mins. **Bad signal.**

**Rule:** Bad signal → do **not** place orders for the next hour (5–6 PM).

**Action:** None. Next market (5–6 PM) starts with no orders.

---

## Example 3: No orders, then good signal → place for current market (dynamic pricing)

**Setup:** Market (5–6 PM) running. We have no orders (we skipped pre-order). Up $0.63, Down $0.38. Time left: 45 mins.  
**Signal check:** Both tokens in [0.35, 0.65]. **Good signal.**

**Rule:** No orders for current market + good signal → place orders for **current** running market with **dynamic pricing**.

**Dynamic pricing:** Cheaper side at current price, opposite side at current − $0.02.
- Down is cheaper ($0.38): place Down @ $0.38 (current price)
- Up is more expensive ($0.63): place Up @ $0.61 (current − $0.02)

**Action:**  
- Place BUY 100 Down @ $0.38  
- Place BUY 100 Up @ $0.61  

**Position:** Orders on the book for current 5–6 PM market. We don't overpay — we match the cheaper side at market and try to get a discount on the other. If both fill before close, we merge.

---

## Example 4: One side matched, danger signal → sell early

**Setup:** Up matched at $0.45. Down not filled. Up price has fallen to $0.12 (market going down).  
**Config:** `sell_unmatched_after_mins: 57`, `signal_danger_price: 0.15`. Only 20 mins passed.

**Signal check:** Matched token (Up) price ≤ 0.15. **Danger signal.**

**Rule:** One side matched + danger signal → sell matched side immediately (do not wait for `sell_unmatched_after_mins`).

**Action:**  
- Sell 100 Up at $0.12  
- Cancel Down order  

**PnL:** Cost $45, sell $12. **Loss ≈ $33** — but avoids holding to $0.01 (which would be ~$44 loss).

---

## Example 5: One side matched, no danger → wait for timeout

**Setup:** Up matched at $0.45. Down not filled. Up price $0.42. 57 minutes passed.  
**Signal check:** Matched token above danger threshold. No danger.

**Rule:** One side matched + no danger + timeout reached → sell matched side, cancel other order (standard risk cut).

**Action:**  
- Sell 100 Up at $0.42  
- Cancel Down order  

**PnL:** Loss = (0.45 - 0.42) × 100 = **$3**.

---

## Example 6: Both orders fill → merge

**Setup:** Both limit orders filled. Up and Down ASK were at or below $0.45.

**Rule:** Both Up and Down filled → merge positions.

**Action:** Merge 100 Up + 100 Down into USDC.

**PnL:** Cost = $90. Redeem = $100. **Profit = $10**.

---

## Config summary

| Config | Example | Meaning |
|--------|---------|---------|
| `price_limit` | 0.45 | Max price per share (Up and Down) |
| `shares` | 100 | Size per side |
| `place_order_before_mins` | 5 | Minutes before next hour to consider pre-orders |
| `sell_unmatched_after_mins` | 57 | Fallback: sell unmatched side after this many minutes |
| `signal_enabled` | true | Use signal-based logic (if false, always place) |
| `signal_stable_min` | 0.35 | Good: both tokens ≥ this |
| `signal_stable_max` | 0.65 | Good: both tokens ≤ this |
| `signal_clear_threshold` | 0.99 | Bad: either token ≥ this |
| `signal_clear_remaining_mins` | 15 | Bad: when one token is “clear” with this much time left |
| `signal_danger_price` | 0.15 | Danger: matched token ≤ this → sell early |
| `signal_mid_market_enabled` | true | Allow placing orders for current market on good signal |

**Note:** `price_limit` is used for pre-orders (fixed price before market starts). Mid-market uses dynamic pricing (cheaper side at current, opposite at current − $0.02).

---

## Run flow (simplified)

1. **Every `check_interval_ms`:**
   - If we have orders → check fills, merge if both filled.
   - If one side matched → check danger: if matched price ≤ `signal_danger_price`, sell early. Else, if `sell_unmatched_after_mins` passed, sell and cancel.
   - If within `place_order_before_mins` of next hour and no orders for next hour:
     - Fetch current market prices.
     - If good signal → place pre-orders for next hour.
     - If bad signal → skip.
   - If no orders for **current** market and `signal_mid_market_enabled`:
     - Fetch current market prices.
     - If good signal → place orders for current market using **dynamic pricing** (cheaper side at current price, opposite side at current − $0.02).
2. After market expiry → clear state.
