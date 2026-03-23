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
- Alpha definition: Sharpe ratio exceeding best single-signal benchmark by > 0.2 points
- Out-of-sample: Last 2 weeks of data held completely (rolling 2-week validation)

**Success Criteria**: 
- Combined signal Sharpe > 0.8 (primary target)
- Combined signal Sharpe > (best single signal + 0.2) (synergy requirement)
- White's reality check p-value < 0.05 (data-snooping correction)
- Profitability maintained at 0%, 0.5%, 1%, 2% fee scenarios

**Dependencies**: SQ1, SQ2, SQ3 (requires validated individual signals).

---

### SQ5: Execution Robustness and Practical Implementation

**Question**: *How sensitive is the combined strategy to execution assumptions (fill slippage, latency, order type selection), and does the strategy exhibit regime stability across different Bitcoin volatility environments?*

**Decomposition Rationale**: Academic validity is necessary but insufficient for practical contribution. This question addresses the gap between backtest performance and live trading reality, and tests non-stationarity concerns raised in the goal document.

**Operational Definitions**:
- Slippage scenarios: 0bps, 5bps, 10bps, 25bps away from mid-price
- Latency assumptions: 100ms, 500ms, 1s execution delay
- Regime classification: Low volatility (< 1% hourly), medium (1-3%), high (> 3%)
- Stability metric: Sharpe ratio consistency across regime subsamples

**Success Criteria**:
- Strategy remains profitable (Sharpe > 0.5) at 10bps slippage
- Sharpe variation < 30% across volatility regimes
- Strategy exhibits no degradation over time (rolling 2-week performance)

**Dependencies**: SQ4 (requires combined signal).

---

## Priority Ranking

| Rank | Sub-Question | Priority Score | Rationale |
|------|--------------|----------------|-----------|
| **1** | SQ1: OBI Signal Validation | **HIGH** | Foundational; validates primary microstructure signal; establishes baseline effect size; no dependencies |
| **2** | SQ2: Fair Value Estimation | **HIGH** | Independent signal; establishes theoretical anchor; no dependencies |
| **3** | SQ3: Mean Reversion Detection | **MEDIUM-HIGH** | Novel contribution; requires separate validation; no dependencies |
| **4** | SQ4: Multi-Signal Combination | **HIGH** | Core research contribution; blocked by SQ1-SQ3 completion |
| **5** | SQ5: Execution Robustness | **MEDIUM** | Practical validation; blocked by SQ4; addressed after primary results |

### Sequencing Logic

```
Week 1-2: SQ1 (OBI validation)
Week 2-3: SQ2 (Fair value estimation)
Week 3-4: SQ3 (Mean reversion detection)
Week 4-6: SQ4 (Signal combination + in-sample)
Week 6-8: SQ4 (Out-of-sample validation)
Week 8-10: SQ5 (Robustness testing)
Week 10-12: Integration + paper writing
```

**Critical Path**: SQ1 → SQ2 → SQ3 → SQ4 → SQ5

---

## Risks

### Statistical Risks

| Risk | Probability | Impact | Mitigation |
|------|-------------|--------|------------|
| **Data snooping bias** | HIGH | HIGH | White's reality check; holdout validation; limited feature combinations; pre-registered hypotheses |
| **Small sample size** | MEDIUM | MEDIUM | ~25,000 contract observations (90 days × 288 windows); bootstrap confidence intervals; avoid overfitting |
| **Low statistical power** | MEDIUM | MEDIUM | Target Sharpe > 0.8 requires detectable effect size; if null result, document power analysis |

### Market Microstructure Risks

| Risk | Probability | Impact | Mitigation |
|------|-------------|--------|------------|
| **Non-stationarity** | HIGH | HIGH | Rolling window validation; regime subsample analysis; results disaggregated by volatility regime |
| **Adverse selection** | MEDIUM | MEDIUM | OBI signals may be anti-predictive if sophisticated traders front-run; test separately on high-volume vs. low-volume contracts |
| **Order book manipulation** | LOW | HIGH | Exclude periods with anomalous order book activity; implement volume filters |

### Execution Risks

| Risk | Probability | Impact | Mitigation |
|------|-------------|--------|------------|
| **Fill slippage** | HIGH | MEDIUM | Explicit slippage sensitivity analysis (SQ5); conservative assumptions in primary reporting |
| **Latency** | MEDIUM | MEDIUM | Test 100ms, 500ms, 1s scenarios; strategies designed for slower execution may exist |
| **Fee structure changes** | LOW | MEDIUM | Report profitability at 0%, 0.5%, 1%, 2% explicitly |

### Platform Risks

| Risk | Probability | Impact | Mitigation |
|------|-------------|--------|------------|
| **API access restriction** | LOW | HIGH | Document data collection methodology; replicate from on-chain data if needed |
| **Contract structure change** | LOW | HIGH | Focus on signal framework rather than specific parameters; emphasize extensibility |
| **Liquidity withdrawal** | MEDIUM | MEDIUM | Test strategy only on periods with sufficient volume; define minimum volume thresholds |

### Research Design Risks

| Risk | Probability | Impact | Mitigation |
|------|-------------|--------|------------|
| **Confirmation bias** | MEDIUM | HIGH | Pre-register success criteria; require negative result documentation if thresholds unmet |
| **Publication bias** | MEDIUM | MEDIUM | Explicit null result section; honest reporting of what was tried |
| **Overfitting to in-sample** | HIGH | HIGH | Strict holdout (2 weeks); no parameter refinement on holdout data |

---

## Summary Matrix

| Sub-Question | Dependencies | Priority | Primary Metric | Threshold |
|--------------|--------------|----------|----------------|-----------|
| SQ1: OBI Validation | None | 1 | Sharpe | > 0.2 |
| SQ2: Fair Value | None | 2 | Sharpe | > 0.15 |
| SQ3: Mean Reversion | None | 3 | Sharpe | > 0.1 |
| SQ4: Combination | SQ1, SQ2, SQ3 | 4 | Sharpe | > 0.8 |
| SQ5: Robustness | SQ4 | 5 | Sharpe @ 10bps | > 0.5 |

---

## Research Workflow Dependencies

```
[START]
   │
   ├─► [SQ1: OBI Validation] ──────────────────────────┐
   │          │                                        │
   ├─► [SQ2: Fair Value] ───────────────────────────────┤
   │          │                                        │
   └─► [SQ3: Mean Reversion] ──────────────────────────┤
                      │                                 │
                      ▼                                 │
            [SQ4: Signal Combination] ───────────────────┤
                      │                                 │
                      ▼                                 │
            [SQ5: Execution Robustness] ────────────────┤
                      │                                 │
                      ▼                                 │
                   [FINAL]                             │
                                                          │
[GOAL: Publishable paper with actionable strategy] ──────┘
```