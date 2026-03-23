# Scalper + Fair Value Strategy Fusion Plan

## 1. Goal
Combine the **theoretical precision** of the Black-Scholes Fair Value model with the **timing and microstructure awareness** of the Scalper strategy. This fusion aims to protect the bankroll from "falling knives" and noisy "coin-flip" regimes.

## 2. Core Components

### Engine A: The Brain (Black-Scholes)
- **Primary Input**: BTC/USD Spot Price via Polymarket RTDS.
- **Output**: BS Fair Probability ($P_{BS}$).
- **Role**: Determines the fundamental value and directional edge.

### Engine B: The Eyes (Scalper Indicators)
- **Primary Input**: 5s and 1m Candle Data (YES/NO token prices).
- **Key Indicators**: EMA3/6, RSI14, Momentum Slope, Bollinger Band Width.
- **Role**: Determines market regime and confirms momentum timing.

---

## 3. Implementation Plan

### Phase 1: Entry Filter (Momentum Confirmation)
Prevent entering a trade when the market tape is moving aggressively against the theoretical value.

- **YES Entry Requirements**:
    1. **Edge**: $P_{BS} - YES_{ask} \ge min\_edge$ (Value exist)
    2. **Conviction**: $P_{BS} \ge 0.55$ (Not a coin-flip)
    3. **Momentum Gate**: `momentum_slope >= -0.002` (Tape is not crashing)
    4. **Trend Confirmation (Optional)**: `EMA3 >= EMA6` (Short-term reversal started)

- **NO Entry Requirements**:
    1. **Edge**: $(1.0 - P_{BS}) - NO_{ask} \ge min\_edge$ (Value exist)
    2. **Conviction**: $P_{BS} \le 0.45$ (Not a coin-flip)
    3. **Momentum Gate**: `momentum_slope <= 0.002` (Tape is not mooning)
    4. **Trend Confirmation (Optional)**: `EMA3 <= EMA6` (Short-term reversal started)

### Phase 2: Exit Improvement (Momentum Stops)
Allow the bot to exit a position early if the trend flips, even if the math still shows an edge.

- **Strategy**: Move momentum check to the **Strategy Runner**.
- **Exit Triggers**:
    - **Long YES**: Exit if `momentum_slope < -0.005` or `EMA3 crosses below EMA6`.
    - **Long NO**: Exit if `momentum_slope > 0.005` or `EMA3 crosses above EMA6`.
- **Benefit**: Protects against fundamental shifts where the crowd knows something the BS model doesn't (e.g., whale sell-off).

### Phase 3: Regime-Aware Edge (Bollinger Band Width)
Dynamically adjust entry sensitivity based on market volatility.

- **Regime: Tight (`bb_width < 0.15`)**: 
    - Market is sleeping. Use standard `min_edge`. 
    - **Rule**: Bypass momentum gate (it's too noisy in flat markets).
- **Regime: Expansion (`bb_width` 0.15 - 0.90)**:
    - Normal trading. Standard `min_edge` + Full Momentum Gate.
- **Regime: Reversal (`bb_width > 0.90`)**:
    - Market is panicking. 
    - **Rule**: Double the `min_edge` requirement (e.g., 0.06). Only bet if the mispricing is extreme.

---

## 4. Technical Tasks
1. [ ] **Update `decide()` logic** in `fair_value.rs` to accept `IndicatorState`.
2. [ ] **Implement Deadzone logic** ($0.45 < P_{BS} < 0.55 \rightarrow HOLD$).
3. [ ] **Implement Momentum Gate** for YES/NO entries.
4. [ ] **Update Strategy Runner** to handle `ExitReason::MomentumFlip`.
5. [ ] **Add CLI support** for `--strategy fused`.

---

## 5. Success Metrics (The $5 Challenge)
- **Positive EV**: Average trade profit > 0 over 100 simulated trades.
- **Max Drawdown**: Reduced by > 30% compared to pure `fairvalue` strategy.
- **Win Rate**: Target > 55% in directional regimes.
