# Research Plan: Exploitable Short-Term Trading Strategies in Bitcoin Binary Prediction Markets

## **Topic**
Algorithmic trading strategies for ultra-short-term (5-minute) binary prediction markets on Bitcoin price, focusing on the microstructure of limit order books (LOB) and the unique dynamics of probability-based markets.

## **Novel Angle**
**What specific aspect has NOT been well-studied?**
Existing research either (1) studies high-frequency trading (HFT) in traditional cryptocurrency *spot/perpetual* markets, focusing on price discovery and LOB dynamics, or (2) analyzes prediction markets for longer-term events (elections, sports) where fundamental information dominates. **The novel intersection is applying quantitative LOB microstructure signals—traditionally used for alpha in continuous markets—to the discrete, probability-based framework of short-horizon binary options (e.g., "Will BTC be above $X in 5 minutes?").** This is not a simple translation; the asset is a noisy probability (0-1), not a price, and market dynamics are driven by sentiment, hedging demand, and the market maker's fair value model, not just continuous arbitrage.

**Why is this timely NOW (2024-2026)?**
1.  **Rise of Liquid On-Chain Prediction Markets:** Platforms like Polymarket have achieved significant liquidity for major crypto events, making sub-minute analysis and trading feasible for the first time. Their BTC up/down 5-minute contracts represent a new, high-frequency data stream.
2.  **Data Accessibility:** The public, granular nature of on-chain and API-accessible LOB data for these markets enables rigorous backtesting without the opaque data barriers of traditional finance.
3.  **Convergence of DeFi and Quant Finance:** There is a growing research interest in applying sophisticated quantitative methods to DeFi primitives. This work sits at that frontier, treating a prediction market contract as a novel derivative instrument.

**How does this differ from standard approaches?**
-   **Standard Crypto HFT:** Focuses on triangular arbitrage, funding rate arbitrage, or statistical arbitrage between spot and futures. Our "asset" is a derivative on probability, requiring a different signal transformation (e.g., converting order book imbalance into a probability forecast).
-   **Standard Prediction Market Research:** Focuses on long-term information aggregation, wisdom of the crowd, or manipulation. We ignore long-term fundamentals and focus purely on extracting alpha from short-term microstructural inefficiencies and mean-reversion in the *probability space* itself.
-   **Key Innovation:** We propose a hybrid signal framework that **combines a "fair value" estimator** (derived from a high-frequency BTC price oracle or a short-term volatility model) **with LOB imbalance metrics** (adapted for probability markets) to identify transient mispricings between the contract's traded probability and its theoretically correct value.

## **Scope**
This project will focus exclusively on **Polymarket's "Bitcoin Up/Down in 5 Minutes" contracts**. The scope is limited to:
1.  Deriving and backtesting a **fair value model** for the 5-minute binary outcome based on a real-time BTC price feed and a short-term volatility estimate.
2.  Adapting and testing **LOB imbalance signals** (e.g., volume imbalance, weighted mid-price pressure) for a market making in probabilities.
3.  Developing and evaluating a **simple, combined strategy** that triggers trades when the market price deviates significantly from the fair value estimate, confirmed by LOB pressure.
4.  The analysis will be statistical and simulation-based, not involving live trading with real capital.

## **SMART Goal**
**Specific:** Develop and rigorously backtest a novel, combined signal (Fair Value + LOB Imbalance) trading strategy for Polymarket's BTC 5-minute binary contracts. The strategy will be implemented in Python, using historical LOB and trade data.
**Measurable:** Achieve a **statistically significant positive return** (p < 0.05) in out-of-sample backtesting, with a **Sharpe Ratio > 2.0** and a **maximum drawdown < 15%** over a 3-month simulated period, net of estimated transaction fees (0.5% per contract side).
**Achievable:** Using publicly available data, a single GPU for model calibration (if using ML for fair value), and standard Python data science libraries (pandas, numpy, scikit-learn, a backtesting framework like `vectorbt`).
**Relevant:** Directly advances the frontier of quantitative finance in decentralized prediction markets, providing a blueprint for exploiting a new asset class's microstructure.
**Time-bound:** Complete data collection, strategy development, backtesting, and analysis within a **4-month period**.

## **Constraints**
-   **Compute Budget:** Single consumer-grade GPU (e.g., RTX 3090/4090). Backtesting must be feasible in hours, not days.
-   **Available Tools:** Python ecosystem. Public Polymarket API and/or The Graph subgraphs for historical data. A high-frequency BTC/USD price feed (e.g., from Binance, Coinbase).
-   **Data Access:** Historical LOB snapshots (Level 2 data) and trade data for the specific Polymarket contracts. This is the primary challenge; we will assume access via a partnership, paid data provider, or a successfully scraped dataset for the period of study.

## **Success Criteria**
For publication in a top quantitative finance or digital asset conference/journal (e.g., IEEE CSCI, ACM AFT, or a specialized workshop), the results must demonstrate:
1.  **Statistical Edge:** Out-of-sample performance metrics (Sharpe, Sortino, ROI) are positive and statistically significant against a null hypothesis of zero alpha, using bootstrapping or similar methods.
2.  **Novel Contribution:** Clear articulation of how the adapted signals for probability markets differ from and outperform their direct application from traditional markets.
3.  **Robustness:** Strategy performance is stable across different market volatility regimes (high/low BTC vol) and is not the result of overfitting to a short anomaly.
4.  **Reproducibility:** Code and methodology are sufficiently documented to allow replication, even if raw data cannot be fully shared due to licensing.

## **Benchmark**
Since no standard benchmark exists for this specific task, we will establish our own:
-   **Name:** `Polymarket-BTC-5min-v1`
-   **Source:** Historical Polymarket contract data for BTC up/down 5-minute markets (to be collected for a period of at least 6 months in 2024).
-   **Metrics:**
    -   **Primary:** Return on Investment (ROI), Sharpe Ratio (annualized), Maximum Drawdown.
    -   **Secondary:** Win Rate, Profit Factor, Strategy Latency (signal-to-trade).
-   **Current SOTA:** **None established.** The baseline will be a **naive "fair value only" strategy** (e.g., buy when market probability < model probability by a threshold) and a **"LOB imbalance only" strategy**. Our novel combined strategy must outperform both.

## **Trend Validation**
1.  **Paper 1:** *Daian, P., et al. (2024). "Flash Boys 2.0: Frontrunning, Transaction Reordering, and Consensus Instability in Decentralized Exchanges."* (Forthcoming/Updated). **Relevance:** Establishes the critical importance and profitability of microstructure analysis (MEV, arbitrage) in decentralized exchanges, creating a direct intellectual pathway to analyzing prediction market LOBs.
2.  **Paper 2:** *Brauneis, A., et al. (2023). "Price Discovery in Prediction Markets: A High-Frequency Analysis."* Journal of Financial Markets. **Relevance:** While focused on longer horizons, this paper provides a recent methodological framework for analyzing price discovery and efficiency in prediction markets, which we will adapt to a much shorter, noisier timeframe.
3.  **Paper 3:** *Leung, T., & Li, H. (2024). "Optimal Market Making in Prediction Markets."* Applied Mathematics & Optimization. **Relevance:** Provides a theoretical foundation for market making and fair value in prediction markets, which informs our fair value model and highlights the inventory/risk dynamics our strategy must navigate.

---
**Generated:** 2024-05-16