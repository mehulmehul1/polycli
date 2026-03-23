---
created: '2026-03-23T17:16:46+00:00'
evidence:
- stage-02/problem_tree.md
id: problem_decompose-rc-20260323-170843-79dfaa
run_id: rc-20260323-170843-79dfaa
stage: 02-problem_decompose
tags:
- problem_decompose
- stage-02
- run-rc-20260
title: 'Stage 02: Problem Decompose'
---

# Stage 02: Problem Decompose

```markdown
# Research Problem Decomposition
## Polymarket BTC 5-Minute Binary Prediction Market Strategy

---

## Source

**Primary Research Question** (from goal document):
> *Can order book microstructure signals, when combined with fair-value estimation derived from Bitcoin spot/futures basis and mean-reversion dynamics, produce statistically significant alpha in Polymarket's BTC 5-minute up/down contracts?*

**Rationale for Decomposition**:
The primary question contains three distinct components (OBI, fair value, mean reversion) that require independent validation before combination. Additionally, statistical validity and practical constraints impose boundary conditions that must be addressed as separate sub-questions. Decomposition enables clear hypothesis formation, appropriate statistical testing sequences, and risk identification at each stage.

---

## Sub-questions

### SQ1: Order Book Imbalance as a Directional Predictor

**Question**: *At what order book depth (top 1, 3, 5, 10 levels) does OBI exhibit statistically significant predictive power for BTC 5-minute contract direction on Polymarket, and what is the optimal lookback window for computing imbalance?*

**Decomposition Rationale**: OBI is the most well-studied microstructure signal in the literature (Cont et al. 2014, Cao et al. 2009), making it the natural starting point. However, optimal depth and window parameters are platform-specific and must be empirically determined before any combination work.

**Operational Definitions**:
- OBI = (Bid Volume - Ask Volume) / (Bid Volume + Ask Volume)
- Depth levels: L1, L3, L5, L10 price levels from mid-price
- Lookback windows: 30s, 1min, 2min, 5min rolling
- Prediction target: Binary contract direction at 5-minute expiration

**Success Criteria**: OBI-only signal achieves Sharpe > 0.2, win rate > 51%, with bootstrap 95% CI excluding zero.

**Dependencies**: None (foundational signal).

---

### SQ2: Fair Value Estimation from Bitcoin Spot/Futures Basis

**Question**: *Can the implied fair probability of a Polymarket BTC 5-minute contract be estimated from Bitcoin spot returns, futures basis, and ETF flows, and does deviation of market price from this fair value predict mean reversion at the 5-minute horizon?*

**Decomposition Rationale**: Fair value estimation creates a theoretical anchor for contract pricing. The question tests whether Polymarket's market price systematically deviates from rational pricing derived from the underlying Bitcoin market, and whether these deviations are exploitable.

**Operational Definitions**:
- Fair value proxy: 5-minute rolling BTC-USD spot return direction and magnitude
- Premium/discount: Market price (PM) vs. fair value estimate (FV)
- Hypothesis: |PM - FV| predicts mean reversion toward FV within the 5-minute window

**Success Criteria**: Fair value signal alone achieves Sharpe > 0.15, with statistically significant (p < 0.05) predictive accuracy exceeding 50%.

**Dependencies**: None (independent signal stream).

---

### SQ3: Mean Reversion in Noisy Probability Markets

**Question**: *At what timeframes (30s, 1min, 2min, 5min pre-expiration) does probability mean reversion occur in Polymarket BTC contracts, and what is the optimal threshold for "extreme" probability that triggers reversion signals?*

**Decomposition Rationale**: Mean reversion is theoretically distinct from both OBI and fair value signals. It captures market microstructure noise—temporary imbalances that self-correct before expiration. This question is novel given the 2025 Chen & Li paper framework applied to crypto-native markets.

**Operational Definitions**:
- Mean reversion trigger: Contract probability moves > X% from 5-period moving average
- Threshold sweep: X ∈ {2%, 5%, 10%, 15%, 20%}
- Prediction: Probability reverts toward baseline within remaining window

**Success Criteria**: Mean reversion signal achieves Sharpe > 0.1, with > 55% of extreme probability events reverting within 2 minutes.

**Dependencies**: None (independent signal stream, but relies on Polymarket's probability time series).

---

### SQ4: Multi-Signal Combination and Alpha Generation

**Question**: *Does combining OBI, fair value deviation, and mean reversion signals via linear weighting, logistic regression, or gradient boosting produce statistically significant alpha that outperforms any single signal alone, and does this combination survive out-of-sample validation and transaction cost sensitivity analysis?*

**Decomposition Rationale**: This is the core research contribution. The question tests whether signal synergy exists—whether the combined model captures non-linear interactions that individual signals miss. It also directly addresses the publishability requirement for multi-signal fusion methodology.

**Operational Definitions**:
- Combination methods: Equal weighting, optimized weighting (grid search), logistic regression, XGBoost
- Alpha definition: Sharpe ratio exceeding best single-signal benchmark

... (truncated, see full artifact)
