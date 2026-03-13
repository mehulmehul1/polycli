# Fair Value Model Specification

## 1. Goal

Define a **fair-value pricing model** for Polymarket crypto binary markets that:

1. Estimates the "true" probability of YES given spot price, time to expiry, and strike
2. Calibrates to market microstructure (bid-ask spread, book imbalance)
3. Provides a reference price for detecting mispricing
4. Enables edge calculation: `fair_prob - market_price`

## 2. Market Context

### 2.1 Market types

| Family | Description | Example |
|--------|-------------|---------|
| `updown_open_close` | Will BTC close higher than it opened in 5 minutes? | `btc-updown-5m-1772243100` |
| `threshold_at_expiry` | Will BTC be above $85,000 at expiry? | `btc-above-85k-2026-03-15` |

### 2.2 Market structure

- **Duration**: 5m, 15m, 1h
- **Outcomes**: Binary (YES/NO)
- **Resolution**: Determined by spot price at specific timestamp
- **Fee model**: ~2-4% round-trip (open + close)
- **Liquidity**: Varies, often $10K-$100K per market

### 2.3 Why fair value matters

Current strategy (`signal.rs`) uses **token-price technical analysis**:
- EMA crossovers on YES token price
- Bollinger Band expansion
- RSI extremes

This is **price-following**, not value-seeking. A fair-value model enables:
- **Directional edge**: Enter when market misprices true probability
- **Size scaling**: Larger positions when edge is larger
- **Exit discipline**: Exit when edge disappears, not just when momentum flips

## 3. Fair Value Model v1: Digital Option Approximation

### 3.1 Theoretical foundation

A binary prediction market on "Will BTC close above current price in 5 minutes?" is mathematically equivalent to a **digital (binary) option**.

For a threshold-at-expiry market:
- Strike $K$ = target price
- Spot $S$ = current price
- Time to expiry $T$ = seconds until resolution
- Volatility $\sigma$ = realized volatility of spot

The Black-Scholes-style fair probability of YES is:

$$P_{YES} = N(d_2)$$

Where:
- $d_2 = \frac{\ln(S/K) + (r - \sigma^2/2)T}{\sigma\sqrt{T}}$
- $N(\cdot)$ = standard normal CDF
- $r$ = risk-free rate (≈ 0 for short horizons)

For **up-down markets** where $K = S_0$ (strike equals entry price):

$$d_2 = \frac{-\sigma^2 T / 2}{\sigma\sqrt{T}} = -\frac{\sigma\sqrt{T}}{2}$$

$$P_{YES} = N\left(-\frac{\sigma\sqrt{T}}{2}\right)$$

This is slightly below 0.5 due to the volatility drag term.

### 3.2 Simplified model for v1

For short horizons (5m-1h), we use a **drift-diffusion model**:

```rust
/// Fair probability model for crypto binary markets
pub struct FairValueModel {
    config: FairValueConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FairValueConfig {
    /// Annualized volatility estimate
    pub base_volatility: f64,
    
    /// Vol scaling for short horizons
    pub vol_scale_short: f64,
    
    /// Drift adjustment (trend following)
    pub drift_weight: f64,
    
    /// Mean reversion for extreme prices
    pub mean_reversion_strength: f64,
    
    /// Use realized vol instead of base
    pub use_realized_vol: bool,
    
    /// Realized vol lookback in seconds
    pub realized_vol_lookback: i64,
}

impl Default for FairValueConfig {
    fn default() -> Self {
        Self {
            base_volatility: 0.60,        // 60% annualized for BTC
            vol_scale_short: 1.2,         // boost vol for short horizons
            drift_weight: 0.10,           // 10% weight on recent drift
            mean_reversion_strength: 0.05,
            use_realized_vol: true,
            realized_vol_lookback: 300,   // 5 minutes
        }
    }
}

impl FairValueModel {
    /// Calculate fair probability for up-down market
    ///
    /// # Arguments
    /// * `spot_current` - Current spot price
    /// * `spot_at_open` - Spot price at market open
    /// * `time_remaining_s` - Seconds until market resolution
    /// * `realized_vol` - Recent realized volatility (if available)
    ///
    /// # Returns
    /// Fair probability of YES (market closes higher than open)
    pub fn fair_prob_updown(
        &self,
        spot_current: f64,
        spot_at_open: f64,
        time_remaining_s: i64,
        realized_vol: Option<f64>,
    ) -> f64 {
        let t = (time_remaining_s as f64) / (365.25 * 24.0 * 3600.0); // years
        
        // Use realized vol if available, else base
        let sigma_ann = realized_vol
            .unwrap_or(self.config.base_volatility)
            * self.config.vol_scale_short;
        
        // Convert to per-second vol
        let sigma_t = sigma_ann * t.sqrt();
        
        // Log return since open
        let log_return = (spot_current / spot_at_open).ln();
        
        // Drift adjustment (recent trend)
        let drift = log_return * self.config.drift_weight;
        
        // For up-down: strike = spot_at_open
        // d2 = -sigma * sqrt(T) / 2 + drift_adjustment
        let d2 = -sigma_t / 2.0 + drift;
        
        // Convert to probability
        let prob = normal_cdf(d2);
        
        // Mean reversion for extreme probabilities
        let mean_reverted = prob + self.config.mean_reversion_strength * (0.5 - prob);
        
        mean_reverted.clamp(0.05, 0.95)
    }
    
    /// Calculate fair probability for threshold-at-expiry market
    ///
    /// # Arguments
    /// * `spot_current` - Current spot price
    /// * `strike` - Threshold price
    /// * `time_remaining_s` - Seconds until expiry
    /// * `realized_vol` - Recent realized volatility
    pub fn fair_prob_threshold(
        &self,
        spot_current: f64,
        strike: f64,
        time_remaining_s: i64,
        realized_vol: Option<f64>,
    ) -> f64 {
        let t = (time_remaining_s as f64) / (365.25 * 24.0 * 3600.0);
        
        let sigma_ann = realized_vol
            .unwrap_or(self.config.base_volatility)
            * self.config.vol_scale_short;
        
        let sigma_t = sigma_ann * t.sqrt();
        
        // Moneyness
        let moneyness = (spot_current / strike).ln();
        
        // d2 = ln(S/K) / (sigma * sqrt(T)) - sigma * sqrt(T) / 2
        let d2 = moneyness / sigma_t - sigma_t / 2.0;
        
        let prob = normal_cdf(d2);
        
        prob.clamp(0.05, 0.95)
    }
    
    /// Calculate edge: fair_prob - market_ask
    ///
    /// Positive edge = market underprices YES
    /// Negative edge = market overprices YES
    pub fn calculate_edge(&self, fair_prob: f64, market_yes_ask: f64) -> f64 {
        fair_prob - market_yes_ask
    }
}

/// Standard normal CDF approximation (Abramowitz & Stegun)
fn normal_cdf(x: f64) -> f64 {
    // Constants
    const A1: f64 = 0.254829592;
    const A2: f64 = -0.284496736;
    const A3: f64 = 1.421413741;
    const A4: f64 = -1.453152027;
    const A5: f64 = 1.061405429;
    const P: f64 = 0.3275911;
    
    let sign = if x < 0.0 { -1.0 } else { 1.0 };
    let x = x.abs() / std::f64::consts::SQRT_2;
    
    let t = 1.0 / (1.0 + P * x);
    let y = 1.0 - (((((A5 * t + A4) * t) + A3) * t + A2) * t + A1) * t * (-x * x).exp();
    
    0.5 * (1.0 + sign * y)
}
```

### 3.3 Realized volatility calculation

```rust
/// Calculate realized volatility from spot returns
///
/// Returns annualized volatility
pub fn calculate_realized_vol(spot_prices: &[(i64, f64)], lookback_s: i64) -> Option<f64> {
    if spot_prices.len() < 10 {
        return None;
    }
    
    let now_ts = spot_prices.last()?.0;
    let cutoff = now_ts - lookback_s;
    
    // Filter to lookback window
    let prices: Vec<f64> = spot_prices
        .iter()
        .filter(|(ts, _)| *ts >= cutoff)
        .map(|(_, p)| *p)
        .collect();
    
    if prices.len() < 10 {
        return None;
    }
    
    // Calculate log returns
    let returns: Vec<f64> = prices
        .windows(2)
        .map(|w| (w[1] / w[0]).ln())
        .collect();
    
    if returns.is_empty() {
        return None;
    }
    
    // Mean return
    let mean = returns.iter().sum::<f64>() / returns.len() as f64;
    
    // Variance
    let variance = returns
        .iter()
        .map(|r| (r - mean).powi(2))
        .sum::<f64>()
        / (returns.len() - 1) as f64;
    
    // Annualize (assume 1-second samples)
    // Annualization factor = sqrt(seconds_per_year) = sqrt(365.25 * 24 * 3600) ≈ 19012
    let annualization = (365.25 * 24.0 * 3600.0_f64).sqrt();
    
    Some(variance.sqrt() * annualization)
}
```

## 4. Calibration Layer

### 4.1 Why calibration?

The theoretical model assumes:
- Efficient markets
- No bid-ask spread
- No fees
- Gaussian returns

Reality:
- Polymarket has 2-4% round-trip fees
- Spreads vary from 1-8 cents
- Orderbook imbalances create structural mispricing
- Crypto returns are fat-tailed

### 4.2 Calibration approach

```rust
/// Calibrated fair value with market microstructure adjustments
pub struct CalibratedFairValue {
    base_model: FairValueModel,
    config: CalibrationConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CalibrationConfig {
    /// Spread cost adjustment (reduce fair prob by spread/2)
    pub spread_adjustment: f64,
    
    /// Book imbalance signal weight
    pub book_imbalance_weight: f64,
    
    /// Historical bias adjustment (if YES wins 55% of time, add 0.025)
    pub historical_bias: f64,
    
    /// Min edge to consider tradeable
    pub min_edge_threshold: f64,
    
    /// Max edge (cap to prevent overconfidence)
    pub max_edge: f64,
}

impl Default for CalibrationConfig {
    fn default() -> Self {
        Self {
            spread_adjustment: 0.5,      // full spread cost
            book_imbalance_weight: 0.10,
            historical_bias: 0.0,        // learn from data
            min_edge_threshold: 0.03,    // 3 cents minimum
            max_edge: 0.10,              // cap at 10 cents
        }
    }
}

#[derive(Debug, Clone)]
pub struct CalibratedProb {
    /// Base theoretical probability
    pub fair_prob_base: f64,
    
    /// After spread adjustment
    pub fair_prob_adjusted: f64,
    
    /// After book imbalance
    pub fair_prob_calibrated: f64,
    
    /// Edge vs market
    pub edge_yes: f64,
    pub edge_no: f64,
    
    /// Is edge tradeable?
    pub tradeable: bool,
}

impl CalibratedFairValue {
    /// Calculate calibrated fair value with all adjustments
    pub fn calculate(
        &self,
        spot_current: f64,
        spot_at_open: f64,
        time_remaining_s: i64,
        realized_vol: Option<f64>,
        yes_ask: f64,
        no_ask: f64,
        yes_spread: f64,
        book_sum: f64,
    ) -> CalibratedProb {
        // 1. Base theoretical probability
        let fair_prob_base = self.base_model.fair_prob_updown(
            spot_current,
            spot_at_open,
            time_remaining_s,
            realized_vol,
        );
        
        // 2. Spread adjustment (cost to trade)
        let spread_cost = yes_spread * self.config.spread_adjustment;
        let fair_prob_adjusted = fair_prob_base - spread_cost / 2.0;
        
        // 3. Book imbalance adjustment
        // book_sum > 1.0 means YES is overpriced
        let book_imbalance = (book_sum - 1.0) * self.config.book_imbalance_weight;
        let fair_prob_calibrated = (fair_prob_adjusted - book_imbalance)
            .add(self.config.historical_bias)
            .clamp(0.05, 0.95);
        
        // 4. Edge calculation
        let edge_yes = fair_prob_calibrated - yes_ask;
        let edge_no = (1.0 - fair_prob_calibrated) - no_ask;
        
        // 5. Tradeability
        let tradeable = edge_yes.abs() >= self.config.min_edge_threshold
            || edge_no.abs() >= self.config.min_edge_threshold;
        
        CalibratedProb {
            fair_prob_base,
            fair_prob_adjusted,
            fair_prob_calibrated,
            edge_yes: edge_yes.clamp(-self.config.max_edge, self.config.max_edge),
            edge_no: edge_no.clamp(-self.config.max_edge, self.config.max_edge),
            tradeable,
        }
    }
}
```

## 5. Integration with Strategy Engine

### 5.1 StrategyObservation extension

```rust
// Add to StrategyObservation in strategy_engine_spec.md

pub struct StrategyObservation {
    // ... existing fields ...
    
    /// Spot price data
    pub spot_price: Option<f64>,
    pub spot_at_market_open: Option<f64>,
    pub spot_realized_vol_30s: Option<f64>,
    pub spot_realized_vol_60s: Option<f64>,
    
    /// Pre-computed fair value (populated by feed layer)
    pub fair_value: Option<CalibratedProb>,
}
```

### 5.2 Fair-value signal checker

```rust
// src/bot/strategy/fair_value_checker.rs

pub struct FairValueChecker {
    config: FairValueSignalConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FairValueSignalConfig {
    /// Minimum edge to trigger entry
    pub min_edge: f64,
    
    /// Require edge to be tradeable (after costs)
    pub require_tradeable: bool,
    
    /// Direction filter: only long YES, only long NO, or both
    pub allowed_directions: AllowedDirections,
    
    /// Confidence scaling: higher edge = higher confidence
    pub edge_to_confidence_scale: f64,
}

#[derive(Debug, Clone, Deserialize)]
pub enum AllowedDirections {
    YesOnly,
    NoOnly,
    Both,
}

impl FairValueChecker {
    pub fn check(&self, fair_value: &CalibratedProb) -> Option<(Direction, EntryReason)> {
        if self.config.require_tradeable && !fair_value.tradeable {
            return None;
        }
        
        let (direction, edge) = match self.config.allowed_directions {
            AllowedDirections::YesOnly if fair_value.edge_yes >= self.config.min_edge => {
                (Direction::Yes, fair_value.edge_yes)
            }
            AllowedDirections::NoOnly if fair_value.edge_no >= self.config.min_edge => {
                (Direction::No, fair_value.edge_no)
            }
            AllowedDirections::Both => {
                if fair_value.edge_yes >= fair_value.edge_no 
                    && fair_value.edge_yes >= self.config.min_edge {
                    (Direction::Yes, fair_value.edge_yes)
                } else if fair_value.edge_no >= self.config.min_edge {
                    (Direction::No, fair_value.edge_no)
                } else {
                    return None;
                }
            }
            _ => return None,
        };
        
        let confidence = Confidence::new(
            (edge / self.config.edge_to_confidence_scale).min(1.0)
        );
        
        Some((direction, EntryReason {
            source: SignalSource::FairValue,
            confidence,
            detail: format!(
                "edge={:.4}, base={:.4}, calibrated={:.4}",
                edge, fair_value.fair_prob_base, fair_value.fair_prob_calibrated
            ),
            fair_value_edge: Some(edge),
            qlib_score: None,
        }))
    }
}
```

### 5.3 Fused engine with fair value

```rust
// In QlibFusedEngine::check_entry

fn check_entry(&mut self, obs: &StrategyObservation) -> Option<StrategyDecision> {
    // Gather all signals
    let indicator_signal = self.indicator_checker.check(&obs.ind_5s, &obs.ind_1m, obs.yes_mid);
    let book_signal = self.book_checker.check(obs.book_sum);
    let fair_value_signal = obs.fair_value.as_ref().and_then(|fv| {
        self.fair_value_checker.check(fv)
    });
    
    // Priority order (configurable)
    // v1: fair_value > indicators > book
    let entry = fair_value_signal
        .or(indicator_signal)
        .or(book_signal)?;
    
    Some(StrategyDecision::Enter {
        direction: entry.0,
        reason: entry.1,
        suggested_size_usd: self.calculate_size(&entry.1.confidence),
    })
}
```

## 6. Spot Feed Integration

### 6.1 Spot price sources

| Source | Latency | Reliability | Cost |
|--------|---------|-------------|------|
| Binance WebSocket | <100ms | High | Free |
| Coinbase WebSocket | <200ms | High | Free |
| PMXT archive | 1s-1h | High | Free |
| Aggregated (CCXT) | <500ms | Medium | Free |

### 6.2 Spot feed interface

```rust
// src/bot/feed/spot.rs

pub trait SpotFeed: Send + Sync {
    /// Get current spot price for asset
    fn current_price(&self, asset: &str) -> Option<f64>;
    
    /// Get price at specific timestamp (for replay)
    fn price_at(&self, asset: &str, ts: i64) -> Option<f64>;
    
    /// Get historical prices for vol calculation
    fn price_history(&self, asset: &str, lookback_s: i64) -> Vec<(i64, f64)>;
    
    /// Is feed healthy?
    fn is_healthy(&self) -> bool;
    
    /// Seconds since last update
    fn staleness(&self) -> u64;
}

/// Binance WebSocket spot feed
pub struct BinanceSpotFeed {
    asset: String,
    current_price: Arc<RwLock<Option<f64>>>,
    price_history: Arc<RwLock<VecDeque<(i64, f64)>>>,
    last_update: Arc<RwLock<u64>>,
}

impl SpotFeed for BinanceSpotFeed {
    fn current_price(&self, asset: &str) -> Option<f64> {
        *self.current_price.read().unwrap()
    }
    
    fn price_history(&self, asset: &str, lookback_s: i64) -> Vec<(i64, f64)> {
        let now = current_timestamp();
        let cutoff = now - lookback_s;
        
        self.price_history
            .read()
            .unwrap()
            .iter()
            .filter(|(ts, _)| *ts >= cutoff)
            .copied()
            .collect()
    }
    
    fn is_healthy(&self) -> bool {
        self.staleness() < 5 // 5 second max staleness
    }
    
    fn staleness(&self) -> u64 {
        let last = *self.last_update.read().unwrap();
        current_timestamp().saturating_sub(last)
    }
}
```

### 6.3 CLI flags

```bash
# Enable fair-value mode with Binance spot feed
polymarket bot watch-btc \
  --engine fused \
  --spot-feed binance \
  --fair-value-primary

# Backtest with historical spot from PMXT
polymarket bot backtest \
  --input pmxt.parquet \
  --with-spot spot.parquet \
  --engine fused
```

## 7. Validation and Calibration

### 7.1 Historical calibration

Run on historical PMXT data to find optimal parameters:

```bash
python -m research.calibrate_fair_value \
  --features btc_5m_v1.parquet \
  --spot btc_spot_history.parquet \
  --out calibration_params.json
```

Output:
```json
{
  "base_volatility": 0.65,
  "vol_scale_short": 1.3,
  "drift_weight": 0.15,
  "spread_adjustment": 0.6,
  "book_imbalance_weight": 0.12,
  "historical_bias": 0.02,
  "min_edge_threshold": 0.025
}
```

### 7.2 Validation metrics

| Metric | Target | Current (heuristic) | Target (fair-value) |
|--------|--------|---------------------|---------------------|
| Win rate | > 50% | 36% | > 55% |
| Avg edge per trade | > 2% | -3.4% | > 2% |
| Sharpe ratio | > 1.0 | -0.8 | > 1.5 |
| Max drawdown | < 10% | 1.3% | < 5% |
| Edge correlation | > 0.3 | N/A | > 0.5 |

### 7.3 Ongoing monitoring

```rust
/// Track fair-value model performance
pub struct FairValueMonitor {
    predictions: Vec<FairValuePrediction>,
}

#[derive(Debug, Clone)]
pub struct FairValuePrediction {
    pub ts: i64,
    pub condition_id: String,
    pub fair_prob_calibrated: f64,
    pub market_yes_ask: f64,
    pub edge_predicted: f64,
    pub outcome: Option<bool>, // true = YES won
}

impl FairValueMonitor {
    /// Record prediction and outcome
    pub fn record(&mut self, pred: FairValuePrediction) {
        self.predictions.push(pred);
    }
    
    /// Calculate Brier score (lower is better)
    pub fn brier_score(&self) -> f64 {
        let resolved: Vec<_> = self.predictions
            .iter()
            .filter_map(|p| p.outcome.map(|o| (p.fair_prob_calibrated, o)))
            .collect();
        
        if resolved.is_empty() {
            return f64::NAN;
        }
        
        resolved
            .iter()
            .map(|(prob, outcome)| {
                let target = if *outcome { 1.0 } else { 0.0 };
                (prob - target).powi(2)
            })
            .sum::<f64>()
            / resolved.len() as f64
    }
    
    /// Calculate edge correlation
    pub fn edge_correlation(&self) -> f64 {
        // Correlation between predicted edge and realized outcome
        // TODO: implement
        0.0
    }
}
```

## 8. Implementation Phases

### Phase 1: Basic fair-value model

- [ ] Implement `FairValueModel::fair_prob_updown`
- [ ] Implement `FairValueModel::fair_prob_threshold`
- [ ] Add `normal_cdf` helper
- [ ] Add unit tests against known option prices

### Phase 2: Realized volatility

- [ ] Implement `calculate_realized_vol`
- [ ] Add spot price history buffer
- [ ] Integrate with existing `CandleEngine`

### Phase 3: Calibration layer

- [ ] Implement `CalibratedFairValue`
- [ ] Add spread and book imbalance adjustments
- [ ] Add `FairValueChecker` for strategy integration

### Phase 4: Spot feed

- [ ] Implement `BinanceSpotFeed`
- [ ] Add `--spot-feed` CLI flag
- [ ] Add staleness detection and fallback

### Phase 5: Strategy integration

- [ ] Add `fair_value` field to `StrategyObservation`
- [ ] Add fair-value signal to `QlibFusedEngine`
- [ ] Add `--fair-value-primary` mode

### Phase 6: Validation

- [ ] Run historical calibration on PMXT data
- [ ] Compare fair-value vs heuristic in replay
- [ ] Add `FairValueMonitor` for live tracking

## 9. References

- Black-Scholes option pricing: https://en.wikipedia.org/wiki/Black%E2%80%93Scholes_model
- Binary options: https://en.wikipedia.org/wiki/Binary_option
- Realized volatility estimation: https://quant.stackexchange.com/questions/26030/
- Polymarket fee structure: https://docs.polymarket.com/
- BTC volatility statistics: https://ycharts.com/indicators/bitcoin_volatility
