---
created: '2026-03-23T17:46:41+00:00'
evidence:
- stage-02/problem_tree.md
id: problem_decompose-rc-20260323-173553-79dfaa
run_id: rc-20260323-173553-79dfaa
stage: 02-problem_decompose
tags:
- problem_decompose
- stage-02
- run-rc-20260
title: 'Stage 02: Problem Decompose'
---

# Stage 02: Problem Decompose

# Research Decomposition: Polymarket BTC 5-Minute Binary Strategy

---

## Source

**Primary Research Question**: Can order book imbalance signals from Polymarket's BTC binary contracts, combined with a fair-value model derived from spot BTC forward rates, generate statistically significant edge on 5-minute trades?

---

## Sub-questions

### Sub-question 1: Fair Value Estimation for Polymarket BTC Binaries

**What is the implied forward premium/discount embedded in Polymarket BTC probability estimates relative to spot BTC, and does the probability-to-fair-value spread (P − FV) exhibit mean-reversion at 5-minute horizons?**

**Rationale**: This is the foundational measurement question. Without establishing a defensible fair-value benchmark, the P-FV spread cannot be computed, and mean-reversion testing is impossible.

**Key Components to Address**:
- Construct fair-value model from Binance BTC spot + implied forward rates
- Characterize the relationship between Polymarket probability and BTC spot/forward prices
- Measure the distribution of P-FV spreads over time
- Test for unit roots / stationarity of P-FV spread (Engle-Granger, KPSS tests)

**Dependencies**: None (foundational)

**Feasibility**: High — requires Binance L2 data and Polymarket historical fills; both accessible via API

---

### Sub-question 2: Polymarket Order Book Imbalance Signal (LOBI-PM) and Its Predictive Content

**Does Polymarket's limit order book imbalance (LOBI-PM) predict 5-minute probability moves in BTC up/down contracts, and what is the optimal LOBI specification (window, depth level, normalization) for maximizing predictive power?**

**Rationale**: The LOBI-PM is the core microstructure signal. This question establishes whether the prediction market's own order flow contains alpha before the book signals merge into prices.

**Key Components to Address**:
- Define LOBI-PM: `(bid_vol - ask_vol) / (bid_vol + ask_vol)` at N levels (N = 1, 3, 5, 10)
- Test predictive regressions: `Δprobability(t+1:t+k) = f(LOBI-PM(t), controls)`
- Cross-validate optimal LOBI parameters using temporal split
- Characterize LOBI-PM dynamics during high-frequency events (news, whale activity)

**Dependencies**: None (can run in parallel with SQ1)

**Feasibility**: Medium — requires Polymarket order book snapshot data (less documented than trade/OHLCV)

---

### Sub-question 3: Signal Synergy — Does LOBI-PM + P-FV Provide Non-Additive Alpha?

**Is the combined LOBI-PM + P-FV spread signal statistically superior to either component alone, and what is the optimal weighting/ensemble methodology?**

**Rationale**: The research novelty claims "joint > sum of parts." This question directly tests that assertion via ablation study and interaction term analysis.

**Key Components to Address**:
- Construct joint signal: `α = w₁·LOBI-PM + w₂·P-FV + w₃·(LOBI-PM × P-FV)`
- Test for superadditivity: `IC(joint) > IC(LOBI-PM) + IC(P-FV)`
- Compare predictive regressions with/without interaction terms
- Evaluate ensemble methods: linear combination vs. gradient-boosted signal stacking
- Measure Information Coefficient (IC) decay over prediction horizon (1–12 bins)

**Dependencies**: SQ1 and SQ2 (requires both signals to be constructed)

**Feasibility**: Medium — requires integrated dataset from SQ1 and SQ2

---

### Sub-question 4: Execution Feasibility and Slippage-Adjusted Edge

**What are the realistic gross and net Sharpe ratios after accounting for Polymarket fee structure, maker/taker spreads, and estimated market impact on 5-minute binary contracts?**

**Rationale**: A signal with statistical edge may still be untradeable after costs. This question ensures the strategy is evaluated under realistic execution assumptions before claiming "exploitable."

**Key Components to Address**:
- Model Polymarket fee structure (0% maker, 1% taker on winnings — verify current structure)
- Estimate effective bid-ask spread from Polymarket LOB at signal time
- Construct slippage model: market orders vs. limit orders; partial fill assumptions
- Backtest with execution costs: compute gross vs. net Sharpe
- Identify breakeven liquidity threshold (minimum open interest for viable execution)

**Dependencies**: SQ3 (requires signal to exist before testing execution)

**Feasibility**: High — requires Polymarket fee schedule and order book depth data

---

### Sub-question 5: Regime Dependency and Out-of-Sample Stability

**How do the combined signal's performance metrics vary across market regimes (high/low volatility, high/low volume, bull/bear BTC markets), and does the strategy retain ≥80% of Sharpe on the out-of-sample holdout period?**

**Rationale**: Publishable strategies must be robust across regimes. This question addresses the "Out-of-Sample Robustness" success criterion from the proposal.

**Key Components to Address**:
- Partition data into regimes using BTC realized vol (VIX-equivalent for crypto) and Polymarket volume
- Measure Sharpe/IC by regime; test for statistically 

... (truncated, see full artifact)
