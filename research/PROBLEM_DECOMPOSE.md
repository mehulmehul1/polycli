# Problem Decompose: OBI/VAMP/Mean-Reversion Edge in Polymarket 5m BTC Markets

**Stage**: ResearchClaw Stage 2 — PROBLEM_DECOMPOSE
**Date**: 2026-03-23
**Goal**: Find exploitable edge in Polymarket 5-minute BTC binary markets using OBI (Order Book Imbalance), VAMP (Volume-Adjusted Mid Price), and mean reversion signals.
**Targets**: 53%+ directional accuracy, Sharpe > 1.0, EV > $0.005/contract, backtested on 1000+ contracts.

---

## Existing Codebase Assets

| Asset | Location | Status |
|-------|----------|--------|
| `StrategyEngine` trait | `src/bot/strategy/mod.rs` | Working |
| `IndicatorEngine` (EMA/RSI/BB/Slope) | `src/bot/indicators.rs` | Working |
| `SignalEngine` / `HeuristicEngine` | `src/bot/signal.rs`, `strategy/heuristic.rs` | Working |
| `FairValueEngine` (logit jump-diffusion) | `src/bot/strategy/fair_value.rs` | Working |
| `HawkesFlowEngine` (Hawkes/VPIN) | `src/bot/strategy/hawkes_flow.rs` | **Failed** — see SP-08 |
| `BacktestEngine` + `BacktestMetrics` | `src/bot/backtest/replay.rs`, `metrics.rs` | Working (no resolution data) |
| `BeckerParser` (parquet/CSV loader) | `src/bot/backtest/data.rs` | Working |
| `PmxtFetcher` (DuckDB parquet query) | `src/bot/backtest/pmxt.rs` | Working |
| `Observation` struct | `src/bot/strategy/mod.rs` | Missing OBI/VAMP fields |
| `KalmanFilter` | `src/bot/pricing/kalman_filter.rs` | Working |
| Parquet order book snapshots | `pmxtarchives/*.parquet` (18 files) | Available |
| Live session recordings | `recordings/*.csv` (3 files) | Available |

**Key gap**: Neither OBI nor VAMP exist anywhere in the codebase. The `Observation` struct carries `yes_bid/yes_ask/no_bid/no_ask/book_sum` but no multi-level order book depth. The backtester has no resolution labels (settled outcome = Yes/No).

---

## Sub-Problems

### SP-01: Define OBI Computation for Binary Prediction Markets

- **Description**: Design the Order Book Imbalance formula adapted for Polymarket's binary YES/NO structure. Classical OBI = (bid_volume - ask_volume) / (bid_volume + ask_volume) at N price levels. For binary markets, must decide: (a) which depth levels to use (L1-only vs L2/L3), (b) whether to compute on YES side, NO side, or a fused metric, (c) normalization strategy when book_sum varies wildly. The parquet snapshots have `yes_bid/yes_ask/no_bid/no_ask/volume` but depth per level is unclear — need to audit parquet schema for multi-level data.
- **Dependencies**: None (pure design)
- **Priority**: P0
- **Complexity**: Medium

### SP-02: Define VAMP (Volume-Adjusted Mid Price) for Binary Markets

- **Description**: Design VAMP formula for binary markets. Classical VAMP weights each price level by its volume: `VAMP = sum(price_i * volume_i) / sum(volume_i)`. For Polymarket's binary structure, must adapt to: (a) use YES and NO sides jointly, (b) handle the complement constraint (yes_price + no_price ≈ 1.0 minus inefficiency), (c) decide weighting scheme (volume, liquidity depth, or time-weighted). Compare VAMP to simple midpoint (`yes_mid`) to measure whether VAMP provides a better fair price estimate.
- **Dependencies**: SP-01 (shared audit of available depth data)
- **Priority**: P0
- **Complexity**: Medium

### SP-03: Audit Parquet Schema for Order Book Depth Availability

- **Description**: Read actual parquet files from `pmxtarchives/` using DuckDB or `BeckerParser` and determine what columns exist beyond `yes_bid/yes_ask/no_bid/no_ask/volume`. Specifically: are there per-level depth columns (bid_size_1, bid_size_2, ..., ask_size_1, ask_size_2)? Is `book_sum` derived from visible depth? If multi-level depth is unavailable, OBI must be approximated from L1 only (bid/ask spread as proxy). This determines feasibility of SP-01/SP-02.
- **Dependencies**: None
- **Priority**: P0
- **Complexity**: Low

### SP-04: Build OBI Signal Engine Module

- **Description**: Implement `OBIEngine` as a new module in `src/bot/strategy/` or `src/bot/pricing/`. Must: (a) accept order book snapshots (from parquet replay or live feed), (b) compute rolling OBI over configurable window, (c) output normalized imbalance score [-1, +1], (d) detect regime changes (balanced → imbalanced). Should integrate with existing `IndicatorEngine` or be a parallel signal source. Add `obi_score: Option<f64>` to `Observation` struct.
- **Dependencies**: SP-01, SP-03
- **Priority**: P0
- **Complexity**: Medium

### SP-05: Build VAMP Signal Engine Module

- **Description**: Implement `VAMPEngine` as a new module. Must: (a) compute volume-weighted mid price from available depth data, (b) track VAMP vs raw midpoint divergence as a signal, (c) output VAMP-derived fair price estimate. Compare performance: does VAMP converge to settlement outcome faster than `yes_mid`? Add `vamp_price: Option<f64>` to `Observation` struct.
- **Dependencies**: SP-02, SP-03
- **Priority**: P1
- **Complexity**: Medium

### SP-06: Build Mean Reversion Signal Component

- **Description**: Implement mean reversion detection for 5-minute binary markets. The codebase has a `mean_reversion: f64` parameter in `temporal_arbitrage.rs` and `pricing/fair_value.rs` but no dedicated engine. Must: (a) detect when price has moved sharply from a rolling mean (z-score or Bollinger %B based), (b) estimate reversion probability using Ornstein-Uhlenbeck or simple half-life estimation, (c) combine with existing `momentum_slope` and `bb_percent` indicators. The existing `HeuristicEngine` already has reversal logic at BB width >= 0.9 — extend this into a standalone signal.
- **Dependencies**: None (can use existing indicators)
- **Priority**: P1
- **Complexity**: Medium

### SP-07: Extend Observation Struct and Data Pipeline for New Signals

- **Description**: Add new fields to `Observation` in `src/bot/strategy/mod.rs`: `obi_score: Option<f64>`, `vamp_price: Option<f64>`, `mean_reversion_z: Option<f64>`, `spread_imbalance: Option<f64>`. Update all code paths that construct `Observation` (backtest replay, live feed, strategy runner). Update `Default` impl. Ensure backward compatibility with existing `FairValueEngine` and `HeuristicEngine`.
- **Dependencies**: SP-04, SP-05, SP-06
- **Priority**: P0
- **Complexity**: Low

### SP-08: Diagnose and Fix HawkesFlowEngine

- **Description**: The HawkesFlowEngine was marked as failed. Diagnose: likely causes are (a) VPIN estimator requires volume-bucketed trades but `book_sum` is a poor volume proxy, (b) trade direction inference from price movement is noisy on L1 data, (c) Hawkes kernel parameters (alpha=0.3, beta=0.5) not tuned for 5-minute binary markets. Fix by: (i) using actual volume from `volume` field instead of `book_sum`, (ii) adding parameter sweep for kernel decay, (iii) potentially combining Hawkes signal with OBI instead of standalone. May serve as supplementary signal rather than primary strategy.
- **Dependencies**: SP-03 (understanding available data)
- **Priority**: P2
- **Complexity**: High

### SP-09: Implement Backtest Resolution Labels

- **Description**: The backtester (`replay.rs`) has `resolution: Option<bool>` on `MarketData` but it's never populated from real data. Need to either: (a) fetch settlement outcomes from Polymarket API, (b) derive from Chainlink oracle final price vs strike, or (c) use last-minute price as proxy (>0.5 = Yes won). Without labels, cannot compute directional accuracy (53% target). This is blocking meaningful backtest evaluation. The `BeckerParser` needs a `load_labels()` method.
- **Dependencies**: None
- **Priority**: P0
- **Complexity**: Medium

### SP-10: Build OBI/VAMP/MR Fused Strategy Engine

- **Description**: Implement `OBIVAMPEngine` implementing `StrategyEngine` trait. Combines: (a) OBI as directional filter (only trade in direction of imbalance), (b) VAMP divergence as entry trigger (market mispriced vs volume-weighted fair), (c) mean reversion z-score as timing signal (fade extreme moves). Entry requires: OBI > threshold AND VAMP divergence > threshold AND reversion z-score confirms. Exit: TP/SL similar to `HawkesFlowEngine`'s dynamic TP/SL with trailing stop. Must respect existing `RiskGate` and `market_classifier` horizon adaptation.
- **Dependencies**: SP-04, SP-05, SP-06, SP-07
- **Priority**: P0
- **Complexity**: High

### SP-11: Parameter Sweep and Statistical Validation

- **Description**: Run parameter sweep over OBI threshold, VAMP divergence threshold, mean reversion z-score threshold, entry band, TP/SL levels. Use `run_parameter_sweep()` pattern from `replay.rs`. Must backtest on 1000+ contracts across all 18 parquet files + 3 live recordings. Compute: win rate (target 53%+), Sharpe (target >1.0), expectancy (target >$0.005), max drawdown, profit factor. Apply walk-forward validation: train on first 70% of time, test on last 30%. Report confidence intervals (not just point estimates). Bootstrap p-value for Sharpe > 0.
- **Dependencies**: SP-09, SP-10
- **Priority**: P0
- **Complexity**: High

### SP-12: Edge Decay and Regime Detection Analysis

- **Description**: Test whether the OBI/VAMP/MR edge is stationary or decays. Analyze: (a) does win rate decline over time within a session? (b) does the edge exist in all market regimes (trending, mean-reverting, compressed) or only specific ones? (c) does the edge survive transaction costs (2.5% Polymarket fee)? Use the existing `HorizonParams` and `MarketHorizon` classifier to stratify results. If edge is regime-dependent, add regime filter to strategy engine. If edge decays, implement adaptive parameter updates (online learning or rolling calibration).
- **Dependencies**: SP-11
- **Priority**: P1
- **Complexity**: High

---

## Dependency Graph

```
SP-03 (Audit parquet schema)
  ├── SP-01 (Define OBI) ────────── SP-04 (Build OBI engine) ──┐
  └── SP-02 (Define VAMP) ──────── SP-05 (Build VAMP engine) ──┤
                                                                 ├── SP-07 (Extend Observation) ── SP-10 (Fused Engine) ── SP-11 (Sweep) ── SP-12 (Edge Decay)
SP-06 (Mean Reversion engine) ──────────────────────────────────┘
                                                                  SP-09 (Resolution labels) ─────────────────────────────────┘
SP-08 (Fix HawkesFlow) ─── [independent, P2]
```

## Execution Order

1. **Phase 1** (P0, blocking): SP-03 → SP-01, SP-02, SP-09 (parallel)
2. **Phase 2** (P0, build signals): SP-04, SP-05, SP-06 (parallel)
3. **Phase 3** (P0, integrate): SP-07 → SP-10
4. **Phase 4** (P0, validate): SP-11
5. **Phase 5** (P1, refine): SP-12, SP-08

## Risk Notes

- **Data risk**: If parquet files lack multi-level depth, OBI must be approximated from L1 spread/size, which may reduce signal quality. Fallback: use `book_sum` imbalance as OBI proxy.
- **Label risk**: If settlement outcomes cannot be recovered, use last-60-second average price as settlement proxy (price > 0.5 at T-0 = Yes won). Introduces ~1-2% label noise.
- **Overfitting risk**: With only 18 parquet files (~18 hours of data), parameter sweep may overfit. Mitigate with walk-forward and bootstrap validation.
- **Fee impact**: 2.5% Polymarket fee means edge must exceed 2.5% round-trip. At EV > $0.005/contract on a $1 contract, that's 0.5% edge — already below fee threshold. Strategy must either trade at larger size or capture moves > 3% to be net profitable.
