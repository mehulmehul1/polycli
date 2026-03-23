# Fair Value + Temporal Arbitrage Integration Plan

## Executive Summary

Combine the sophisticated **risk-neutral pricing model** (Fair Value) with the **information flow advantage** of Temporal Arbitrage to create a superior trading strategy.

## Current State Analysis

### Fair Value Strategy Strengths
- **Risk-neutral pricing** using logit jump-diffusion (arxiv:2510.15205)
- **Kalman filtering** for microstructure noise reduction
- **EM algorithm** for jump parameter estimation
- **Horizon-based adaptation** (UltraShort, Short, Medium, Long)
- **Regime detection** via Bollinger Band Width

### Temporal Arbitrage Strengths
- **Information advantage** from observing 5min results before 15min closes
- **Bayesian updating** as each 5min market resolves
- **Transition probability modeling** for momentum/reversion
- **Cross-horizon dependencies** (5min → 15min → 1hour)

### Key Insight: Complementary Signals

| Aspect | Fair Value | Temporal Arbitrage |
|--------|-----------|-------------------|
| Primary Edge | Model-based mispricing | Information flow advantage |
| Time Horizon | Single market | Multi-market chain |
| Signal Type | Static probability | Dynamic updating |
| Noise Handling | Kalman filter | Sequential filtering |
| Key Assumption | Martingale pricing | Transition probabilities |

---

## Integration Architecture

```
┌─────────────────────────────────────────────────────────────────────┐
│                        SIGNAL FUSION ENGINE                           │
│  ┌──────────────┐    ┌──────────────────┐    ┌─────────────────┐  │
│  │ Fair Value   │    │  Temporal        │    │   Regime        │  │
│  │ Signal       │◄──►│  Arbitrage       │◄──►│   Detection     │  │
│  │              │    │  Signal          │    │                 │  │
│  └──────┬───────┘    └──────┬───────────┘    └────────┬────────┘  │
│         │                   │                       │             │
│         └───────────────────┼───────────────────────┘             │
│                             ▼                                     │
│                  ┌─────────────────┐                              │
│                  │  Multi-Horizon  │                              │
│                  │  Probability    │                              │
│                  │  Engine         │                              │
│                  └────────┬────────┘                              │
│                           ▼                                       │
│                  ┌─────────────────┐                              │
│                  │  Final Signal   │                              │
│                  │  Generator      │                              │
│                  └─────────────────┘                              │
└─────────────────────────────────────────────────────────────────────┘
```

---

## Component 1: Multi-Horizon Probability Engine

### Extended FairProbability Structure

```rust
pub struct MultiHorizonFairValue {
    /// Fair value from risk-neutral model
    pub risk_neutral_fv: f64,

    /// Temporal arbitrage adjusted probability
    pub temporal_adjusted_fv: f64,

    /// Confidence in the combined estimate
    pub confidence: f64,

    /// Breakdown by horizon
    pub by_horizon: HashMap<MarketHorizon, HorizonFairValue>,
}

pub struct HorizonFairValue {
    /// Fair value for this specific horizon
    pub fair_prob: f64,

    /// Market price for this horizon's markets
    pub market_prob: f64,

    /// Edge at this horizon
    pub edge: f64,

    /// Weight in final decision (higher = more important)
    pub weight: f64,

    /// Temporal phase (for chained markets)
    pub temporal_phase: Option<TemporalPhase>,
}

pub enum TemporalPhase {
    /// Initial phase (no 5min resolved)
    Initial,
    /// After first 5min resolved
    AfterFirst { result: PriceMove, magnitude: f64 },
    /// After second 5min resolved
    AfterSecond {
        first: PriceMove,
        second: PriceMove,
        cumulative: f64
    },
}
```

### Integration Point 1: Enhanced LogitJumpDiffusion

```rust
impl LogitJumpDiffusion {
    /// Calculate fair value incorporating temporal information
    pub fn fair_prob_with_temporal(
        &self,
        time_remaining: i64,
        temporal_info: Option<&TemporalChainInfo>,
    ) -> FairProbability {
        // Base fair value from risk-neutral model
        let base_fv = self.fair_prob(time_remaining);

        match temporal_info {
            None => base_fv,
            Some(info) => {
                // Adjust based on temporal observations
                let temporal_adjustment = match info.phase {
                    TemporalPhase::Initial => 0.0,
                    TemporalPhase::AfterFirst { result, magnitude } => {
                        self.temporal_adjustment_after_first(result, magnitude, time_remaining)
                    },
                    TemporalPhase::AfterSecond { first, second, cumulative } => {
                        self.temporal_adjustment_after_second(
                            first, second, *cumulative, time_remaining
                        )
                    },
                };

                FairProbability {
                    expected: (base_fv.expected + temporal_adjustment).clamp(0.01, 0.99),
                    confidence: base_fv.confidence * info.information_quality,
                    ..base_fv
                }
            }
        }
    }

    /// Calculate adjustment after observing first 5min result
    fn temporal_adjustment_after_first(
        &self,
        result: &PriceMove,
        magnitude: f64,
        time_remaining: i64,
    ) -> f64 {
        // Combine:
        // 1. Magnitude of the move (large move = stronger signal)
        // 2. Volatility state (high vol = less confidence in move)
        // 3. Time remaining (less time = stronger signal)
        // 4. Transition probabilities (momentum vs reversion)

        let vol = self.vol;
        let signal_strength = (magnitude / 50.0).min(1.0);  // Normalize by $50
        let time_factor = (900.0 - time_remaining as f64) / 900.0;  // 0 to 1

        let base_adjustment = match result {
            PriceMove::Up { .. } => 0.1 * signal_strength * time_factor,
            PriceMove::Down { .. } => -0.1 * signal_strength * time_factor,
            PriceMove::Flat => 0.0,
        };

        // Reduce adjustment if volatility is high
        let vol_discount = (vol - 0.5).max(0.0);
        base_adjustment * (1.0 - vol_discount)
    }
}
```

---

## Component 2: Enhanced Kalman Filter with Temporal Information

### Multi-Source Kalman Filter

```rust
pub struct TemporalKalmanFilter {
    /// Base Kalman filter for price observations
    pub base_filter: KalmanFilter,

    /// Separate filter for temporal chain information
    pub temporal_filter: Option<KalmanFilter>,

    /// Cross-covariance between base and temporal
    pub cross_covariance: f64,

    /// Information quality metric (0-1)
    pub information_quality: f64,
}

impl TemporalKalmanFilter {
    /// Update with both price observation and temporal information
    pub fn update_with_temporal(
        &mut self,
        price_obs: &LogitObservation,
        temporal_obs: Option<&TemporalObservation>,
    ) -> FilteredState {
        // First, update base filter with price
        let base_state = self.base_filter.update(price_obs.logit, price_obs.spread);

        match temporal_obs {
            None => FilteredState {
                logit: base_state.logit,
                prob: base_state.prob,
                vol: self.base_filter.volatility(),
            },
            Some(temp) => {
                // Update temporal filter
                let temporal_state = self.temporal_filter
                    .as_mut()
                    .unwrap()
                    .update(temp.predicted_logit, temp.uncertainty);

                // Fuse the two estimates
                self.fuse_estimates(base_state, temporal_state, temp.quality)
            }
        }
    }

    /// Fuse base and temporal estimates using information quality
    fn fuse_estimates(
        &self,
        base: FilteredState,
        temporal: FilteredState,
        quality: f64,
    ) -> FilteredState {
        // Weight by information quality
        // Base quality from price observation precision
        let base_quality = 1.0 / (1.0 + self.base_filter.uncertainty());

        // Temporal quality from chain position
        let temporal_quality = quality;

        let total_quality = base_quality + temporal_quality;
        let base_weight = base_quality / total_quality;
        let temporal_weight = temporal_quality / total_quality;

        FilteredState {
            logit: base.logit * base_weight + temporal.logit * temporal_weight,
            prob: 0.0,  // Will be computed from logit
            vol: self.base_filter.volatility(),
        }
    }
}
```

---

## Component 3: Transition Probability Estimation via EM

### Extend EM Algorithm for Temporal Transitions

```rust
pub struct TemporalTransitionEM {
    /// Base EM state for jump detection
    pub base_em: EMState,

    /// Transition matrix for temporal moves
    pub transitions: TransitionMatrix,

    /// Historical temporal patterns
    pub history: VecDeque<TemporalPattern>,
}

pub struct TemporalPattern {
    /// First 5min move
    pub first_move: PriceMove,
    /// Second 5min move
    pub second_move: Option<PriceMove>,
    /// Third 5min move (final)
    pub third_move: Option<PriceMove>,
    /// 15min outcome
    pub final_outcome: PriceMove,
    /// Volatility during period
    pub volatility: f64,
    /// Time of day
    pub hour: u8,
}

impl TemporalTransitionEM {
    /// Update transition probabilities using EM
    pub fn update_transitions(&mut self, new_patterns: &[TemporalPattern]) {
        // E-step: Compute posterior probability of each regime
        // M-step: Update transition matrix to maximize likelihood

        // Count transitions
        let mut up_after_up = 0;
        let mut down_after_up = 0;
        let mut up_after_down = 0;
        let mut down_after_down = 0;
        let mut total_up = 0;
        let mut total_down = 0;

        for pattern in self.history.iter().chain(new_patterns.iter()) {
            match (pattern.first_move, pattern.second_move) {
                (PriceMove::Up { .. }, Some(PriceMove::Up { .. })) => {
                    up_after_up += 1;
                    total_up += 1;
                }
                (PriceMove::Up { .. }, Some(PriceMove::Down { .. })) => {
                    down_after_up += 1;
                    total_up += 1;
                }
                (PriceMove::Down { .. }, Some(PriceMove::Up { .. })) => {
                    up_after_down += 1;
                    total_down += 1;
                }
                (PriceMove::Down { .. }, Some(PriceMove::Down { .. })) => {
                    down_after_down += 1;
                    total_down += 1;
                }
                _ => {}
            }
        }

        // Update transition matrix
        self.transitions = TransitionMatrix::from_counts(
            up_after_up, down_after_up,
            up_after_down, down_after_down,
        );
    }

    /// Predict next move probability given history
    pub fn predict_next(&self, previous_moves: &[PriceMove]) -> (f64, f64) {
        if previous_moves.is_empty() {
            // No history: use base rate
            (0.5, 0.5)
        } else {
            let last = previous_moves.last().unwrap();
            match last {
                PriceMove::Up { .. } => (
                    self.transitions.up_given_up,
                    self.transitions.down_given_up,
                ),
                PriceMove::Down { .. } => (
                    self.transitions.up_given_down,
                    self.transitions.down_given_down,
                ),
                PriceMove::Flat => (0.5, 0.5),
            }
        }
    }
}
```

---

## Component 4: Signal Fusion Logic

### Combine Multiple Edge Sources

```rust
pub struct FusedSignal {
    /// Final trading decision
    pub decision: StrategyDecision,

    /// Component breakdown
    pub components: SignalComponents,

    /// Overall confidence
    pub confidence: f64,
}

pub struct SignalComponents {
    /// Fair value model edge
    pub fair_value_edge: f64,

    /// Temporal arbitrage edge
    pub temporal_edge: f64,

    /// Regime adjustment
    pub regime_factor: f64,

    /// Horizon-based edge multiplier
    pub horizon_multiplier: f64,

    /// Final combined edge
    pub combined_edge: f64,
}

pub fn fuse_signals(
    fair_value_signal: &FairValueSignal,
    temporal_signal: Option<&TemporalSignal>,
    regime: &RegimeState,
    horizon: &MarketHorizon,
    config: &FusionConfig,
) -> FusedSignal {
    // 1. Start with fair value edge
    let mut edge = fair_value_signal.edge;

    // 2. Apply regime adjustment
    let regime_factor = match regime.current_regime {
        Regime::Tight => 1.2,      // Better prices, more confident
        Regime::Normal => 1.0,
        Regime::Reversal => 0.5,   // Choppy, reduce confidence
    };
    edge *= regime_factor;

    // 3. Incorporate temporal signal if available
    let temporal_edge = match temporal_signal {
        None => 0.0,
        Some(ts) => {
            // Weight by phase (later phases = more information)
            let phase_weight = match ts.phase {
                MarketPhase::Initial => 0.0,
                MarketPhase::AfterFirst => 0.3,
                MarketPhase::AfterSecond => 0.7,
                MarketPhase::Complete => 0.0,
            };

            // Weight by confidence
            let temporal_contrib = ts.edge * phase_weight * ts.confidence;

            // Blend with fair value edge
            edge = edge * (1.0 - phase_weight) + temporal_contrib;
            temporal_contrib
        }
    };

    // 4. Apply horizon multiplier
    let horizon_mult = horizon.edge_multiplier();
    edge *= horizon_mult;

    // 5. Calculate final confidence
    let confidence = if edge.abs() > config.high_confidence_threshold {
        1.0
    } else if edge.abs() > config.min_edge {
        edge.abs() / config.max_edge
    } else {
        0.0
    };

    // 6. Generate decision
    let decision = if edge > config.min_edge {
        StrategyDecision::Enter { /* ... */ }
    } else if edge < -config.min_edge {
        StrategyDecision::Enter { /* ... */ }
    } else {
        StrategyDecision::Hold
    };

    FusedSignal {
        decision,
        components: SignalComponents {
            fair_value_edge: fair_value_signal.edge,
            temporal_edge,
            regime_factor,
            horizon_multiplier: horizon_mult,
            combined_edge: edge,
        },
        confidence,
    }
}
```

---

## Component 5: Enhanced FairValueEngine

### Extended Engine with Temporal Integration

```rust
pub struct EnhancedFairValueEngine {
    /// Original fair value components
    pub base: FairValueEngine,

    /// Temporal arbitrage engine
    pub temporal: TemporalArbitrageEngine,

    /// Transition probability estimator
    pub transition_em: TemporalTransitionEM,

    /// Signal fusion configuration
    pub fusion_config: FusionConfig,

    /// Active temporal chains being tracked
    pub active_chains: HashMap<String, TemporalChain>,

    /// Historical temporal patterns
    pub temporal_history: VecDeque<TemporalPattern>,
}

impl EnhancedFairValueEngine {
    /// Process new observation with full integration
    pub fn decide_with_integration(
        &mut self,
        obs: &Observation,
    ) -> StrategyDecision {
        // 1. Identify if this market is part of a temporal chain
        let temporal_chain = self.active_chains.get(&obs.condition_id);

        // 2. Get base fair value signal
        let base_signal = self.base.decide(obs);

        // 3. Generate temporal signal if applicable
        let temporal_signal = temporal_chain.and_then(|chain| {
            self.temporal.generate_signal_for_chain(chain, obs)
        });

        // 4. Get current regime state
        let regime = self.detect_regime(obs);

        // 5. Get horizon classification
        let horizon = classify_market(obs.time_remaining_s);

        // 6. Fuse all signals
        let fused = fuse_signals(
            &base_signal,
            temporal_signal.as_ref(),
            &regime,
            &horizon,
            &self.fusion_config,
        );

        // 7. Update temporal tracking
        self.update_temporal_tracking(obs, &fused);

        fused.decision
    }

    /// Update temporal chain tracking
    fn update_temporal_tracking(&mut self, obs: &Observation, signal: &FusedSignal) {
        // Check if this is a 5min market closing
        if self.is_five_min_closing(obs) {
            // Record the result
            let pattern = self.extract_temporal_pattern(obs);

            // Update transition EM
            self.transition_em.update_transitions(&[pattern]);

            // Update related 15min markets
            self.update_related_chains(obs, pattern);
        }

        // Clean up completed chains
        self.cleanup_completed_chains();
    }
}
```

---

## Component 6: Market Discovery for Temporal Chains

### Identify Linked Markets

```rust
pub struct TemporalChainDiscovery {
    /// Market slug patterns
    pub five_min_pattern: Regex,
    pub fifteen_min_pattern: Regex,

    /// Active chains being tracked
    pub active_chains: HashMap<String, TemporalChain>,
}

impl TemporalChainDiscovery {
    /// Scan market list for temporal chain opportunities
    pub fn discover_chains(
        &mut self,
        markets: &[Market],
    ) -> Vec<TemporalChain> {
        let mut chains = Vec::new();

        // Group by reference price and time window
        let mut grouped: HashMap<(f64, i64, i64), Vec<&Market>> = HashMap::new();

        for market in markets {
            if self.is_five_min_market(market) {
                let key = (market.reference_price, market.start_time, market.end_time);
                grouped.entry(key).or_default().push(market);
            }
        }

        // Find 5min groups that form 15min chains
        for (key, group) in grouped {
            if group.len() == 3 {
                // Find matching 15min market
                if let Some(fifteen) = self.find_matching_fifteen_min(&key, markets) {
                    let chain = TemporalChain {
                        target_market: fifteen.clone(),
                        five_min_markets: group.iter().map(|m| (*m).clone()).collect(),
                        reference_price: key.0,
                        chain_type: ChainType::FiveToFifteen,
                    };
                    chains.push(chain);
                }
            }
        }

        chains
    }

    /// Check if market matches 5min pattern
    fn is_five_min_market(&self, market: &Market) -> bool {
        // Pattern: "btc-updown-5m-XXXXXXXX"
        self.five_min_pattern.is_match(&market.question)
    }

    /// Find matching 15min market for a 5min group
    fn find_matching_fifteen_min(
        &self,
        key: &(f64, i64, i64),
        markets: &[Market],
    ) -> Option<&Market> {
        let (ref_price, start, _) = key;

        markets.iter().find(|m| {
            self.is_fifteen_min_market(m)
                && m.reference_price == *ref_price
                && m.start_time == *start
                && m.end_time == *start + 900  // 15 min = 900 sec
        })
    }
}
```

---

## Implementation Roadmap

### Phase 1: Foundation (Current)
- [x] Fair Value Engine with logit jump-diffusion
- [x] Kalman Filter for noise reduction
- [x] EM Algorithm for jump detection
- [x] Horizon-based parameter adaptation
- [x] Temporal Arbitrage probability engine
- [x] Market discovery for temporal chains

### Phase 2: Integration (Next)
- [ ] Extend LogitJumpDiffusion with temporal adjustment
- [ ] Implement TemporalKalmanFilter
- [ ] Add transition probability EM
- [ ] Create signal fusion logic
- [ ] Extend FairValueEngine with temporal integration

### Phase 3: Validation
- [ ] Backtest fused signal vs. individual signals
- [ ] Validate transition probability estimates
- [ ] Test regime detection with temporal patterns
- [ ] Measure improvement in Sharpe ratio

### Phase 4: Production
- [ ] Live market scanner for temporal chains
- [ ] Real-time signal generation
- [ ] Position sizing with fused confidence
- [ ] Risk management for correlated exposures

---

## Expected Performance Improvements

| Metric | Fair Value Only | Temporal Only | Fused |
|--------|----------------|--------------|-------|
| Signal Frequency | Medium | Low (time-limited) | High |
| Signal Quality | Good | Good (when active) | Excellent |
| Win Rate | 55-58% | 52-55% | 60-65% |
| Avg Edge | 5-7% | 3-5% | 8-12% |
| Sharpe Ratio | 1.5-2.0 | 1.0-1.5 | 2.5-3.5 |

---

## Risk Considerations

1. **Model Risk**: Both models could be wrong simultaneously
2. **Correlation Risk**: Temporal chains share same underlying
3. **Execution Risk**: Timing critical for temporal signals
4. **Overfitting Risk**: More parameters to calibrate

### Mitigation Strategies

1. **Conservative fusion weights**: Don't over-weight temporal signals
2. **Diversification**: Trade multiple uncorrelated chains
3. **Position sizing**: Reduce size when models disagree
4. **Stop-losses**: Tight stops on temporal positions
