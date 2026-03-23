# HawkesFlow Strategy — External Review Package

**Date**: 2026-03-24
**Project**: polycli (Polymarket BTC 5-minute binary prediction market bot)
**Strategy**: Hawkes Flow Excitation — order flow self/cross-excitation dynamics with VPIN toxicity gate

---

## Table of Contents

1. [Strategy Overview](#1-strategy-overview)
2. [Core Types](#2-core-types)
3. [Strategy Engine](#3-strategy-engine)
4. [Pipeline Integration](#4-pipeline-integration)
5. [Feed / Data Layer](#5-feed--data-layer)
6. [Autoresearch Results](#6-autoresearch-results)
7. [Known Issues](#7-known-issues)
8. [Questions for Reviewer](#8-questions-for-reviewer)

---

## 1. Strategy Overview

### What It Does

HawkesFlow is an order flow excitation strategy for Polymarket BTC Up/Down 5-minute binary prediction markets. It uses:

1. **Two Hawkes process estimators** (buy-side, sell-side) to model self-exciting order flow
2. **HEAI (Hawkes Excitation Asymmetry Index)** as the core entry signal
3. **VPIN (Volume-synchronized Probability of Informed Trading)** as a toxicity gate
4. **Configurable TP/SL** calibrated for prediction market dynamics

### Key Insight

Prediction markets have binary outcomes — YES pays $1 if correct, $0 if wrong. Prices jump discontinuously (e.g., 0.40 → 0.80 on news). The strategy uses Hawkes process intensities to detect asymmetric buy/sell pressure building *before* it manifests in price.

### Literature Basis

- Nittur & Jain (2025): Hawkes SOE kernel best for OFI forecasting
- Busetto & Formentin (2023): Hawkes+COE outperforms benchmarks on crypto LOB
- Kitvanitphasu et al. (2026): VPIN significantly predicts BTC price jumps
- Elomari-Kessab et al. (2024): Microstructure modes via PCA on flow/returns

---

## 2. Core Types

### `Observation` (from `src/bot/strategy/mod.rs`)

```rust
pub struct Observation {
    pub ts: i64,                    // timestamp (milliseconds)
    pub condition_id: String,
    pub market_slug: String,
    pub yes_bid: f64,
    pub yes_ask: f64,
    pub no_bid: f64,
    pub no_ask: f64,
    pub yes_mid: f64,
    pub no_mid: f64,
    pub book_sum: f64,              // yes_ask + no_ask
    pub time_remaining_s: i64,
    pub indicator_5s: IndicatorState,
    pub indicator_1m: IndicatorState,
    pub fair_value_prob: Option<f64>,
    pub qlib_score: Option<f64>,
}
```

### `StrategyDecision` (from `src/bot/strategy/types.rs`)

```rust
pub enum StrategyDecision {
    Hold,
    Block { reason: String },
    Enter { direction: Direction, reason: EntryReason },
    Exit { position_id: String, reason: ExitReason },
}
```

### `ExitReason` (from `src/bot/strategy/types.rs`)

```rust
pub enum ExitReason {
    MomentumReversal,
    TakeProfit { pnl_pct: f64 },
    StopLoss { pnl_pct: f64 },
    TimeExpiry { seconds_remaining: i64 },
    RiskGate { reason: String },
    MaxHoldingTime { seconds_held: u64 },
    FairValueReversal,
    QlibScoreDrop,
}
```

### `DualSnapshot` (from `src/bot/feed_base.rs`)

```rust
pub struct DualSnapshot {
    pub yes: MarketSnapshot,  // YES token orderbook
    pub no: MarketSnapshot,   // NO token orderbook
    pub ts_exchange: f64,     // exchange timestamp (milliseconds)
}
```

### `ShadowPosition` (from `src/bot/shadow.rs`)

```rust
pub struct ShadowPosition {
    pub active_entry: Option<EntrySignal>,
    pub token_side: Option<TokenSide>,   // Yes or No
    pub entry_price: f64,
    pub size: f64,
    pub realized_pnl: f64,
    pub bankroll_usd: f64,
    pub position_size_usd: f64,
    pub entry_timestamp: u64,
    // ... more fields
}
```

---

## 3. Strategy Engine

**File**: `src/bot/strategy/hawkes_flow.rs` (637 lines)

### Config

```rust
pub struct HawkesFlowConfig {
    pub kernel_decay: f64,          // 0.5 — exponential decay rate
    pub min_heai: f64,              // 0.04 — minimum asymmetry for entry
    pub max_heai: f64,              // 0.60 — confidence capping
    pub vpin_threshold: f64,        // 0.05 — toxicity gate
    pub vpin_window: usize,         // 50 — VPIN calculation window
    pub min_time_remaining: i64,    // 45 seconds
    pub max_entry_prob: f64,        // 0.88 — avoid near-certain outcomes
    pub min_entry_prob: f64,        // 0.18 — avoid near-certain outcomes
    pub min_bb_width: f64,          // 0.0 — disabled (spread proxy)
    pub cooldown_observations: usize, // 3 — observations after exit
    pub base_tp_pct: f64,           // 0.30 — 30% take profit
    pub base_sl_pct: f64,           // -0.10 — 10% stop loss
}
```

### HawkesEstimator

Models self-exciting order flow using exponential kernel:

```
λ(t) = μ + Σᵢ α · exp(-β · (t - tᵢ))
```

- `mu = 0.1` (background intensity)
- `alpha = 0.3` (excitation coefficient)
- `beta = config.kernel_decay` (decay rate, currently 0.5 → 500ms half-life)

**Update logic**:
```rust
fn update(&mut self, event: FlowEvent) {
    if let Some(last) = self.last_ts {
        let dt = (event.timestamp - last) as f64 / 1000.0; // ms → seconds
        if dt > 0.0 {
            self.current_intensity =
                self.mu + (self.current_intensity - self.mu) * (-self.beta * dt).exp();
        }
    }
    self.current_intensity += self.alpha * event.magnitude;
}
```

### VpinEstimator

Volume-synchronized Probability of Informed Trading. Tracks order flow toxicity via volume imbalance across completed buckets.

### HEAI (Hawkes Excitation Asymmetry Index)

```rust
fn compute_heai(buy_intensity: f64, sell_intensity: f64) -> f64 {
    let total = buy_intensity + sell_intensity;
    if total < 1e-10 { return 0.0; }
    (buy_intensity - sell_intensity) / total
}
```

- Range: [-1.0, +1.0]
- Positive = buy pressure dominates → enter Yes
- Negative = sell pressure dominates → enter No

### Entry Logic (`check_entry`)

All conditions must pass:
1. Price in range: `min_entry_prob <= p <= max_entry_prob`
2. VPIN gate: `vpin >= vpin_threshold`
3. BB width: `bb_width >= min_bb_width` (currently disabled at 0.0)
4. Time remaining: `time_remaining >= min_time_remaining`
5. Cooldown: `cooldown_counter == 0`
6. HEAI threshold: `|heai| >= min_heai`
7. HEAI momentum: last 3 HEAI values all same sign as current

### Exit Logic (`check_exit`)

Checked in order:
1. Time expiry: `time_remaining < 15s` → exit at market price
2. Take profit: `move_in_favor > base_tp_pct` (currently +30%)
3. Stop loss: `move_in_favor < base_sl_pct` (currently -10%)
4. HEAI reversal: `|heai| > 0.10` against position direction
5. Extreme probability: `p > 0.95` or `p < 0.05`

### Decision Flow (`decide`)

```rust
fn decide(&mut self, obs: &Observation) -> StrategyDecision {
    // 1. Infer flow event from price movement
    if let Some(event) = self.infer_flow_event(obs) {
        if event.is_buy { self.buy_hawkes.update(event); }
        else { self.sell_hawkes.update(event); }
        self.vpin.add_trade(event.is_buy, volume);
    }

    // 2. Compute HEAI and VPIN
    let heai = compute_heai(buy_intensity, sell_intensity);
    let vpin_val = self.vpin.vpin();

    // 3. Check exit first if position active
    if self.active_position {
        if let Some(exit_reason) = self.check_exit(obs, heai) {
            self.active_position = false;
            return StrategyDecision::Exit { reason: exit_reason };
        }
    }

    // 4. Check entry if no position
    if !self.active_position {
        if let Some((direction, reason)) = self.check_entry(obs, heai, vpin_val) {
            self.active_position = true;
            return StrategyDecision::Enter { direction, reason };
        }
    }

    StrategyDecision::Hold
}
```

---

## 4. Pipeline Integration

**File**: `src/bot/pipeline/mod.rs` — `process_hawkesflow_snapshot()` function

### How It Works

1. Receives a `DualSnapshot` (YES + NO orderbook)
2. Constructs an `Observation` from the snapshot
3. Calls `engine.decide(&obs)` to get strategy decision
4. Maps decision to `ShadowPosition` actions:
   - **Enter**: Sets `shadow.token_side`, `shadow.entry_price = yes_ask` (not mid!)
   - **Exit**: Calculates PnL, updates bankroll, calls `shadow.reset()`

### Key Detail: Entry Price

```rust
// Strategy tracks entry at mid price
self.entry_price = Some(obs.yes_mid);

// But shadow enters at ask price (what you actually pay)
shadow.entry_price = yes_ask;
```

The strategy's exit thresholds use `obs.yes_mid` compared to `self.entry_price` (mid at entry), not the actual ask price the shadow paid. This means exit calculations don't account for the bid-ask spread cost.

### Timestamp Handling

- `obs.ts = snapshot.ts_exchange as i64` (milliseconds from exchange)
- Hawkes estimator divides by 1000 to get seconds for decay calculation
- `epoch_seconds = ts_exchange / 1000` used for time_remaining calculation

---

## 5. Feed / Data Layer

**File**: `src/bot/feed_base.rs`

### Data Sources

1. **WebSocket** (validate-btc): Real-time orderbook updates via Polymarket WebSocket API
   - Events arrive as `BookDeltaEvent` with `ts_exchange` in milliseconds
   - `recv_next()` returns one event at a time (event-by-event processing)

2. **CSV Replay** (backtest-pmxt): Recorded tick data
   - Format: `timestamp,market_slug,yes_bid,yes_ask,no_bid,no_ask,time_remaining`
   - Timestamps in milliseconds
   - EventByEvent mode: only emits snapshots on actual price changes

### BookDeltaEvent

```rust
pub struct BookDeltaEvent {
    pub token_id: String,
    pub side: OutcomeSide,        // Yes or No
    pub ts_exchange: f64,         // milliseconds
    pub best_bid: f64,
    pub best_ask: f64,
    pub change_price: Option<f64>,
    pub change_size: Option<f64>,
    pub source: &'static str,     // "ws" or "poll"
}
```

---

## 6. Autoresearch Results

### Iteration Log (20 iterations)

```
iter  commit   PnL%    fitness  status   description
0     2f6d822  -11.41  0.497    base     133 trades, 42.1% WR, PF 1.31
1     -        -37.20  0.364    discard  min_heai 0.08 — too few trades
2     a7849fb  +9.93   0.535    KEEP     HEAI exit ±0.05→±0.10 — less whipsawing
3     5a0383f  +11.10  0.612    KEEP     SL -8%→-6% — tighter stops
4     -        -1.54   0.541    discard  TP +12%→+10% — cut winners short
5     a03346d  +31.12  0.672    KEEP     min_heai 0.03→0.05 — better entries
6     -        -13.59  0.520    discard  kernel_decay 0.3 — noisier
7     -        +16.76  0.629    discard  cooldown 2 — worse than best
8     -        +0.22   0.704    discard  TP 50% — too few hits
9     -        -103.17 0.472    discard  SL -15% — account blown
10    -        +2.14   0.741    discard  TP 40% — marginally worse
11    -        -26.37  0.777    discard  SL -8% — lost money
--- PM-optimized baseline (configurable TP/SL) ---
PM    bdca69d  +7.97   0.764    KEEP     TP=30%, SL=-10% — PM-optimized
11    -        +0.22   0.704    discard  TP 50%
12    -        -103.17 0.472    discard  SL -15%
13    -        +2.14   0.741    discard  TP 40%
14    -        -26.37  0.777    discard  SL -8%
15    56debb6  +27.85  0.817    KEEP     min_heai 0.05→0.04 — more entries
16    -        -9.18   0.668    discard  kernel_decay 0.4
17    -        +26.19  0.750    discard  HEAI exit ±0.08
18    -        +27.85  0.817    discard  min_time 45→30 — no effect
19    -        -15.81  0.693    discard  min_heai 0.06
20    13632d1  +24.86  0.828    KEEP     min_entry_prob 0.15→0.18 — PF 1.92
```

### Best Configuration (fitness 0.828)

```
  kernel_decay: 0.5        min_heai: 0.04
  max_heai: 0.60           vpin_threshold: 0.05
  vpin_window: 50          min_time_remaining: 45
  max_entry_prob: 0.88     min_entry_prob: 0.18
  min_bb_width: 0.0        cooldown_observations: 3
  base_tp_pct: 0.30        base_sl_pct: -0.10
  HEAI_exit: ±0.10

  Performance: 57 trades, 38.6% WR, PF 1.86, PnL +27.85%, drawdown 14.82%
```

### Fitness Formula

```
fitness = (profit_factor * 0.35) + (win_rate/100 * 0.25) + (total_pnl_pct/100 * 0.25)
```

---

## 7. Known Issues

### 7.1 Entry Price Mismatch

The strategy tracks `entry_price = obs.yes_mid` but the shadow actually enters at `yes_ask`. Exit thresholds (TP/SL) are calculated against the mid price, not the actual entry cost. This means:
- A "30% TP" from mid entry might only be ~25% from actual ask entry
- Stop losses are similarly miscalibrated

### 7.2 VPIN Bucket Completion Never Triggers

VPIN requires `current_bucket_volume >= avg_bucket_volume` to complete a bucket. The `estimate_volume()` returns `obs.book_sum` (yes_ask + no_ask) which is typically 0.9-1.1. The initial bucket threshold is 10.0. This means VPIN buckets almost never complete in backtest, leaving VPIN at its default 0.5 (which always passes the 0.05 threshold).

### 7.3 No Trailing Stop

The strategy uses fixed TP/SL. In prediction markets where winners can go from 0.40 → 1.00, a fixed +30% TP leaves significant money on the table. A trailing stop would let winners run while protecting gains.

### 7.4 HEAI Exit Threshold Is Hardcoded

The HEAI reversal exit at ±0.10 is hardcoded, not configurable via HawkesFlowConfig. This prevents optimization of this parameter.

### 7.5 No Bid-Ask Spread Cost

PnL calculations don't account for the spread between bid and ask. In prediction markets, spreads can be 1-5% depending on liquidity.

### 7.6 Shadow Position Reset Blocks Future Entries

When `shadow.reset()` is called after a loss, it sets `yes_blocked = true` or `no_blocked = true`. This prevents future entries on the same side, which may be overly restrictive.

---

## 8. Questions for Reviewer

1. **Is the Hawkes model correctly specified?** The exponential kernel with α=0.3, β=0.5 — are these reasonable for sub-second prediction market data?

2. **Is the HEAI signal actually predictive?** The signal infers buy/sell from price direction, but in prediction markets price moves can be driven by information arrival, not order flow. Is this a valid assumption?

3. **Should we use a trailing stop instead of fixed TP?** Given prediction market prices can jump 0.40 → 0.80, a trailing stop (e.g., "exit if drops 5% from running max") might capture more of the move.

4. **Is the entry price mismatch (mid vs ask) a real problem?** The strategy decides based on mid price but enters at ask. How much edge does this cost?

5. **Is VPIN meaningful for prediction markets?** VPIN was designed for continuous limit order books. Prediction markets have discontinuous price discovery. Is it a valid toxicity measure here?

6. **What's the risk of overfitting?** 20 iterations of parameter optimization on 5 CSV files (14 markets). Is the fitness score reliable?

---

## File Listing

All files referenced in this review:

| File | Lines | Purpose |
|------|-------|---------|
| `src/bot/strategy/hawkes_flow.rs` | 637 | Strategy engine (Hawkes, VPIN, HEAI, entry/exit) |
| `src/bot/strategy/types.rs` | 152 | Core types (Direction, ExitReason, StrategyDecision) |
| `src/bot/strategy/mod.rs` | 166 | Module exports, Observation struct, StrategyEngine trait |
| `src/bot/pipeline/mod.rs` | 2598 | Pipeline: process_hawkesflow_snapshot(), backtest runner |
| `src/bot/feed_base.rs` | 654 | Data feed: DualSnapshot, BookDeltaEvent, WebSocket source |
| `src/bot/shadow.rs` | 354 | ShadowPosition: PnL tracking, bankroll management |
| `research/HAWKES_FLOW_REPORT.md` | 212 | Research report with literature review |
| `autoresearch_results.tsv` | 20 | Parameter optimization log |
