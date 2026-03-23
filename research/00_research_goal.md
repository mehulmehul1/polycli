# Research Goal: Exploitable Short-Term Edge in Polymarket 5-Minute BTC Binary Markets

## 1. Topic

Identify and quantify a statistically robust, implementable trading edge in Polymarket's 5-minute Bitcoin price prediction contracts (BTC up/down binary) by combining three signal families that have individually shown academic promise but have **not been jointly exploited** in binary prediction market microstructure:

1. **Order Book Imbalance (OBI)** — the ratio of bid-to-ask depth as a predictor of next-tick direction
2. **Volume-Adjusted Mid Price (VAMP)** — a fair value estimate superior to naive mid-price, used to detect when the market's implied probability deviates from "true" probability
3. **Mean Reversion in Noisy Probability Regimes** — identification of conditions where 5-minute binary token prices overshoot and revert within the contract window

The research produces a **concrete strategy specification** with entry/exit rules, expected edge size, and a backtesting methodology — not a theoretical survey.

---

## 2. Novel Angle

### What's already been tried (and failed)
- **HawkesFlow**: Modeled order flow excitation (self-exciting point processes on trade arrivals). Result: no edge. The Hawkes model assumes information cascades matter; in 5-minute binary markets on BTC, the window is too short and noise too dominant for excitation dynamics to produce predictive signal.

### What's unexplored
No existing work combines the following three insights in a **binary prediction market** context:

| Insight | Source | Status in Binary Markets |
|---|---|---|
| OBI explains ~65% of short-term price variance | Cont et al., academic microstructure lit | **Untested** on Polymarket order books |
| VAMP > mid-price for fair value estimation | Hasbrouck (2002), extended by Debnath et al. | **Never applied** to prediction token pricing |
| Taker toxicity averages -1.12% (adverse selection) | Becker 2026, 72M Kalshi trades | **Not exploited** as a contrarian signal on Polymarket |
| Mean reversion dominates in low-volume regimes | BTC spot market confirmed | **Not characterized** for 5-min binary contracts specifically |

The novel contribution is **the fusion**: use VAMP-derived fair value to detect mispricing, use OBI to confirm directional pressure, and use regime detection (volume/volatility state) to determine whether to trade momentum or mean-reversion. Prior work treats these as separate signals. We treat them as a **joint filter stack**.

Specific novel questions:
- Does OBI on Polymarket's CLOB (central limit order book) carry the same predictive power as on equity exchanges, given that prediction market participants are structurally different (retail-heavy, event-driven)?
- Can VAMP be computed from Polymarket's API-obtainable book data, and does the VAMP-implied probability diverge systematically from traded price in exploitable ways?
- Does the -1.12% taker penalty (Becker) imply that **maker-side strategies** have a structural edge, and can this be combined with OBI/VAMP signals to define a market-making-with-direction strategy?

---

## 3. Scope

### In Scope
- **Market**: Polymarket BTC up/down 5-minute binary contracts only
- **Data sources**:
  - Polymarket CLOB API (order book snapshots, trade tape, historical settled contracts)
  - BTC spot reference price (used by Polymarket for settlement — Coinbase, Binance, or specified oracle)
- **Signal families**: OBI, VAMP, mean reversion regime detection, taker toxicity
- **Outputs**:
  - Statistical characterization of each signal's predictive power (R², directional accuracy, information coefficient)
  - A combined strategy with explicit entry/exit/filter rules
  - Backtested P&L with realistic assumptions (fees, slippage, fill probability on maker orders)
  - Minimum sample size and power analysis for edge detection
- **Time horizon**: Per-contract holding period of 0–5 minutes (intradac, no overnight)
- **Fee model**: Polymarket's maker/taker fee schedule applied; Becker's -1.12% taker cost factored in as baseline adverse selection cost

### Out of Scope
- Other Polymarket contracts (non-BTC, non-5-min, non-binary)
- Other prediction markets (Kalshi, Manifold, Metaculus)
- Machine learning / neural net approaches (the goal is interpretable, explainable edge — if ML is needed later, it's Phase 2)
- Smart contract risk, platform risk, regulatory analysis
- High-frequency strategies requiring co-location or sub-second latency
- Cross-asset signals (e.g., using ETH order book to predict BTC binary)

---

## 4. SMART Goal

### Specific
Produce a document titled **"Joint OBI-VAMP Mean Reversion Strategy for Polymarket 5-Min BTC Binaries"** containing:
1. A data collection pipeline specification (API endpoints, polling frequency, storage schema)
2. Per-signal statistical tests with defined null hypotheses (e.g., "OBI has no predictive power for 5-min BTC direction on Polymarket" → reject/fail-to-reject with p-value)
3. A combined strategy with machine-readable entry/exit rules (e.g., "Enter SHORT when: VAMP-implied prob < traded prob by > 2σ AND OBI < 0.4 AND regime = low-volume")
4. Backtest results on ≥ 1,000 historical contracts with Sharpe, max drawdown, win rate, and expected value per trade
5. A minimum viable edge threshold: "Strategy is deployable if EV per trade > fees + 0.5% after slippage"

### Measurable
| Metric | Target | Why |
|---|---|---|
| Historical contracts analyzed | ≥ 1,000 | Statistical power for 55%+ directional accuracy detection |
| Directional accuracy | ≥ 53% | Breakeven after fees on binary (fee-adjusted); 55% = comfortable edge |
| EV per trade (net of fees) | > $0.005 per $1 contract | Minimum to overcome gas/withdrawal friction |
| Sharpe ratio (annualized, 5-min freq) | > 1.0 | Standard quant threshold for "real" edge |
| Max drawdown | < 20% of bankroll | Risk management constraint |
| Signal IC (information coefficient) | > 0.03 per individual signal | Academic threshold for "useful" short-horizon signal |

### Achievable
- **Data is available**: Polymarket CLOB API provides order book depth, trade history, and historical contract outcomes. BTC spot data is ubiquitous.
- **Computation is trivial**: No ML training, no GPU. Strategy runs on a single machine with < 100ms computation per tick.
- **Academic precedent exists**: OBI's 65% explanatory power (Cont), VAMP's superiority (Hasbrouck), and taker toxicity (Becker) are all published and replicated. We are **combining** known signals in a new market, not inventing new theory.
- **Failed attempt provides calibration**: HawkesFlow's failure tells us that flow-based signals don't work here. This narrows the search space and validates focusing on book-level signals instead.

### Relevant
- Directly addresses the user's goal: find an exploitable edge in Polymarket 5-min BTC binaries
- Builds on and explains why HawkesFlow failed (wrong signal family for this market structure)
- Produces actionable output (strategy rules), not just analysis
- Aligns with the known microstructure literature — if OBI explains 65% of variance in traditional markets, the question is whether it transfers to prediction markets, which is a **testable, high-value question**

### Time-Bound
| Phase | Duration | Deliverable |
|---|---|---|
| Phase 1: Data Collection & Validation | Days 1–3 | Verified dataset of ≥ 2,000 historical 5-min BTC contracts with full order book snapshots |
| Phase 2: Individual Signal Characterization | Days 4–7 | Statistical tests for OBI, VAMP, and mean reversion individually — reject/fail-to-reject each null hypothesis |
| Phase 3: Combined Strategy Design | Days 8–10 | Strategy specification with entry/exit rules, filter logic, and parameter sensitivity analysis |
| Phase 4: Backtest & Validation | Days 11–14 | Backtest on held-out data, walk-forward analysis, Sharpe/drawdown/edge confirmation |
| Phase 5: Write-Up | Day 15 | Final document with all results, strategy spec, and deployment recommendation |

**Total: 15 days from research start to final deliverable.**

---

## Appendix: Key Assumptions & Risks

### Assumptions
- Polymarket's CLOB is sufficiently liquid for 5-min BTC contracts to allow ≥ 10 trades/day at reasonable size (< $500 per trade)
- Order book depth data is obtainable via API at ≥ 1-second frequency (ideally 100ms)
- The BTC reference price used for settlement is observable in real-time

### Risks
| Risk | Mitigation |
|---|---|
| Polymarket book too thin for OBI calculation | Test with top-5, top-10, full-book depth; if thin, OBI signal may be noisier — adjust confidence thresholds |
| Survivorship bias in historical data | Use only contracts with complete lifecycle data; verify no missing settlements |
| Overfitting to historical regime | Walk-forward validation with expanding window; out-of-sample holdout of final 20% of contracts |
| Edge exists but is too small to overcome fees | Explicit fee modeling in backtest; strategy rejected if net EV < threshold |
| Mean reversion signal confounded by BTC trending | Regime filter: only trade mean-reversion signal when realized vol is in bottom 40% of distribution |
