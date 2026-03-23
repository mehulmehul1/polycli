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
- Measure Sharpe/IC by regime; test for statistically significant differences
- Run temporal holdout: train ≤ 2025-03-31, test ≥ 2025-04-01
- Compare last-6-months performance vs. full test set (≥80% threshold)
- Test for structural breaks (Chow test) in signal efficacy over time

**Dependencies**: SQ3 (requires backtestable strategy)

**Feasibility**: Medium — requires sufficient data span for regime partitioning

---

## Priority Ranking

| Rank | Sub-question | Priority Rationale | Dependency Chain Position |
|------|--------------|-------------------|---------------------------|
| **1** | SQ1: Fair Value & P-FV Spread | Foundation for all P-FV analysis; no strategy without benchmark | Must complete first |
| **1** | SQ2: LOBI-PM Signal | Independent microstructure signal; establishes order-flow alpha | Can run in parallel with SQ1 |
| **2** | SQ3: Signal Synergy | Tests core research novelty ("joint > sum of parts"); required for ablation study | Requires SQ1 + SQ2 |
| **3** | SQ4: Execution Feasibility | Validates "exploitable" claim; disqualifies signals that are statistically but not economically significant | Requires SQ3 |
| **4** | SQ5: Regime Robustness | Ensures publishability (out-of-sample stability criterion); addresses reviewer skepticism | Requires SQ3 |

**Rationale for Parallel P1**: SQ1 and SQ2 are methodologically independent — SQ1 constructs the fair-value model from spot/forward BTC data, while SQ2 extracts order-flow signals from Polymarket's LOB. Running them in parallel accelerates timeline without risk of dependency conflict.

---

## Risks

| Sub-question | Risk Category | Risk Description | Likelihood | Mitigation |
|--------------|---------------|------------------|------------|------------|
| **SQ1** | **Data Quality** | Polymarket's API may not expose historical order book snapshots; only trade fills may be available | Medium | Use Kaiko/CCData third-party scraper (budgeted $200/mo); fall back to reconstructed book from trade flow if LOB unavailable |
| **SQ1** | **Model Risk** | Forward rate model may not capture Polymarket's unique settlement dynamics (USDBTC rate at expiry vs. mid) | Medium | Validate with Polymarket settlement API; test sensitivity to forward rate assumption |
| **SQ2** | **Data Availability** | Polymarket LOB depth at 100ms granularity may exceed API rate limits or be rate-limited | High | Use websocket streaming with local buffer; limit to top-10 levels if full book unavailable |
| **SQ2** | **Signal Decay** | LOBI-PM may exhibit rapid decay due to HFT activity; 5-min horizon may be too slow | Medium | Test LOBI at 1-min and 2-min windows; compare IC across horizons |
| **SQ3** | **Overfitting** | Interaction terms and stacking may overfit to training data; joint signal may not generalize | Medium | Enforce strict temporal split; use nested walk-forward validation |
| **SQ3** | **Correlation Instability** | LOBI-PM and P-FV may be correlated non-linearly in ways that break during regime shifts | Low-Medium | Test DCC-GARCH for time-varying correlation; include regime dummy variables |
| **SQ4** | **Liquidity Risk** | Polymarket BTC contracts may lack sufficient OI for reliable execution during test period | High | Filter contracts with ≥$10K OI only; exclude thin contracts from backtest universe |
| **SQ4** | **Fee Structure Change** | Polymarket may modify fee model before paper publication | Low | Document current fee structure explicitly; note in limitations |
| **SQ5** | **Insufficient Data for Regime Partition** | 16 months of data may not contain enough regime cycles for stable Sharpe-by-regime estimates | Medium | Use overlapping regimes (vol × volume grid) rather than discrete regimes; bootstrap confidence intervals |
| **SQ5** | **Temporal Non-Stationarity** | Signal efficacy may drift due to market structure changes (more HFT participants, etc.) | Medium | Add time-trend controls; test for structural breaks; acknowledge in limitations |

---

## Summary Table

| Sub-question | Research Focus | Priority | Key Deliverable | Primary Risk |
|--------------|----------------|----------|-----------------|--------------|
| **SQ1** | Fair value model & P-FV mean reversion | 1 (parallel) | Stationary P-FV spread series | Data quality / model risk |
| **SQ2** | Polymarket LOBI signal design | 1 (parallel) | Optimized LOBI-PM specification | LOB data availability |
| **SQ3** | Combined signal synergy | 2 | Ablation-confirmed superadditivity | Overfitting |
| **SQ4** | Execution feasibility | 3 | Net Sharpe after slippage | Liquidity risk |
| **SQ5** | Regime robustness | 4 | Out-of-sample Sharpe ≥80% threshold | Insufficient regime data |