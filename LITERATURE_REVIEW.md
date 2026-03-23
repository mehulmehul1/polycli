# Literature Review: Exploitable Edge in Polymarket 5-Minute BTC Binary Markets Using OBI, VAMP, and Mean Reversion Signals

**Date:** March 2026  
**Research Stage:** Stage 3 — Literature Review

---

## Abstract

This review synthesizes academic and practitioner research across five domains relevant to extracting alpha from Polymarket's 5-minute BTC binary markets: (1) Order Book Imbalance (OBI) as a predictive signal, (2) Volume-Adjusted Mid Price (VAMP) estimation, (3) mean reversion in crypto and binary markets, (4) prediction market microstructure, and (5) Polymarket/Kalshi-specific order flow and fee dynamics. We identify 12 key references spanning market microstructure theory, empirical high-frequency trading research, and emerging prediction market analysis.

---

## 1. Order Book Imbalance (OBI) — Queue Imbalance as a Predictive Signal

### Ref 1: Cont, Kukanov & Stoikov (2014) — "The Price Impact of Order Book Events"

- **Authors:** Rama Cont, Arseniy Kukanov, Sasha Stoikov
- **Year:** 2014
- **Journal:** *Journal of Financial Econometrics*, 12(1), 47–88
- **Key Finding:** Over short time intervals, price changes are mainly driven by order flow imbalance (OFI) — the imbalance between supply and demand at the best bid and ask. A linear relationship between OFI and price changes was documented with R² ≈ 65% on average, with a slope inversely proportional to market depth. This relationship is robust across intraday seasonality and across 50 U.S. stocks.
- **Relevance:** This is the foundational paper for OBI-based trading signals. The linear OFI–price change model is directly transferable to binary markets: if OFI predicts mid-price direction in continuous LOB markets, it should predict directional pressure on the YES/NO price in Polymarket's CLOB. The high R² at short horizons (<1 min) is especially relevant for 5-minute binary contracts.
- **Source:** https://doi.org/10.1093/jjfinec/nbt003

### Ref 2: Gould & Bonart (2015) — "Queue Imbalance as a One-Tick-Ahead Price Predictor in a Limit Order Book"

- **Authors:** Martin D. Gould, Julius Bonart
- **Year:** 2015
- **Published:** arXiv:1512.03492; also at SSRN
- **Key Finding:** Queue imbalance at the best bid/ask provides strongly statistically significant predictive power for the direction of the next mid-price movement. Logistic regression fits on 10 Nasdaq stocks showed considerable improvement over null models for both binary and probabilistic classification, especially for large-tick stocks.
- **Relevance:** Polymarket's BTC markets are effectively large-tick (prices in 1-cent increments from $0.01–$0.99 with discrete tick sizes). Gould & Bonart's finding that OBI works *best* for large-tick stocks is directly encouraging: Polymarket's tick structure may amplify rather than degrade OBI signal quality. The one-tick-ahead prediction maps naturally to predicting the next price move in a binary contract.
- **Source:** https://arxiv.org/abs/1512.03492

### Ref 3: Cartea, Donnelly & Jaimungal (2018) — "Enhancing Trading Strategies with Order Book Signals"

- **Authors:** Álvaro Cartea, Ryan F. Donnelly, Sebastian Jaimungal
- **Year:** 2018 (working paper 2015)
- **Journal:** *Applied Mathematical Finance*, 25(1), 1–35
- **Key Finding:** Volume imbalance in the LOB is a strong predictor of both the sign of the next market order and price changes immediately after. Incorporating volume imbalance into a stochastic optimal execution strategy "considerably boosts profits" by reducing adverse selection costs and positioning limit orders to exploit favorable price movements. Out-of-sample tests on 11 Nasdaq equities confirmed profitability.
- **Relevance:** Demonstrates that OBI signals are not merely statistical curiosities — they translate into P&L when embedded in a proper execution framework. For Polymarket, this suggests that an OBI signal can be used not just for directional bets but for optimal order placement (maker vs. taker decisions), which is critical given Polymarket's near-zero maker fees and positive maker rebates.
- **Source:** https://ssrn.com/abstract=2668277

### Ref 4: Kolm, Ritter & Simunovic (2023) — "Deep Order Flow Imbalance: Extracting Alpha at Multiple Horizons from the Limit Order Book"

- **Authors:** Petter N. Kolm, Gordon Ritter, Simunovic
- **Year:** 2023
- **Key Finding:** Deep learning models trained on multi-level, multi-horizon OFI features significantly outperform models trained directly on raw order book states or returns. OFI features transformed from the LOB are stationary and provide state-of-the-art predictive accuracy for high-frequency returns across 115 Nasdaq stocks.
- **Relevance:** Extends single-level OBI to richer representations. For a 5-minute binary market, multi-level OFI (integrating depth beyond the best bid/ask) may capture information that single-level OBI misses, particularly when liquidity in Polymarket BTC markets is concentrated at a few price levels. The stationarity finding is critical: OFI features are more stable than raw price/volume inputs, making them suitable for live trading systems.
- **Source:** https://www.researchgate.net/publication/372568099

### Ref 5: arXiv (2025) — "Order Book Filtration and Directional Signal Extraction"

- **Authors:** [Not specified in search results]
- **Year:** 2025
- **Key Finding:** Structural filtration of LOB event streams (by order lifetime, update count, or inter-update delay) improves OBI directional signal clarity in correlation and regime-based metrics. However, OBI computed using *trade events only* exhibits stronger causal alignment with future price movements than OBI from filtered order events.
- **Relevance:** Suggests that naively computing OBI from all order book events (placements, cancellations, modifications) introduces noise. For Polymarket — where the CLOB receives frequent quote updates — filtering to trade-only events or applying lifetime-based filtration could dramatically improve signal-to-noise ratio. This is especially relevant for the 5-minute horizon where transient noise can dominate.
- **Source:** https://arxiv.org/html/2507.22712v1

---

## 2. Volume-Adjusted Mid Price (VAMP)

### Ref 6: Hasbrouck (2002) — "Stalking the 'Efficient Price' in Market Microstructure Specifications"

- **Authors:** Joel Hasbrouck
- **Year:** 2002
- **Journal:** *Journal of Financial Markets*, 5(3), 329–339
- **Key Finding:** The observed transaction price decomposes into an unobservable efficient price (a martingale reflecting fundamental value) plus a transient microstructure noise component (bid-ask bounce, discreteness, liquidity effects). The mid-quote is a better proxy for the efficient price than the last trade price, but standard mid still ignores volume information.
- **Relevance:** Establishes the theoretical foundation for VAMP. In Polymarket's binary contracts, the "efficient price" is the market's true probability estimate. The quoted mid ($0.52 bid / $0.54 ask → mid = $0.53) may be a noisy estimator. VAMP adjusts this by weighting toward the side with more volume, producing a better estimate of the fair probability — directly useful for detecting when the market price deviates from its true value.
- **Source:** https://ideas.repec.org/a/eee/finmar/v5y2002i3p329-339.html

### Ref 7: Gabbireddy (2023) — "Volume-Adjusted Mid Price and Order Book Imbalance" [PSU Thesis]

- **Authors:** Gabbireddy (Penn State Honors Thesis)
- **Year:** 2023
- **Key Finding:** Using Coinbase order book data, the volume-adjusted mid price (VAMP) outperforms both the weighted mid (quote imbalance) and trade-imbalance-based mid forecasts in predicting short-term price changes. VAMP achieves the highest trading profit in backtests compared to other midpoint estimation methods. A dynamic regression model that accounts for nonlinearity between OBI and forward returns further improves on static VAMP.
- **Relevance:** Directly validates VAMP in a crypto exchange context (Coinbase), which shares structural similarities with Polymarket's order book (electronic, continuous double auction). The finding that VAMP beats simple mid-price in crypto markets is directly applicable. The thesis also shows that precomputing regression coefficients for various OBI regimes is computationally feasible for real-time trading.
- **Source:** https://honors.libraries.psu.edu/files/final_submissions/9619

---

## 3. Mean Reversion in Crypto and Binary Markets

### Ref 8: Prevayo (2026) — "Mean Reversion Trading in Prediction Markets: Statistical Edge"

- **Author:** Prevayo Research
- **Year:** 2026
- **Key Finding:** Prediction markets exhibit more pronounced mean reversion than traditional markets due to emotional betting patterns and limited liquidity. Moves exceeding 2 standard deviations from the recent mean show 68% reversion rates in sports markets and 58% in political markets. Crypto-related prediction markets show lower (41%) reversion rates due to inherent volatility. A modified Kelly Criterion incorporating time decay (f = (bp - q) / b × √(T/t)) provides optimal position sizing for mean reversion in time-limited contracts.
- **Relevance:** Directly addresses mean reversion in prediction markets. The 41% reversion rate in crypto-related prediction markets is lower than other categories but still provides a statistical edge if properly filtered. The modified Kelly formula with time decay is essential for 5-minute binary markets where the contract expiration creates natural time pressure. The finding that emotional overreaction creates systematic mispricing is the core thesis behind mean reversion in BTC binary markets.
- **Source:** https://www.prevayo.com/blog/mean-reversion-trading-prediction-markets-statistical-edge

### Ref 9: QuantifiedStrategies (2026) — "Bitcoin Mean Reversion Strategies Outperform Momentum in Low Volume Regimes"

- **Author:** Quantified Trading
- **Year:** 2026
- **Key Finding:** Mean reversion strategies outperform trend-following/momentum strategies during low-volume consolidation periods in Bitcoin. The Hurst exponent for BTC (2021–2024) measured 0.52, indicating mild trending — but during volume-depressed periods, the exponent drops below 0.5, confirming mean-reverting behavior. Bollinger Band-based entry at 2–3σ with volume oscillator confirmation provides the highest Sharpe ratios in range-bound regimes.
- **Relevance:** Polymarket's 5-minute BTC binary markets operate at ultra-short horizons where the underlying BTC price often exhibits noise-dominated (mean-reverting) behavior rather than trending. Low-volume periods (off-hours, weekends) should amplify this effect. The Hurst exponent framework provides a regime-detection tool: when H < 0.5, activate mean reversion; when H > 0.5, stand aside or fade.
- **Source:** https://www.quantifiedstrategies.com/bitcoin-mean-reversion-strategies-outperform-momentum-in-low-volume-regimes/

### Ref 10: Amberdata (2025) — "Crypto Pairs Trading: Verifying Mean Reversion with ADF and Hurst Tests"

- **Author:** Amberdata Research
- **Year:** 2025
- **Key Finding:** The Augmented Dickey-Fuller (ADF) test (p-value < 0.05) and Hurst exponent (H < 0.5) together provide rigorous confirmation of stationarity and mean-reverting tendency in crypto price series. Log-transformed price relationships normalize volatility distortions, and Z-score-based entry thresholds with hedge ratio constraints produce robust signals even in volatile crypto environments.
- **Relevance:** Provides the statistical testing framework (ADF, Hurst) for validating whether the BTC price deviation implied by Polymarket binary contract prices is mean-reverting. For 5-minute contracts, we can apply ADF tests to rolling windows of the Polymarket implied probability series to detect when mean reversion is statistically valid versus when genuine information is driving price discovery.
- **Source:** https://blog.amberdata.io/crypto-pairs-trading-part-2-verifying-mean-reversion-with-adf-and-hurst-tests

---

## 4. Prediction Market Microstructure

### Ref 11: Becker (2026) — "The Microstructure of Wealth Transfer in Prediction Markets"

- **Author:** Jonathan Becker
- **Year:** 2026
- **Key Finding:** Analysis of 72.1 million trades on Kalshi reveals a systematic wealth transfer from liquidity takers (−1.12% excess return) to liquidity makers (+1.12% excess return). This is driven by the longshot bias — takers overpay for affirmative YES outcomes — and is strongest in high-engagement categories. The effect is *not* inherent to prediction market microstructure; it requires sophisticated market makers and sufficient liquidity. Makers profit via structural arbitrage (providing liquidity to biased flow), not superior forecasting.
- **Relevance:** Fundamental insight for Polymarket strategy design. Being a maker (posting resting limit orders) captures the optimism tax, while being a taker (hitting market orders) pays it. For 5-minute BTC markets, this suggests a maker-first strategy: post bids below the fair value and asks above it, capturing spread while the OBI/VAMP signals inform *where* to post. Polymarket's 25% maker rebate amplifies this structural edge.
- **Source:** https://jbecker.dev/research/prediction-market-microstructure

### Ref 12: HumanInvariant (2025) — "The Case For Alternative Ordering Mechanisms in Prediction Markets"

- **Author:** HumanInvariant
- **Year:** 2025
- **Key Finding:** All major prediction market platforms (Polymarket, Kalshi, Opinion, Limitless) use first-come-first-served (FCFS) matching, which creates a latency war favoring takers. Takers can pick off stale maker quotes when new information arrives, particularly in binary markets where a single event can move price from 1% to 100%. Priority batch auctions could level the playing field but are unlikely to be adopted by incumbents.
- **Relevance:** Confirms that FCFS market structure on Polymarket creates adverse selection risk for makers. For 5-minute BTC markets, the rapid information cycle (BTC price updates every second) means stale quotes are highly vulnerable. This implies: (a) maker strategies must include rapid quote refresh logic, (b) taker strategies exploiting stale quotes may be profitable but require ultra-low latency, and (c) our OBI/VAMP signals should inform both entry timing and quote placement to avoid being picked off.
- **Source:** https://www.humaninvariant.com/blog/pm-ordering

---

## 5. Polymarket/Kalshi Platform-Specific Research

### Ref 13: Daedalus Research / 0x_Shaw (2026) — "Polymarket Market Making Bible"

- **Author:** @0x_Shaw_dalen for Daedalus Research (via Odaily)
- **Year:** 2026
- **Key Finding:** A complete market making framework for Polymarket using Logit transformation to map probabilities to unbounded space, coupled with a jump-diffusion model for belief dynamics. The martingale property of belief processes means market makers only need to price "uncertainty" (volatility) rather than direction. The model defines prediction-market-specific Greeks (Delta = p(1-p)) and four risk types (directional, curvature, information intensity, cross-event). An improved Avellaneda-Stoikov model dynamically adjusts spreads based on inventory, volatility, and time to resolution. Calibration uses Kalman filtering and EM algorithms.
- **Relevance:** This is the most comprehensive practitioner framework for Polymarket market making. The Logit transformation is critical: binary contract prices (bounded 0–1) become unbounded, enabling standard quantitative tools. The Delta formula (p(1-p)) shows that risk is highest near 50% — exactly where 5-minute BTC markets often settle. The inventory management framework is directly applicable to maintaining positions across multiple 5-minute contracts.
- **Source:** https://www.odaily.news/en/post/5209790

### Ref 14: QuantVPS / Vasilyev (2026) — "Market Making in Prediction Markets: How Liquidity Providers Trade"

- **Author:** Thomas Vasilyev
- **Year:** 2026
- **Key Finding:** The Stoikov model for binary outcomes calculates optimal bid/ask prices by weighing inventory risk against expected spread capture. Market makers use WebSocket-based real-time order flow monitoring to detect informed trader activity (e.g., series of large buy orders hitting the ask), triggering spread widening or temporary quote withdrawal. GTD (Good Till Date) orders and kill switches are essential risk management tools. Polymarket and Kalshi combined process ~$10B monthly volume (as of Nov 2025).
- **Relevance:** Confirms that real-time order flow monitoring is standard practice for prediction market makers. For our OBI-based strategy, the WebSocket order flow signal is essentially the same information channel. The finding that informed traders reveal themselves through aggressive taker flow validates the OBI approach: when we observe sustained buy-side imbalance, informed flow is likely driving the market.
- **Source:** https://www.quantvps.com/blog/market-making-in-prediction-markets

### Ref 15: PRED Scanner (2026) — "Comparing Sports Betting Fees on Polymarket vs Kalshi in 2026"

- **Author:** PRED Scanner
- **Year:** 2026
- **Key Finding:** Polymarket charges ~0.01% taker fees (global: 0% on most markets) versus Kalshi's ~1.2% average. For 50/50 contracts, Kalshi fees can be 175x higher than Polymarket. Polymarket's 25% maker rebate incentivizes liquidity provision. Slippage follows a square root relationship with order size: $1K–$5K orders incur 1–3% slippage, $10K–$50K incur 3–10%.
- **Relevance:** Polymarket's near-zero fees are essential for 5-minute trading strategies that require high turnover. A strategy executing 100+ trades per day would face prohibitive costs on Kalshi (~$1.20/trade) but negligible costs on Polymarket (~$0.01/trade). The slippage profile informs maximum position sizing: for 5-minute contracts, keeping orders under $5K limits slippage to <3%, which is critical for maintaining edge.
- **Source:** https://www.predscanner.com/comparing-sports-betting-fees-on-polymarket-vs-kalshi-in-2026/

---

## Synthesis and Gaps

### What the Literature Confirms

1. **OBI is a proven, robust predictor** of short-horizon price direction in LOB markets, with linear OFI–price relationships achieving R² ≈ 65% (Cont et al. 2014) and significant classification accuracy (Gould & Bonart 2015). These signals are strongest for large-tick instruments — matching Polymarket's tick structure.

2. **VAMP outperforms naive mid-price** in crypto markets (Gabbireddy 2023), providing a better estimate of fair value that can be compared against the binary contract price to detect mispricings.

3. **Mean reversion is present in prediction markets** at 41–68% reversion rates depending on category (Prevayo 2026), and in crypto during low-volume regimes when H < 0.5 (QuantifiedStrategies 2026).

4. **Maker strategies capture structural edge** on prediction markets via the optimism tax (Becker 2026), amplified by Polymarket's fee structure (near-zero fees + 25% maker rebate).

### What the Literature Does NOT Cover (Gaps)

1. **No academic research on OBI/VAMP in binary prediction markets.** All OBI studies use equity markets. Whether the linear OFI–price relationship holds for binary contracts with bounded prices is untested empirically.

2. **No studies on 5-minute resolution prediction markets.** Most microstructure research operates at tick-by-tick or sub-second horizons. The signal decay rate over 5 minutes is unknown.

3. **No formal treatment of the interaction between OBI signals and binary contract pricing.** The Logit transformation from Daedalus Research provides a framework, but no empirical validation exists.

4. **Limited research on crypto-BTC prediction market microstructure.** The underlying asset (BTC) is itself highly volatile and traded 24/7, creating unique dynamics not present in equity prediction markets.

5. **No academic analysis of Polymarket's specific CLOB implementation** (matching engine speed, tick size dynamics, oracle resolution lag for BTC markets).

### Recommended Next Steps

1. **Empirical validation:** Collect Polymarket order book data for 5-minute BTC markets and test the Cont et al. (2014) linear OFI–price model directly.
2. **VAMP implementation:** Adapt Gabbireddy's (2023) Coinbase VAMP methodology to Polymarket's USDC-denominated binary contracts.
3. **Regime detection:** Implement rolling ADF/Hurst tests on Polymarket implied probability series to activate/deactivate mean reversion signals.
4. **Maker strategy backtest:** Use the Daedalus Research framework to simulate maker P&L with OBI/VAMP-informed quote placement.

---

## References

| # | Citation | Domain |
|---|----------|--------|
| 1 | Cont, R., Kukanov, A., & Stoikov, S. (2014). The Price Impact of Order Book Events. *J. Financial Econometrics*, 12(1), 47–88. | OBI |
| 2 | Gould, M.D. & Bonart, J. (2015). Queue Imbalance as a One-Tick-Ahead Price Predictor in a Limit Order Book. arXiv:1512.03492. | OBI |
| 3 | Cartea, Á., Donnelly, R.F., & Jaimungal, S. (2018). Enhancing Trading Strategies with Order Book Signals. *Applied Mathematical Finance*, 25(1), 1–35. | OBI |
| 4 | Kolm, P.N., Ritter, G., & Simunovic (2023). Deep Order Flow Imbalance: Extracting Alpha at Multiple Horizons from the LOB. | OBI |
| 5 | arXiv:2507.22712 (2025). Order Book Filtration and Directional Signal Extraction. | OBI |
| 6 | Hasbrouck, J. (2002). Stalking the "Efficient Price" in Market Microstructure Specifications. *J. Financial Markets*, 5(3), 329–339. | VAMP |
| 7 | Gabbireddy (2023). Volume-Adjusted Mid Price and Order Book Imbalance. Penn State Honors Thesis. | VAMP |
| 8 | Prevayo (2026). Mean Reversion Trading in Prediction Markets: Statistical Edge. | Mean Reversion |
| 9 | QuantifiedStrategies (2026). Bitcoin Mean Reversion Strategies Outperform Momentum in Low Volume Regimes. | Mean Reversion |
| 10 | Amberdata (2025). Crypto Pairs Trading: Verifying Mean Reversion with ADF and Hurst Tests. | Mean Reversion |
| 11 | Becker, J. (2026). The Microstructure of Wealth Transfer in Prediction Markets. | Market Microstructure |
| 12 | HumanInvariant (2025). The Case For Alternative Ordering Mechanisms in Prediction Markets. | Market Microstructure |
| 13 | Daedalus Research / 0x_Shaw (2026). Polymarket Market Making Bible. | Polymarket Specific |
| 14 | Vasilyev, T. (2026). Market Making in Prediction Markets: How Liquidity Providers Trade. | Polymarket Specific |
| 15 | PRED Scanner (2026). Comparing Sports Betting Fees on Polymarket vs Kalshi in 2026. | Polymarket Specific |
